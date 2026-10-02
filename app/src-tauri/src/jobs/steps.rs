//! Pipeline steps 1.9 (ADR-021): transcribe each track, then merge (FR-3.1, FR-3.2, FR-2.2).

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use super::StepRunner;
use crate::capture::recorder::{parse_chunk_index, read_chunk, RecordingTarget};
use crate::capture::Track;
use crate::error::AppError;
use crate::providers::{AudioChunk, ProviderId, ProviderService, TranscribeRequest};
use crate::store::meetings::MeetingRepo;
use crate::store::segments::{NewSegment, Segment, SegmentRepo};
use crate::store::Store;

/// Chunk N starts at (N-1) x 10 s (docs/05 "Audio chunks").
const CHUNK_MS: i64 = 10_000;
/// A mic segment this close to system speech may be its echo (docs/04 "merge").
pub const ECHO_WINDOW_MS: i64 = 300;
/// Share of a mic segment's words found in the system speech beside it that makes it echo.
pub const ECHO_SHARE: f64 = 0.6;

const DAMAGED: &str = "A recording file could not be read. It may be damaged.";

/// Transcribes one track. The mic track is the user and is not diarized (rule 4); the
/// system track is. Decrypted audio stays in memory (ADR-021). Replaces the track's
/// segments, so a rerun is safe.
pub struct TranscribeStep {
    pub store: Arc<Store>,
    pub providers: ProviderService,
    /// `<app_data>`; meetings store their audio dir relative to it.
    pub data_dir: PathBuf,
    pub track: Track,
}

impl TranscribeStep {
    /// The track's chunks, decrypted, in time order.
    fn chunks(&self, meeting_id: &str, audio_dir: &str) -> Result<Vec<AudioChunk>, AppError> {
        let target = RecordingTarget {
            meeting_id: meeting_id.to_owned(),
            dir: self.data_dir.join(audio_dir),
            key: self.store.audio_key().clone(),
        };
        let mut indexes: Vec<u32> = match std::fs::read_dir(target.dir.join(self.track.as_str())) {
            Ok(entries) => entries
                .flatten()
                .filter_map(|e| e.file_name().to_str().and_then(parse_chunk_index))
                .collect(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => {
                tracing::debug!(error = %e, "could not list chunks");
                return Err(AppError::Storage(DAMAGED.into()));
            }
        };
        indexes.sort_unstable();
        indexes
            .into_iter()
            .map(|index| {
                let ogg = read_chunk(&target, self.track, index).map_err(|e| {
                    tracing::debug!(error = %e, index, "could not read a chunk");
                    AppError::Storage(DAMAGED.into())
                })?;
                Ok(AudioChunk {
                    ogg,
                    start_ms: i64::from(index - 1) * CHUNK_MS,
                })
            })
            .collect()
    }
}

impl StepRunner for TranscribeStep {
    fn run(&self, meeting_id: &str) -> Result<(), AppError> {
        let (audio_dir, language) = MeetingRepo::new(&*self.store.conn()?)
            .processing_info(meeting_id)?
            .ok_or_else(|| AppError::NotFound("This meeting no longer exists.".into()))?;
        let chunks = self.chunks(meeting_id, &audio_dir)?;
        let segments = if chunks.is_empty() {
            Vec::new()
        } else {
            let provider = self.providers.get(ProviderId::Gemini)?;
            let request = TranscribeRequest {
                chunks,
                language,
                diarize: self.track == Track::Sys,
                vocabulary: Vec::new(),
            };
            // Runs on the jobs thread, outside the async runtime (ADR-019).
            tauri::async_runtime::block_on(provider.transcribe(request))?
                .segments
                .into_iter()
                .map(|s| NewSegment {
                    start_ms: s.start_ms,
                    end_ms: s.end_ms,
                    text: s.text,
                    speaker_label: s.speaker_label,
                })
                .collect()
        };
        SegmentRepo::new(&*self.store.conn()?).replace_track(meeting_id, self.track, &segments)?;
        tracing::info!(
            track = self.track.as_str(),
            segments = segments.len(),
            "track transcribed"
        );
        Ok(())
    }
}

/// Drops mic segments that are the user's microphone picking up the other side (echo).
/// The segments table is already in time order, so that is all merging needs.
pub struct MergeStep {
    pub store: Arc<Store>,
}

impl StepRunner for MergeStep {
    fn run(&self, meeting_id: &str) -> Result<(), AppError> {
        let conn = self.store.conn()?;
        let repo = SegmentRepo::new(&conn);
        let segments = repo.list(meeting_id)?;
        let (mic, sys): (Vec<_>, Vec<_>) = segments.into_iter().partition(|s| s.track == "mic");
        let echo = echo_ids(&mic, &sys);
        repo.delete(&echo)?;
        tracing::info!(echo = echo.len(), "tracks merged");
        Ok(())
    }
}

/// Lowercase words of letters and digits.
fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Mic segments whose words mostly appear in system speech overlapping them, give or take
/// `ECHO_WINDOW_MS`.
pub fn echo_ids(mic: &[Segment], sys: &[Segment]) -> Vec<String> {
    mic.iter()
        .filter(|m| {
            let mic_words = words(&m.text);
            if mic_words.is_empty() {
                return false;
            }
            let near: HashSet<String> = sys
                .iter()
                .filter(|s| {
                    s.start_ms <= m.end_ms + ECHO_WINDOW_MS
                        && s.end_ms >= m.start_ms - ECHO_WINDOW_MS
                })
                .flat_map(|s| words(&s.text))
                .collect();
            let found = mic_words.iter().filter(|w| near.contains(*w)).count();
            found as f64 >= ECHO_SHARE * mic_words.len() as f64
        })
        .map(|m| m.id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;

    use super::*;
    use crate::capture::recorder::chunk_file_name;
    use crate::capture::recorder::tests::TempDir;
    use crate::providers::keys::tests::MemoryApiKeys;
    use crate::providers::keys::ApiKeys;
    use crate::providers::{
        AnalysisJson, AnalyzeRequest, Capabilities, Provider, ProviderError, TranscribeResult,
        TranscriptSegment,
    };
    use crate::store::crypto::{chunk_aad, encrypt_chunk};
    use crate::store::meetings::MeetingStatus;

    type Seen = Arc<Mutex<Vec<TranscribeRequest>>>;

    /// Returns one segment per chunk, labelled when asked to diarize.
    struct FakeProvider(Seen);

    #[async_trait]
    impl Provider for FakeProvider {
        fn id(&self) -> ProviderId {
            ProviderId::Gemini
        }
        fn capabilities(&self) -> Capabilities {
            Capabilities {
                stt: true,
                diarization: true,
                llm: true,
                embeddings: false,
                offline: false,
            }
        }
        async fn check(&self) -> Result<(), ProviderError> {
            Ok(())
        }
        async fn transcribe(
            &self,
            req: TranscribeRequest,
        ) -> Result<TranscribeResult, ProviderError> {
            let segments = req
                .chunks
                .iter()
                .map(|c| TranscriptSegment {
                    start_ms: c.start_ms + 100,
                    end_ms: c.start_ms + 900,
                    text: String::from_utf8(c.ogg.clone()).unwrap(),
                    speaker_label: req.diarize.then(|| "Speaker 1".to_owned()),
                })
                .collect();
            self.0.lock().unwrap().push(req);
            Ok(TranscribeResult { segments })
        }
        async fn analyze(&self, _: AnalyzeRequest) -> Result<AnalysisJson, ProviderError> {
            Err(ProviderError::Unsupported("no".into()))
        }
        async fn embed(&self, _: &[String]) -> Result<Vec<Vec<f32>>, ProviderError> {
            Ok(Vec::new())
        }
        fn estimate_cost(&self, _: u32, _: u32) -> f64 {
            0.0
        }
    }

    struct Harness {
        _dir: TempDir,
        store: Arc<Store>,
        providers: ProviderService,
        keys: Arc<MemoryApiKeys>,
        seen: Seen,
        data_dir: PathBuf,
        meeting_id: String,
    }

    impl Harness {
        fn new(name: &str) -> Self {
            let dir = TempDir::new(&format!("steps-{name}"));
            let store = Arc::new(Store::open_in_memory().unwrap());
            let meeting_id = {
                let conn = store.conn().unwrap();
                let repo = MeetingRepo::new(&conn);
                let m = repo.create_recording("Standup", "zoom", 0).unwrap();
                repo.finish(&m.id, 30_000, 30, MeetingStatus::Processing)
                    .unwrap();
                m.id
            };
            let keys = Arc::new(MemoryApiKeys::default());
            keys.set(ProviderId::Gemini, "key").unwrap();
            let seen = Seen::default();
            let calls = Arc::clone(&seen);
            let providers = ProviderService::new(
                Arc::clone(&keys) as Arc<dyn ApiKeys>,
                Box::new(move |_, _| {
                    Ok(Arc::new(FakeProvider(Arc::clone(&calls))) as Arc<dyn Provider>)
                }),
            );
            Self {
                data_dir: dir.0.clone(),
                _dir: dir,
                store,
                providers,
                keys,
                seen,
                meeting_id,
            }
        }

        /// Writes sealed chunks whose plain content is `text`, as the recorder would.
        fn write_chunks(&self, track: Track, chunks: &[(u32, &str)]) {
            let dir = self
                .data_dir
                .join("meetings")
                .join(&self.meeting_id)
                .join(track.as_str());
            std::fs::create_dir_all(&dir).unwrap();
            for (index, text) in chunks {
                let aad = chunk_aad(&self.meeting_id, track.as_str(), *index);
                let sealed = encrypt_chunk(self.store.audio_key(), &aad, text.as_bytes()).unwrap();
                std::fs::write(dir.join(chunk_file_name(*index)), sealed).unwrap();
            }
            // Leftovers of a crash are not chunks.
            std::fs::write(dir.join("000009.opus.enc.tmp"), b"partial").unwrap();
        }

        fn step(&self, track: Track) -> TranscribeStep {
            TranscribeStep {
                store: Arc::clone(&self.store),
                providers: self.providers.clone(),
                data_dir: self.data_dir.clone(),
                track,
            }
        }

        fn rows(&self) -> Vec<(String, i64, String)> {
            SegmentRepo::new(&self.store.conn().unwrap())
                .list(&self.meeting_id)
                .unwrap()
                .into_iter()
                .map(|s| (s.track, s.start_ms, s.text))
                .collect()
        }
    }

    #[test]
    fn fr_2_2_mic_is_me_and_fr_3_2_system_is_diarized() {
        let h = Harness::new("tracks");
        h.write_chunks(Track::Mic, &[(2, "mic two"), (1, "mic one")]);
        h.write_chunks(Track::Sys, &[(1, "sys one")]);
        h.step(Track::Mic).run(&h.meeting_id).unwrap();
        h.step(Track::Sys).run(&h.meeting_id).unwrap();

        let seen = h.seen.lock().unwrap();
        assert_eq!(
            (seen[0].diarize, seen[0].language.as_deref()),
            (false, Some("en"))
        );
        let starts: Vec<_> = seen[0].chunks.iter().map(|c| c.start_ms).collect();
        assert_eq!(starts, [0, 10_000]);
        assert!(seen[1].diarize);
        drop(seen);

        let row = |track: &str, ms, text: &str| (track.to_owned(), ms, text.to_owned());
        assert_eq!(
            h.rows(),
            [
                row("mic", 100, "mic one"),
                row("sys", 100, "sys one"),
                row("mic", 10_100, "mic two"),
            ]
        );
        let speakers = SegmentRepo::new(&h.store.conn().unwrap())
            .speakers(&h.meeting_id)
            .unwrap();
        let labels: Vec<_> = speakers
            .iter()
            .map(|s| (s.label.as_str(), s.is_me))
            .collect();
        assert_eq!(labels, [("Me", true), ("Speaker 1", false)]);

        // A rerun replaces the track instead of adding to it.
        h.step(Track::Mic).run(&h.meeting_id).unwrap();
        assert_eq!(h.rows().len(), 3);
    }

    #[test]
    fn a_track_without_audio_needs_no_provider() {
        let h = Harness::new("empty");
        h.keys.clear(ProviderId::Gemini).unwrap();
        h.step(Track::Sys).run(&h.meeting_id).unwrap();
        assert!(h.seen.lock().unwrap().is_empty());
        assert!(h.rows().is_empty());
    }

    #[test]
    fn nfr_7_no_key_and_damaged_audio_are_readable_errors() {
        let h = Harness::new("errors");
        h.write_chunks(Track::Mic, &[(1, "hi")]);
        h.keys.clear(ProviderId::Gemini).unwrap();
        let err = h.step(Track::Mic).run(&h.meeting_id).unwrap_err();
        assert_eq!(err.code(), "no_api_key");

        h.keys.set(ProviderId::Gemini, "key").unwrap();
        let chunk = h
            .data_dir
            .join("meetings")
            .join(&h.meeting_id)
            .join("mic")
            .join(chunk_file_name(1));
        std::fs::write(chunk, b"not sealed").unwrap();
        let err = h.step(Track::Mic).run(&h.meeting_id).unwrap_err();
        assert_eq!((err.code(), err.to_string().as_str()), ("storage", DAMAGED));

        let err = h.step(Track::Mic).run("gone").unwrap_err();
        assert_eq!(err.code(), "not_found");
    }

    fn seg(id: &str, track: &str, start_ms: i64, end_ms: i64, text: &str) -> Segment {
        Segment {
            id: id.into(),
            track: track.into(),
            speaker_id: None,
            start_ms,
            end_ms,
            text: text.into(),
        }
    }

    #[test]
    fn fr_2_2_echo_of_the_other_side_is_dropped_and_my_words_kept() {
        let sys = [
            seg("s1", "sys", 1_000, 4_000, "Can everyone see my screen?"),
            seg("s2", "sys", 10_000, 12_000, "Next slide, please."),
        ];
        let cases = [
            // Echo: same words, same time.
            (
                seg("m1", "mic", 1_100, 3_900, "can everyone see my screen"),
                true,
            ),
            // Echo starting just within the window after the sys segment ends.
            (seg("m2", "mic", 4_300, 5_000, "my screen"), true),
            // Just outside the window.
            (seg("m3", "mic", 4_301, 5_000, "my screen"), false),
            // My own answer while they speak.
            (
                seg("m4", "mic", 2_000, 3_000, "Yes, I can see it fine"),
                false,
            ),
            // Same words, but nobody on the other side was speaking then.
            (seg("m5", "mic", 20_000, 21_000, "next slide please"), false),
            // Mostly echo (2 of 3 words).
            (seg("m6", "mic", 10_500, 11_500, "next slide um"), true),
            // Nothing but punctuation.
            (seg("m7", "mic", 1_000, 2_000, "..."), false),
        ];
        for (mic, echo) in cases {
            let got = echo_ids(std::slice::from_ref(&mic), &sys);
            assert_eq!(!got.is_empty(), echo, "{}", mic.id);
        }
    }

    #[test]
    fn merge_deletes_echo_and_is_idempotent() {
        let h = Harness::new("merge");
        {
            let conn = h.store.conn().unwrap();
            let repo = SegmentRepo::new(&conn);
            let new = |start_ms, text: &str| NewSegment {
                start_ms,
                end_ms: start_ms + 2_000,
                text: text.into(),
                speaker_label: Some("Speaker 1".into()),
            };
            repo.replace_track(
                &h.meeting_id,
                Track::Mic,
                &[new(0, "hello there"), new(5_000, "I agree")],
            )
            .unwrap();
            repo.replace_track(&h.meeting_id, Track::Sys, &[new(100, "Hello there!")])
                .unwrap();
        }
        let merge = MergeStep {
            store: Arc::clone(&h.store),
        };
        merge.run(&h.meeting_id).unwrap();
        merge.run(&h.meeting_id).unwrap();
        let texts: Vec<_> = h.rows().into_iter().map(|(_, _, t)| t).collect();
        assert_eq!(texts, ["Hello there!", "I agree"]);
    }
}
