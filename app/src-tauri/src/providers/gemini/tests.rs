use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::thread::JoinHandle;

use super::*;
use crate::providers::analysis::tests::sample;

/// One captured HTTP request.
struct Seen {
    head: String,
    body: Value,
}

/// Serves the given (status, body) answers, one connection each, and returns what it got.
fn serve(answers: Vec<(u16, String)>) -> (String, JoinHandle<Vec<Seen>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/v1beta", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let mut seen = Vec::new();
        for (status, body) in answers {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            let mut head = String::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                head.push_str(&line);
            }
            let len = head
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            let mut raw = vec![0; len];
            reader.read_exact(&mut raw).unwrap();
            let reply = format!(
                "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            reader.get_mut().write_all(reply.as_bytes()).unwrap();
            seen.push(Seen {
                head,
                body: serde_json::from_slice(&raw).unwrap_or(Value::Null),
            });
        }
        seen
    });
    (base, handle)
}

fn provider(base: &str) -> GeminiProvider {
    GeminiProvider::with_base(
        "secret-key".into(),
        base,
        reqwest::Client::builder().no_proxy(),
    )
    .unwrap()
}

fn run<T>(fut: impl std::future::Future<Output = T>) -> T {
    tauri::async_runtime::block_on(fut)
}

/// Gemini's answer carrying `text`, with a thought part that must be ignored.
fn answer(text: &str) -> String {
    json!({ "candidates": [{
        "finishReason": "STOP",
        "content": { "parts": [{ "text": "thinking...", "thought": true }, { "text": text }] },
    }]})
    .to_string()
}

struct Chunks(PathBuf);

impl Chunks {
    fn new(name: &str, count: usize) -> (Self, Vec<AudioChunk>) {
        let dir = std::env::temp_dir().join(format!("ma-gemini-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let chunks = (0..count)
            .map(|i| {
                let path = dir.join(format!("{i}.ogg"));
                std::fs::write(&path, format!("ogg{i}")).unwrap();
                AudioChunk {
                    path,
                    start_ms: i as i64 * 10_000,
                }
            })
            .collect();
        (Self(dir), chunks)
    }
}

impl Drop for Chunks {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn request(chunks: Vec<AudioChunk>, diarize: bool) -> TranscribeRequest {
    TranscribeRequest {
        chunks,
        language: Some("en".into()),
        diarize,
        vocabulary: vec!["Acme".into()],
    }
}

#[test]
fn fr_8_1_check_sends_the_key_in_a_header_and_asks_for_both_models() {
    let (base, server) = serve(vec![(200, "{}".into()), (200, "{}".into())]);
    run(provider(&base).check()).unwrap();
    let seen = server.join().unwrap();
    assert!(seen[0]
        .head
        .starts_with(&format!("GET /v1beta/models/{TRANSCRIBE_MODEL} ")));
    assert!(seen[1]
        .head
        .starts_with(&format!("GET /v1beta/models/{ANALYZE_MODEL} ")));
    assert!(seen[0]
        .head
        .to_ascii_lowercase()
        .contains("x-goog-api-key: secret-key"));
    assert!(!seen[0].head.lines().next().unwrap().contains("secret-key"));
}

#[test]
fn fr_8_1_error_statuses_become_readable_errors() {
    let invalid = json!({ "error": { "code": 400, "message": "API key not valid.",
        "details": [{ "reason": "API_KEY_INVALID" }] } })
    .to_string();
    let cases = [
        (400, invalid, "rejected", KEY_INVALID),
        (403, "{}".into(), "rejected", KEY_FORBIDDEN),
        (429, "{}".into(), "unavailable", RATE_LIMITED),
        (503, "oops".into(), "unavailable", SERVER_DOWN),
        (400, "{}".into(), "rejected", REQUEST_REFUSED),
    ];
    for (status, body, kind, message) in cases {
        let (base, server) = serve(vec![(status, body)]);
        let err = run(provider(&base).check()).unwrap_err();
        server.join().unwrap();
        let got = match &err {
            ProviderError::Rejected(_) => "rejected",
            ProviderError::Unavailable(_) => "unavailable",
            _ => "other",
        };
        assert_eq!((got, err.to_string().as_str()), (kind, message), "{status}");
    }
}

#[test]
fn an_unreachable_server_is_unavailable() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/v1beta", listener.local_addr().unwrap());
    drop(listener);
    let err = run(provider(&base).check()).unwrap_err();
    assert_eq!(err.to_string(), UNREACHABLE);
}

#[test]
fn fr_3_1_transcribe_sends_each_chunk_and_maps_times_to_the_meeting() {
    let (_dir, chunks) = Chunks::new("map", 3);
    let transcript = json!({ "segments": [
        { "chunk": 2, "start_s": 1.5, "end_s": 4.0, "speaker": "Speaker 2", "text": "Bye." },
        { "chunk": 0, "start_s": 0.25, "end_s": 12.0, "speaker": " Speaker 1 ", "text": " Hello all. " },
        { "chunk": 1, "start_s": 3.0, "end_s": 2.0, "speaker": "", "text": "Hm" },
        { "chunk": 1, "start_s": 5.0, "end_s": 6.0, "speaker": "Speaker 1", "text": "  " },
    ]});
    let (base, server) = serve(vec![(200, answer(&transcript.to_string()))]);
    let result = run(provider(&base).transcribe(request(chunks, true))).unwrap();
    let body = &server.join().unwrap()[0].body;

    let parts = body["contents"][0]["parts"].as_array().unwrap();
    assert_eq!(parts.len(), 1 + 3 * 2);
    let prompt = parts[0]["text"].as_str().unwrap();
    assert!(prompt.contains("Speaker 1") && prompt.contains("en") && prompt.contains("Acme"));
    assert_eq!(parts[3]["text"], "Chunk 1");
    assert_eq!(parts[4]["inlineData"]["mimeType"], "audio/ogg");
    assert_eq!(
        parts[4]["inlineData"]["data"],
        base64::engine::general_purpose::STANDARD.encode("ogg1")
    );
    let config = &body["generationConfig"];
    assert_eq!(config["responseMimeType"], "application/json");
    assert!(
        config["responseSchema"]["properties"]["segments"]["items"]["required"]
            .as_array()
            .unwrap()
            .contains(&json!("speaker"))
    );

    let seg = |start_ms, end_ms, text: &str, speaker: Option<&str>| TranscriptSegment {
        start_ms,
        end_ms,
        text: text.into(),
        speaker_label: speaker.map(Into::into),
    };
    assert_eq!(
        result.segments,
        [
            seg(250, 12_000, "Hello all.", Some("Speaker 1")),
            seg(13_000, 13_000, "Hm", None),
            seg(21_500, 24_000, "Bye.", Some("Speaker 2")),
        ]
    );
}

#[test]
fn fr_2_2_without_diarization_no_speaker_is_asked_or_kept() {
    let (_dir, chunks) = Chunks::new("mic", 1);
    let transcript = json!({ "segments": [
        { "chunk": 0, "start_s": 0, "end_s": 1, "speaker": "Speaker 1", "text": "Hi" }
    ]});
    let (base, server) = serve(vec![(200, answer(&transcript.to_string()))]);
    let result = run(provider(&base).transcribe(request(chunks, false))).unwrap();
    let body = &server.join().unwrap()[0].body;
    assert!(!body["contents"][0]["parts"][0]["text"]
        .as_str()
        .unwrap()
        .contains("Speaker 1"));
    assert_eq!(result.segments[0].speaker_label, None);
}

#[test]
fn rule_6_malformed_transcripts_are_rejected_whole() {
    let chunks = vec![AudioChunk {
        path: PathBuf::new(),
        start_ms: 0,
    }];
    let bad = [
        json!({ "segments": [{ "chunk": 1, "start_s": 0, "end_s": 1, "text": "Hi" }] }),
        json!({ "segments": [{ "chunk": 0, "start_s": -1, "end_s": 1, "text": "Hi" }] }),
        json!({ "segments": [{ "chunk": 0, "start_s": 0, "text": "Hi" }] }),
        json!({ "text": "Hi" }),
    ];
    for answer in bad {
        let err = parse_segments(&answer.to_string(), &chunks, false).unwrap_err();
        assert!(matches!(err, ProviderError::InvalidOutput(_)), "{answer}");
    }
}

#[test]
fn long_meetings_are_sent_in_batches() {
    assert_eq!(batches(&[1; 5], 2, 100), [0..2, 2..4, 4..5]);
    assert_eq!(batches(&[60, 60, 30, 10], 10, 100), [0..1, 1..4]);
    assert_eq!(batches(&[500, 1], 10, 100), [0..1, 1..2]);
    assert!(batches(&[], 10, 100).is_empty());

    let (_dir, chunks) = Chunks::new("batch", BATCH_CHUNKS + 1);
    let first = json!({ "segments": [{ "chunk": 0, "start_s": 0, "end_s": 1, "text": "A" }] });
    let second = json!({ "segments": [{ "chunk": 0, "start_s": 0, "end_s": 1, "text": "B" }] });
    let (base, server) = serve(vec![
        (200, answer(&first.to_string())),
        (200, answer(&second.to_string())),
    ]);
    let result = run(provider(&base).transcribe(request(chunks, false))).unwrap();
    let seen = server.join().unwrap();
    assert_eq!(
        seen[1].body["contents"][0]["parts"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    let starts: Vec<_> = result.segments.iter().map(|s| s.start_ms).collect();
    assert_eq!(starts, [0, BATCH_CHUNKS as i64 * 10_000]);
}

#[test]
fn rule_6_analyze_returns_a_checked_analysis() {
    let (base, server) = serve(vec![(200, answer(&sample().to_string()))]);
    let req = AnalyzeRequest {
        transcript: "[s1] [00:01] Me: Ship on Friday".into(),
        metrics: json!({ "talk_ratio": 0.4 }),
        template: "general".into(),
        highlights: Vec::new(),
    };
    let analysis = run(provider(&base).analyze(req)).unwrap();
    let seen = server.join().unwrap();
    assert!(seen[0].head.starts_with(&format!(
        "POST /v1beta/models/{ANALYZE_MODEL}:generateContent "
    )));
    let prompt = seen[0].body["contents"][0]["parts"][0]["text"]
        .as_str()
        .unwrap();
    assert!(prompt.contains("[s1] [00:01] Me: Ship on Friday") && prompt.contains("talk_ratio"));
    assert_eq!(analysis.summary, "Planned the release.");

    let mut off = sample();
    off["scores"]["value"]["score"] = json!(250);
    let (base, server) = serve(vec![(200, answer(&off.to_string()))]);
    let req = AnalyzeRequest {
        transcript: String::new(),
        metrics: Value::Null,
        template: "general".into(),
        highlights: Vec::new(),
    };
    let err = run(provider(&base).analyze(req)).unwrap_err();
    server.join().unwrap();
    assert!(matches!(err, ProviderError::InvalidOutput(_)));
}

#[test]
fn answers_without_usable_text_are_errors() {
    let blocked = json!({ "promptFeedback": { "blockReason": "SAFETY" } });
    let cut = json!({ "candidates": [{ "finishReason": "MAX_TOKENS",
        "content": { "parts": [{ "text": "{" }] } }] });
    let safety = json!({ "candidates": [{ "finishReason": "SAFETY" }] });
    let only_thought = json!({ "candidates": [{ "finishReason": "STOP",
        "content": { "parts": [{ "text": "hmm", "thought": true }] } }] });
    assert!(matches!(
        response_text(&blocked),
        Err(ProviderError::Rejected(_))
    ));
    assert!(matches!(
        response_text(&cut),
        Err(ProviderError::InvalidOutput(_))
    ));
    assert!(matches!(
        response_text(&safety),
        Err(ProviderError::Rejected(_))
    ));
    assert!(matches!(
        response_text(&only_thought),
        Err(ProviderError::InvalidOutput(_))
    ));
    assert!(matches!(
        response_text(&json!({})),
        Err(ProviderError::InvalidOutput(_))
    ));
}

#[test]
fn fr_8_4_cost_grows_with_audio_and_transcript() {
    let p = GeminiProvider::new("k".into()).unwrap();
    let hour = p.estimate_cost(3_600, 15_000);
    assert!((0.05..0.2).contains(&hour), "{hour}");
    assert!(p.estimate_cost(7_200, 30_000) > hour);
}

#[test]
fn a_key_that_cannot_be_a_header_is_rejected() {
    assert!(matches!(
        GeminiProvider::new("bad\nkey".into()),
        Err(ProviderError::Rejected(_))
    ));
}
