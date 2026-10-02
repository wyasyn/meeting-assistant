//! PipeWire backend (Fedora/Wayland first). See ADR-013 for the spike findings.
//!
//! PipeWire objects are not `Send`, so each capture owns one thread with its own main loop,
//! context, core and two streams. `stop` reaches that loop through a `pipewire::channel`.
//! Callbacks run on that thread (no `RT_PROCESS`), so they may allocate and send frames.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::time::{Duration, Instant};

use pipewire as pw;
use pw::properties::properties;
use pw::spa;
use pw::spa::param::audio::{AudioFormat, AudioInfoRaw};
use pw::spa::pod::Pod;
use pw::types::ObjectType;

use super::{
    AudioBackend, AudioDevice, AudioFrame, CaptureConfig, CaptureError, CaptureSession, DeviceKind,
    Track, TrackClock, FRAME_QUEUE, SAMPLE_RATE,
};

const CLASS_SOURCE: &str = "Audio/Source";
const CLASS_SINK: &str = "Audio/Sink";
const DEFAULT_SOURCE_KEY: &str = "default.audio.source";
const DEFAULT_SINK_KEY: &str = "default.audio.sink";

pub struct PipeWireBackend;

impl PipeWireBackend {
    pub fn new() -> Self {
        pw::init();
        Self
    }
}

impl AudioBackend for PipeWireBackend {
    fn list_devices(&self) -> Result<Vec<AudioDevice>, CaptureError> {
        list_devices()
    }

    fn start(&self, config: CaptureConfig) -> Result<CaptureSession, CaptureError> {
        if config.mic.is_some() || config.output.is_some() {
            let devices = list_devices()?;
            for wanted in [&config.mic, &config.output].into_iter().flatten() {
                if !devices.iter().any(|d| &d.id == wanted) {
                    return Err(CaptureError::DeviceNotFound(wanted.clone()));
                }
            }
        }

        let (frame_tx, frame_rx) = mpsc::sync_channel(FRAME_QUEUE);
        let (ready_tx, ready_rx) = mpsc::channel();
        let (stop_tx, stop_rx) = pw::channel::channel::<()>();
        let thread = std::thread::Builder::new()
            .name("pipewire-capture".into())
            .spawn(move || {
                if let Err(err) = run_capture(&config, frame_tx, stop_rx, &ready_tx) {
                    // Ignored when start() already returned; the session then sees the queue close.
                    let _ = ready_tx.send(Err(err));
                }
            })
            .map_err(|e| CaptureError::Stream(e.to_string()))?;

        match ready_rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                let _ = thread.join();
                return Err(err);
            }
            Err(_) => return Err(CaptureError::Stream("capture thread exited".into())),
        }
        let stop = Box::new(move || {
            // Fails only when the loop has already ended.
            let _ = stop_tx.send(());
        });
        Ok(CaptureSession::new(frame_rx, stop, Some(thread)))
    }
}

fn connect_error(err: pw::Error) -> CaptureError {
    CaptureError::Connect(err.to_string())
}

fn stream_error(err: pw::Error) -> CaptureError {
    CaptureError::Stream(err.to_string())
}

/// A stream that delivers nothing for this long is reconnected (ADR-013).
const STALL_TIMEOUT: Duration = Duration::from_secs(2);
const WATCHDOG_INTERVAL: Duration = Duration::from_millis(500);

/// Per-stream state owned by the stream listener.
struct StreamState {
    track: Track,
    channels: u32,
    clock: TrackClock,
    started: Instant,
    last_data: Rc<Cell<Instant>>,
    frames: SyncSender<AudioFrame>,
    dropped: u64,
}

/// A connected stream plus what the watchdog needs to see and revive it.
struct CaptureStream {
    track: Track,
    stream: pw::stream::StreamRc,
    last_data: Rc<Cell<Instant>>,
    _listener: pw::stream::StreamListener<StreamState>,
}

impl CaptureStream {
    fn connect(&self) -> Result<(), CaptureError> {
        let format = format_param()?;
        let mut params = [Pod::from_bytes(&format)
            .ok_or_else(|| CaptureError::Stream("invalid format pod".into()))?];
        self.stream
            .connect(
                spa::utils::Direction::Input,
                None,
                pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
                &mut params,
            )
            .map_err(stream_error)
    }

    /// Bluetooth profile switches can leave a stream linked but silent; reconnecting relinks it.
    fn revive_if_stalled(&self) {
        if self.last_data.get().elapsed() < STALL_TIMEOUT {
            return;
        }
        tracing::warn!(track = ?self.track, "capture stream stalled, reconnecting");
        self.last_data.set(Instant::now());
        if let Err(err) = self.stream.disconnect() {
            tracing::warn!(%err, "could not disconnect stalled stream");
        }
        if let Err(err) = self.connect() {
            tracing::warn!(%err, "could not reconnect stalled stream");
        }
    }
}

fn run_capture(
    config: &CaptureConfig,
    frames: SyncSender<AudioFrame>,
    stop: pw::channel::Receiver<()>,
    ready: &mpsc::Sender<Result<(), CaptureError>>,
) -> Result<(), CaptureError> {
    let mainloop = pw::main_loop::MainLoopRc::new(None).map_err(connect_error)?;
    let context = pw::context::ContextRc::new(&mainloop, None).map_err(connect_error)?;
    let core = context.connect_rc(None).map_err(connect_error)?;

    let _stop = stop.attach(mainloop.loop_(), {
        let mainloop = mainloop.clone();
        move |()| mainloop.quit()
    });

    let started = Instant::now();
    let streams = Rc::new([
        open_stream(
            &core,
            Track::Mic,
            config.mic.as_deref(),
            frames.clone(),
            started,
        )?,
        open_stream(&core, Track::Sys, config.output.as_deref(), frames, started)?,
    ]);

    let watchdog = mainloop.loop_().add_timer({
        let streams = streams.clone();
        move |_| streams.iter().for_each(CaptureStream::revive_if_stalled)
    });
    watchdog
        .update_timer(Some(WATCHDOG_INTERVAL), Some(WATCHDOG_INTERVAL))
        .into_sync_result()
        .map_err(|e| CaptureError::Stream(e.to_string()))?;

    tracing::info!("capture started");
    let _ = ready.send(Ok(()));
    mainloop.run();

    // Disconnect before the listeners drop so no callback runs half torn down.
    for capture in streams.iter() {
        if let Err(err) = capture.stream.disconnect() {
            tracing::warn!(%err, "could not disconnect capture stream");
        }
    }
    tracing::info!("capture stopped");
    Ok(())
}

fn open_stream(
    core: &pw::core::CoreRc,
    track: Track,
    target: Option<&str>,
    frames: SyncSender<AudioFrame>,
    started: Instant,
) -> Result<CaptureStream, CaptureError> {
    let name = match track {
        Track::Mic => "meeting-assistant-mic",
        Track::Sys => "meeting-assistant-sys",
    };
    let mut props = properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Capture",
        *pw::keys::NODE_NAME => name,
    };
    match track {
        Track::Mic => props.insert(*pw::keys::MEDIA_ROLE, "Communication"),
        // Record what the output device plays (its monitor), not a microphone.
        Track::Sys => props.insert(*pw::keys::STREAM_CAPTURE_SINK, "true"),
    }
    if let Some(target) = target {
        // `pw::keys::TARGET_OBJECT` needs a crate feature; the key is stable since 0.3.44.
        props.insert("target.object", target);
    }
    let stream = pw::stream::StreamRc::new(core.clone(), name, props).map_err(stream_error)?;

    let last_data = Rc::new(Cell::new(Instant::now()));
    let state = StreamState {
        track,
        channels: 1,
        clock: TrackClock::default(),
        started,
        last_data: last_data.clone(),
        frames,
        dropped: 0,
    };
    let listener = stream
        .add_local_listener_with_user_data(state)
        .state_changed(|_, state, old, new| {
            tracing::debug!(track = ?state.track, ?old, ?new, "capture stream state");
        })
        .param_changed(|_, state, id, param| {
            let Some(param) = param else { return };
            if id != spa::param::ParamType::Format.as_raw() {
                return;
            }
            let mut info = AudioInfoRaw::new();
            if info.parse(param).is_ok() {
                state.channels = info.channels().max(1);
                tracing::debug!(
                    track = ?state.track,
                    rate = info.rate(),
                    channels = info.channels(),
                    "capture format"
                );
            }
        })
        .process(|stream, state| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let Some(data) = buffer.datas_mut().first_mut() else {
                return;
            };
            let chunk = data.chunk();
            let (offset, size) = (chunk.offset() as usize, chunk.size() as usize);
            let Some(bytes) = data.data() else { return };
            let Some(bytes) = bytes.get(offset..offset + size) else {
                return;
            };
            let samples = downmix(bytes, state.channels);
            if samples.is_empty() {
                return;
            }
            state.last_data.set(Instant::now());
            let elapsed = state.started.elapsed().as_micros() * u128::from(SAMPLE_RATE) / 1_000_000;
            let elapsed = u64::try_from(elapsed).unwrap_or(u64::MAX);
            let frame = state.clock.frame_at(state.track, samples, elapsed);
            match state.frames.try_send(frame) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {
                    state.dropped += 1;
                    if state.dropped.is_power_of_two() {
                        tracing::warn!(track = ?state.track, dropped = state.dropped, "capture queue full, frames dropped");
                    }
                }
                Err(TrySendError::Disconnected(_)) => {}
            }
        })
        .register()
        .map_err(stream_error)?;

    let capture = CaptureStream {
        track,
        stream,
        last_data,
        _listener: listener,
    };
    capture.connect()?;
    Ok(capture)
}

/// F32LE, 48 kHz, mono. PipeWire's adapter converts from whatever the device runs at.
fn format_param() -> Result<Vec<u8>, CaptureError> {
    let mut info = AudioInfoRaw::new();
    info.set_format(AudioFormat::F32LE);
    info.set_rate(SAMPLE_RATE);
    info.set_channels(1);
    let object = spa::pod::Object {
        type_: spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
        id: spa::param::ParamType::EnumFormat.as_raw(),
        properties: info.into(),
    };
    let (cursor, _) = spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(object),
    )
    .map_err(|e| CaptureError::Stream(format!("{e:?}")))?;
    Ok(cursor.into_inner())
}

/// Interleaved little-endian f32 to mono by averaging channels.
fn downmix(bytes: &[u8], channels: u32) -> Vec<f32> {
    let channels = channels.max(1) as usize;
    let (words, _) = bytes.as_chunks::<4>();
    let samples: Vec<f32> = words.iter().map(|b| f32::from_le_bytes(*b)).collect();
    if channels == 1 {
        return samples;
    }
    samples
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// A registry node as far as device listing cares.
struct NodeInfo {
    class: String,
    name: String,
    description: Option<String>,
}

fn list_devices() -> Result<Vec<AudioDevice>, CaptureError> {
    let mainloop = pw::main_loop::MainLoopRc::new(None).map_err(connect_error)?;
    let context = pw::context::ContextRc::new(&mainloop, None).map_err(connect_error)?;
    let core = context.connect_rc(None).map_err(connect_error)?;
    let registry = core.get_registry_rc().map_err(connect_error)?;

    let nodes: Rc<RefCell<Vec<NodeInfo>>> = Rc::default();
    let defaults: Rc<RefCell<Defaults>> = Rc::default();
    let metadata: Rc<RefCell<Vec<(pw::metadata::Metadata, pw::metadata::MetadataListener)>>> =
        Rc::default();

    let _registry_listener = registry
        .add_listener_local()
        .global({
            let nodes = nodes.clone();
            let defaults = defaults.clone();
            let metadata = metadata.clone();
            let registry = registry.downgrade();
            move |global| {
                let Some(props) = global.props else { return };
                match global.type_ {
                    ObjectType::Node => {
                        if let (Some(class), Some(name)) =
                            (props.get("media.class"), props.get("node.name"))
                        {
                            nodes.borrow_mut().push(NodeInfo {
                                class: class.to_owned(),
                                name: name.to_owned(),
                                description: props.get("node.description").map(str::to_owned),
                            });
                        }
                    }
                    ObjectType::Metadata if props.get("metadata.name") == Some("default") => {
                        let Some(registry) = registry.upgrade() else {
                            return;
                        };
                        let Ok(meta) = registry.bind::<pw::metadata::Metadata, _>(global) else {
                            return;
                        };
                        let listener = meta
                            .add_listener_local()
                            .property({
                                let defaults = defaults.clone();
                                move |_subject, key, _type, value| {
                                    defaults.borrow_mut().update(key, value);
                                    0
                                }
                            })
                            .register();
                        metadata.borrow_mut().push((meta, listener));
                    }
                    _ => {}
                }
            }
        })
        .register();

    // Two roundtrips: the first lists globals, the second delivers metadata properties.
    let pending = Rc::new(Cell::new(core.sync(0).map_err(connect_error)?));
    let rounds = Rc::new(Cell::new(0u8));
    let sync_error: Rc<RefCell<Option<pw::Error>>> = Rc::default();
    let _core_listener = core
        .add_listener_local()
        .done({
            let core = core.clone();
            let mainloop = mainloop.clone();
            let pending = pending.clone();
            let sync_error = sync_error.clone();
            move |id, seq| {
                if id != pw::core::PW_ID_CORE || seq != pending.get() {
                    return;
                }
                rounds.set(rounds.get() + 1);
                if rounds.get() >= 2 {
                    mainloop.quit();
                    return;
                }
                match core.sync(0) {
                    Ok(next) => pending.set(next),
                    Err(err) => {
                        *sync_error.borrow_mut() = Some(err);
                        mainloop.quit();
                    }
                }
            }
        })
        .error({
            let mainloop = mainloop.clone();
            move |_id, _seq, _res, message| {
                tracing::debug!(message, "pipewire core error while listing devices");
                mainloop.quit();
            }
        })
        .register();
    mainloop.run();

    if let Some(err) = sync_error.borrow_mut().take() {
        return Err(connect_error(err));
    }
    let defaults = defaults.borrow();
    let devices = nodes
        .borrow()
        .iter()
        .filter_map(|node| to_device(node, &defaults))
        .collect();
    Ok(devices)
}

/// Default device names from the `default` metadata object.
#[derive(Debug, Default, PartialEq, Eq)]
struct Defaults {
    source: Option<String>,
    sink: Option<String>,
}

impl Defaults {
    fn update(&mut self, key: Option<&str>, value: Option<&str>) {
        let name = value.and_then(parse_default_name);
        match key {
            Some(DEFAULT_SOURCE_KEY) => self.source = name,
            Some(DEFAULT_SINK_KEY) => self.sink = name,
            _ => {}
        }
    }
}

/// Values look like `{"name":"alsa_output.pci-0000_00_1f.3.analog-stereo"}`.
fn parse_default_name(value: &str) -> Option<String> {
    let json: serde_json::Value = serde_json::from_str(value).ok()?;
    json.get("name")?.as_str().map(str::to_owned)
}

fn to_device(node: &NodeInfo, defaults: &Defaults) -> Option<AudioDevice> {
    let (kind, default) = match node.class.as_str() {
        CLASS_SOURCE => (DeviceKind::Input, &defaults.source),
        CLASS_SINK => (DeviceKind::Output, &defaults.sink),
        _ => return None,
    };
    Some(AudioDevice {
        id: node.name.clone(),
        name: node
            .description
            .clone()
            .unwrap_or_else(|| node.name.clone()),
        kind,
        is_default: default.as_deref() == Some(node.name.as_str()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(class: &str, name: &str, description: Option<&str>) -> NodeInfo {
        NodeInfo {
            class: class.into(),
            name: name.into(),
            description: description.map(Into::into),
        }
    }

    #[test]
    fn fr_2_3_maps_sources_and_sinks_including_bluetooth() {
        let defaults = Defaults {
            source: Some("bluez_input.94:A4".into()),
            sink: Some("alsa_output.analog".into()),
        };
        let bt = to_device(
            &node(CLASS_SOURCE, "bluez_input.94:A4", Some("Headset mic")),
            &defaults,
        )
        .unwrap();
        assert_eq!(bt.kind, DeviceKind::Input);
        assert_eq!(bt.name, "Headset mic");
        assert!(bt.is_default);

        let speakers = to_device(&node(CLASS_SINK, "alsa_output.analog", None), &defaults).unwrap();
        assert_eq!(speakers.kind, DeviceKind::Output);
        assert_eq!(speakers.name, "alsa_output.analog");
        assert!(speakers.is_default);
    }

    #[test]
    fn skips_nodes_that_are_not_plain_devices() {
        let defaults = Defaults::default();
        for class in ["Stream/Input/Audio", "Audio/Source/Virtual", "Video/Source"] {
            assert!(
                to_device(&node(class, "x", None), &defaults).is_none(),
                "{class}"
            );
        }
    }

    #[test]
    fn reads_default_names_from_metadata() {
        let mut defaults = Defaults::default();
        defaults.update(
            Some(DEFAULT_SINK_KEY),
            Some(r#"{"name":"alsa_output.analog"}"#),
        );
        defaults.update(Some(DEFAULT_SOURCE_KEY), Some("not json"));
        defaults.update(Some("default.video.source"), Some(r#"{"name":"cam"}"#));
        assert_eq!(
            defaults,
            Defaults {
                source: None,
                sink: Some("alsa_output.analog".into()),
            }
        );
    }

    #[test]
    fn downmixes_interleaved_stereo_to_mono() {
        let bytes: Vec<u8> = [0.5f32, 0.1, -0.2, 0.2]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let mono = downmix(&bytes, 2);
        assert_eq!(mono.len(), 2);
        assert!((mono[0] - 0.3).abs() < 1e-6);
        assert!(mono[1].abs() < 1e-6);
        assert_eq!(downmix(&bytes, 1).len(), 4);
    }

    // Needs a running PipeWire session: cargo test -- --ignored pipewire_lists
    #[test]
    #[ignore]
    fn pipewire_lists_devices_on_this_machine() {
        let devices = PipeWireBackend::new().list_devices().unwrap();
        assert!(devices.iter().any(|d| d.kind == DeviceKind::Output));
    }
}
