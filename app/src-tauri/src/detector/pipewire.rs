//! Mic streams on Linux from the PipeWire registry (FR-1.1, FR-1.2, ADR-016).
//!
//! Registry globals do not carry the owning process, so each capture stream node is bound and
//! its info read once: `application.process.binary` and `application.process.id` are set for
//! native and PulseAudio clients alike. Same thread model as `capture/pipewire.rs`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc::{self, Sender};

use pipewire as pw;
use pw::types::ObjectType;

use super::{DetectorError, MicStreamSource, StreamEvent, StreamInfo, StreamWatch};

/// Apps capturing audio; `Stream/Input/Audio/Internal` and others are not mic users.
const CLASS_CAPTURE_STREAM: &str = "Stream/Input/Audio";

pub struct PipeWireStreams;

impl PipeWireStreams {
    pub fn new() -> Self {
        pw::init();
        Self
    }
}

impl MicStreamSource for PipeWireStreams {
    fn watch(&self, events: Sender<StreamEvent>) -> Result<StreamWatch, DetectorError> {
        let (ready_tx, ready_rx) = mpsc::channel();
        let (stop_tx, stop_rx) = pw::channel::channel::<()>();
        let thread = std::thread::Builder::new()
            .name("pipewire-detector".into())
            .spawn(move || {
                if let Err(err) = run_watch(events, stop_rx, &ready_tx) {
                    let _ = ready_tx.send(Err(err));
                }
            })
            .map_err(|e| DetectorError::Watch(e.to_string()))?;
        match ready_rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                let _ = thread.join();
                return Err(err);
            }
            Err(_) => return Err(DetectorError::Watch("watch thread exited".into())),
        }
        let stop = Box::new(move || {
            // Fails only when the loop has already ended.
            let _ = stop_tx.send(());
        });
        Ok(StreamWatch::new(stop, Some(thread)))
    }
}

fn watch_error(err: pw::Error) -> DetectorError {
    DetectorError::Watch(err.to_string())
}

/// A bound capture stream; dropping it releases the proxy and its listener.
struct Bound {
    _node: pw::node::Node,
    _listener: pw::node::NodeListener,
}

fn run_watch(
    events: Sender<StreamEvent>,
    stop: pw::channel::Receiver<()>,
    ready: &mpsc::Sender<Result<(), DetectorError>>,
) -> Result<(), DetectorError> {
    let mainloop = pw::main_loop::MainLoopRc::new(None).map_err(watch_error)?;
    let context = pw::context::ContextRc::new(&mainloop, None).map_err(watch_error)?;
    let core = context.connect_rc(None).map_err(watch_error)?;
    let registry = core.get_registry_rc().map_err(watch_error)?;

    let _stop = stop.attach(mainloop.loop_(), {
        let mainloop = mainloop.clone();
        move |()| mainloop.quit()
    });

    let bound: Rc<RefCell<HashMap<u32, Bound>>> = Rc::default();
    let _registry_listener = registry
        .add_listener_local()
        .global({
            let bound = bound.clone();
            let events = events.clone();
            let registry = registry.downgrade();
            move |global| {
                if global.type_ != ObjectType::Node
                    || global.props.and_then(|p| p.get("media.class")) != Some(CLASS_CAPTURE_STREAM)
                {
                    return;
                }
                let Some(registry) = registry.upgrade() else {
                    return;
                };
                let node = match registry.bind::<pw::node::Node, _>(global) {
                    Ok(node) => node,
                    Err(err) => {
                        tracing::debug!(%err, "could not bind a capture stream");
                        return;
                    }
                };
                let id = global.id;
                let sent = Rc::new(RefCell::new(false));
                let listener = node
                    .add_listener_local()
                    .info({
                        let events = events.clone();
                        move |info| {
                            // Info repeats on every state change; the owner is reported once.
                            if sent.replace(true) {
                                return;
                            }
                            let info = stream_info(|key| info.props().and_then(|p| p.get(key)));
                            tracing::debug!(
                                id,
                                binary = info.binary.as_deref().unwrap_or("?"),
                                "capture stream opened"
                            );
                            let _ = events.send(StreamEvent::Opened { id, info });
                        }
                    })
                    .register();
                bound.borrow_mut().insert(
                    id,
                    Bound {
                        _node: node,
                        _listener: listener,
                    },
                );
            }
        })
        .global_remove({
            let bound = bound.clone();
            let events = events.clone();
            move |id| {
                if bound.borrow_mut().remove(&id).is_some() {
                    tracing::debug!(id, "capture stream closed");
                    let _ = events.send(StreamEvent::Closed { id });
                }
            }
        })
        .register();

    // Without PipeWire the watch ends and the detector retries later.
    let _core_listener = core
        .add_listener_local()
        .error({
            let mainloop = mainloop.clone();
            move |id, _seq, _res, message| {
                if id == pw::core::PW_ID_CORE {
                    tracing::debug!(message, "pipewire core error in the stream watch");
                    mainloop.quit();
                }
            }
        })
        .register();

    let _ = ready.send(Ok(()));
    mainloop.run();
    // Drop proxies before the core goes away.
    bound.borrow_mut().clear();
    Ok(())
}

/// Reads a stream's owner from its properties.
fn stream_info<'a>(get: impl Fn(&str) -> Option<&'a str>) -> StreamInfo {
    StreamInfo {
        pid: get("application.process.id").and_then(|pid| pid.trim().parse().ok()),
        binary: get("application.process.binary")
            .map(str::trim)
            .filter(|b| !b.is_empty())
            .map(str::to_owned),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(pairs: &[(&'static str, &'static str)]) -> StreamInfo {
        stream_info(|key| pairs.iter().find(|(k, _)| *k == key).map(|(_, v)| *v))
    }

    #[test]
    fn fr_1_1_reads_owner_from_stream_props() {
        assert_eq!(
            info(&[
                ("application.process.id", "4242"),
                ("application.process.binary", "firefox"),
                ("application.name", "Firefox"),
            ]),
            StreamInfo {
                pid: Some(4242),
                binary: Some("firefox".into())
            }
        );
    }

    #[test]
    fn missing_or_bad_props_give_none() {
        assert_eq!(
            info(&[
                ("application.process.id", "not a pid"),
                ("application.process.binary", " "),
            ]),
            StreamInfo {
                pid: None,
                binary: None
            }
        );
        assert_eq!(
            info(&[]),
            StreamInfo {
                pid: None,
                binary: None
            }
        );
    }
}
