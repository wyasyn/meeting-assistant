//! Gemini over its REST API (FR-8.1, ADR-020). The only module that knows Gemini's wire format.
//! Audio goes inline as base64, one part per chunk, so times come back per chunk and stay
//! exact; long meetings are sent in batches that fit the inline request limit.

mod schema;

use std::ops::Range;
use std::time::Duration;

use base64::Engine as _;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::Deserialize;
use serde_json::{json, Value};

use super::{
    AnalysisJson, AnalyzeRequest, AudioChunk, Capabilities, Provider, ProviderError, ProviderId,
    TranscribeRequest, TranscribeResult, TranscriptSegment,
};

/// Transcription is most of the tokens, so it uses the cheaper model (ADR-020).
pub const TRANSCRIBE_MODEL: &str = "gemini-3.5-flash-lite";
pub const ANALYZE_MODEL: &str = "gemini-3.8-flash";
const BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

/// 30 minutes of 10 s chunks per request.
const BATCH_CHUNKS: usize = 180;
/// Raw audio per request; base64 grows it by a third, under the 20 MB inline limit.
const BATCH_BYTES: u64 = 14 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const CHECK_TIMEOUT: Duration = Duration::from_secs(20);
const GENERATE_TIMEOUT: Duration = Duration::from_secs(600);
const MAX_OUTPUT_TOKENS: u32 = 65_536;

/// Gemini counts 32 tokens per second of audio.
const AUDIO_TOKENS_PER_S: f64 = 32.0;
/// US dollars per 1M tokens, paid tier list prices on 2026-10-02.
const LITE_IN: f64 = 0.30;
const LITE_OUT: f64 = 2.50;
const FLASH_IN: f64 = 0.75;
const FLASH_OUT: f64 = 3.75;
/// Analysis output plus thinking, a rough allowance.
const ANALYSIS_OUT_TOKENS: f64 = 4_000.0;

const UNREACHABLE: &str = "Could not reach Gemini. Check the internet connection.";
const TIMED_OUT: &str = "Gemini took too long to answer.";
const SERVER_DOWN: &str = "Gemini is not responding right now. Try again later.";
const RATE_LIMITED: &str = "The Gemini rate limit or quota was reached. Try again later.";
const KEY_INVALID: &str = "Google did not accept this API key. Check it in Google AI Studio.";
const KEY_FORBIDDEN: &str = "This API key is not allowed to use the Gemini API.";
const REQUEST_REFUSED: &str = "Gemini refused the request.";
const DECLINED: &str = "Gemini declined to process this meeting.";
const CUT_OFF: &str = "The Gemini answer was cut off.";
const EMPTY: &str = "Gemini returned an empty answer.";
const BAD_TRANSCRIPT: &str = "Gemini returned a transcript in the wrong format.";

pub struct GeminiProvider {
    http: reqwest::Client,
    base: String,
}

impl GeminiProvider {
    pub fn new(key: String) -> Result<Self, ProviderError> {
        Self::with_base(key, BASE_URL, reqwest::Client::builder())
    }

    fn with_base(
        key: String,
        base: &str,
        builder: reqwest::ClientBuilder,
    ) -> Result<Self, ProviderError> {
        // A header, never the URL, so the key cannot end up in a logged URL (rule 8).
        let mut value =
            HeaderValue::from_str(&key).map_err(|_| ProviderError::Rejected(KEY_INVALID.into()))?;
        value.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert("x-goog-api-key", value);
        let http = builder
            .default_headers(headers)
            .connect_timeout(CONNECT_TIMEOUT)
            .https_only(base.starts_with("https://"))
            .build()
            .map_err(|e| {
                tracing::debug!(error = %e, "http client failed to build");
                ProviderError::Unavailable(UNREACHABLE.into())
            })?;
        Ok(Self {
            http,
            base: base.trim_end_matches('/').to_owned(),
        })
    }

    /// Sends a request and returns the JSON body, or the error its status means.
    async fn send(
        &self,
        request: reqwest::RequestBuilder,
        timeout: Duration,
    ) -> Result<Value, ProviderError> {
        let response = request.timeout(timeout).send().await.map_err(send_error)?;
        let status = response.status().as_u16();
        let body = response.text().await.map_err(send_error)?;
        if !(200..300).contains(&status) {
            return Err(status_error(status, &body));
        }
        serde_json::from_str(&body).map_err(|e| {
            tracing::debug!(error = %e, "gemini answer is not json");
            ProviderError::InvalidOutput(EMPTY.into())
        })
    }

    async fn generate(&self, model: &str, body: Value) -> Result<String, ProviderError> {
        let url = format!("{}/models/{model}:generateContent", self.base);
        let answer = self
            .send(self.http.post(url).json(&body), GENERATE_TIMEOUT)
            .await?;
        response_text(&answer)
    }

    async fn transcribe_batch(
        &self,
        chunks: &[AudioChunk],
        req: &TranscribeRequest,
    ) -> Result<Vec<TranscriptSegment>, ProviderError> {
        let mut parts = vec![json!({ "text": transcribe_prompt(req) })];
        for (i, chunk) in chunks.iter().enumerate() {
            parts.push(json!({ "text": format!("Chunk {i}") }));
            parts.push(json!({ "inlineData": {
                "mimeType": "audio/ogg",
                "data": base64::engine::general_purpose::STANDARD.encode(&chunk.ogg),
            }}));
        }
        let body = json!({
            "contents": [{ "role": "user", "parts": parts }],
            "generationConfig": {
                "responseMimeType": "application/json",
                "responseSchema": schema::transcript(req.diarize),
                "maxOutputTokens": MAX_OUTPUT_TOKENS,
            },
        });
        let text = self.generate(TRANSCRIBE_MODEL, body).await?;
        parse_segments(&text, chunks, req.diarize)
    }
}

#[async_trait::async_trait]
impl Provider for GeminiProvider {
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

    /// Looks up both models: free, and proves the key can use them.
    async fn check(&self) -> Result<(), ProviderError> {
        for model in [TRANSCRIBE_MODEL, ANALYZE_MODEL] {
            let url = format!("{}/models/{model}", self.base);
            self.send(self.http.get(url), CHECK_TIMEOUT).await?;
        }
        Ok(())
    }

    async fn transcribe(&self, req: TranscribeRequest) -> Result<TranscribeResult, ProviderError> {
        let sizes: Vec<u64> = req.chunks.iter().map(|c| c.ogg.len() as u64).collect();
        let mut segments = Vec::new();
        let mut speakers = 0;
        for range in batches(&sizes, BATCH_CHUNKS, BATCH_BYTES) {
            let mut batch = self.transcribe_batch(&req.chunks[range], &req).await?;
            speakers = relabel(&mut batch, speakers);
            segments.extend(batch);
        }
        Ok(TranscribeResult { segments })
    }

    async fn analyze(&self, req: AnalyzeRequest) -> Result<AnalysisJson, ProviderError> {
        let body = json!({
            "contents": [{ "role": "user", "parts": [{ "text": analyze_prompt(&req) }] }],
            "generationConfig": {
                "responseMimeType": "application/json",
                "responseSchema": schema::analysis(),
                "maxOutputTokens": MAX_OUTPUT_TOKENS,
            },
        });
        AnalysisJson::parse(&self.generate(ANALYZE_MODEL, body).await?)
    }

    async fn embed(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>, ProviderError> {
        Err(ProviderError::Unsupported(
            "Embeddings are not available with Gemini yet.".into(),
        ))
    }

    fn estimate_cost(&self, audio_seconds: u32, transcript_tokens: u32) -> f64 {
        let audio = f64::from(audio_seconds) * AUDIO_TOKENS_PER_S;
        let transcript = f64::from(transcript_tokens);
        // The transcript comes back wrapped in JSON, roughly half again its size.
        let transcribe = audio * LITE_IN + transcript * 1.5 * LITE_OUT;
        let analyze = transcript * FLASH_IN + ANALYSIS_OUT_TOKENS * FLASH_OUT;
        (transcribe + analyze) / 1_000_000.0
    }
}

/// Renumbers a batch's speakers after the `before` already used, in order of first
/// appearance: Gemini's "Speaker 1" in one batch need not be the same voice in the next
/// (ADR-021). Returns the speakers used so far.
fn relabel(segments: &mut [TranscriptSegment], before: usize) -> usize {
    let mut seen: Vec<String> = Vec::new();
    for seg in segments.iter_mut() {
        if let Some(label) = seg.speaker_label.take() {
            let n = match seen.iter().position(|l| *l == label) {
                Some(i) => i,
                None => {
                    seen.push(label);
                    seen.len() - 1
                }
            };
            seg.speaker_label = Some(format!("Speaker {}", before + n + 1));
        }
    }
    before + seen.len()
}

/// Splits chunks into runs of at most `max_chunks` and `max_bytes` (a lone oversized chunk
/// still gets its own run).
fn batches(sizes: &[u64], max_chunks: usize, max_bytes: u64) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let (mut start, mut bytes) = (0, 0);
    for (i, &size) in sizes.iter().enumerate() {
        if i > start && (i - start == max_chunks || bytes + size > max_bytes) {
            out.push(start..i);
            (start, bytes) = (i, 0);
        }
        bytes += size;
    }
    if start < sizes.len() {
        out.push(start..sizes.len());
    }
    out
}

fn transcribe_prompt(req: &TranscribeRequest) -> String {
    let mut prompt = String::from(
        "Transcribe this meeting recording. The audio is split into numbered chunks of about \
         10 seconds that follow each other without gaps. Return every spoken utterance in \
         order. For each, give the number of the chunk it starts in, its start and end in \
         seconds from the start of that chunk (the end may be past the end of the chunk), \
         and the exact words spoken. Do not summarise, translate or correct the words. Skip \
         silence, music and other sounds.",
    );
    if req.diarize {
        prompt.push_str(
            " Label each voice consistently as Speaker 1, Speaker 2 and so on, in order of \
             first appearance.",
        );
    }
    match &req.language {
        Some(lang) => prompt.push_str(&format!(" The language is {lang}.")),
        None => prompt.push_str(" Write each utterance in the language it was spoken in."),
    }
    if !req.vocabulary.is_empty() {
        prompt.push_str(&format!(
            " Names and terms that may appear: {}.",
            req.vocabulary.join(", ")
        ));
    }
    prompt
}

/// Placeholder wording; the analysis prompt is designed in roadmap task 2.2.
fn analyze_prompt(req: &AnalyzeRequest) -> String {
    let highlights = if req.highlights.is_empty() {
        "none".to_owned()
    } else {
        req.highlights.join("; ")
    };
    format!(
        "You review a recorded meeting for the user, whose lines are marked \"Me\". Using only \
         the transcript, write the summary, key points, decisions, open questions, action \
         items, the four judged scores (0 to 100, each with a short rationale) and 3 to 5 \
         suggestions. Cite segment ids from the transcript where they support a point; never \
         invent ids. The metrics were computed exactly: use them, do not recompute them.\n\n\
         Template: {}\nHighlights: {highlights}\nMetrics: {}\n\nTranscript:\n{}",
        req.template, req.metrics, req.transcript
    )
}

#[derive(Deserialize)]
struct RawTranscript {
    segments: Vec<RawSegment>,
}

#[derive(Deserialize)]
struct RawSegment {
    chunk: usize,
    start_s: f64,
    end_s: f64,
    #[serde(default)]
    speaker: Option<String>,
    text: String,
}

/// Turns per-chunk times into meeting times. Anything malformed rejects the whole answer.
fn parse_segments(
    text: &str,
    chunks: &[AudioChunk],
    diarize: bool,
) -> Result<Vec<TranscriptSegment>, ProviderError> {
    let bad = || ProviderError::InvalidOutput(BAD_TRANSCRIPT.into());
    let raw: RawTranscript = serde_json::from_str(text).map_err(|e| {
        tracing::debug!(error = %e, "transcript did not parse");
        bad()
    })?;
    let mut segments = Vec::with_capacity(raw.segments.len());
    for seg in raw.segments {
        let chunk = chunks.get(seg.chunk).ok_or_else(bad)?;
        if !(seg.start_s.is_finite() && seg.end_s.is_finite() && seg.start_s >= 0.0) {
            return Err(bad());
        }
        let text = seg.text.trim();
        if text.is_empty() {
            continue;
        }
        let ms = |s: f64| chunk.start_ms + (s * 1000.0).round() as i64;
        let start_ms = ms(seg.start_s);
        segments.push(TranscriptSegment {
            start_ms,
            end_ms: ms(seg.end_s).max(start_ms),
            text: text.to_owned(),
            speaker_label: seg
                .speaker
                .map(|s| s.trim().to_owned())
                .filter(|s| diarize && !s.is_empty()),
        });
    }
    segments.sort_by_key(|s| s.start_ms);
    Ok(segments)
}

/// The answer text, minus thought parts, or why there is none.
fn response_text(answer: &Value) -> Result<String, ProviderError> {
    if answer.pointer("/promptFeedback/blockReason").is_some() {
        return Err(ProviderError::Rejected(DECLINED.into()));
    }
    let candidate = answer
        .pointer("/candidates/0")
        .ok_or_else(|| ProviderError::InvalidOutput(EMPTY.into()))?;
    match candidate.get("finishReason").and_then(Value::as_str) {
        None | Some("STOP") => {}
        Some("MAX_TOKENS") => return Err(ProviderError::InvalidOutput(CUT_OFF.into())),
        Some(reason) => {
            tracing::debug!(reason, "gemini stopped early");
            return Err(ProviderError::Rejected(DECLINED.into()));
        }
    }
    let text: String = candidate
        .pointer("/content/parts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|p| p.get("thought").and_then(Value::as_bool) != Some(true))
        .filter_map(|p| p.get("text").and_then(Value::as_str))
        .collect();
    if text.trim().is_empty() {
        return Err(ProviderError::InvalidOutput(EMPTY.into()));
    }
    Ok(text)
}

fn send_error(err: reqwest::Error) -> ProviderError {
    tracing::debug!(error = %err, "gemini request failed");
    ProviderError::Unavailable(
        if err.is_timeout() {
            TIMED_OUT
        } else {
            UNREACHABLE
        }
        .into(),
    )
}

/// Maps an error status to what the user can do about it. Google's own message goes to
/// the debug log only.
fn status_error(status: u16, body: &str) -> ProviderError {
    let detail: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let reason_is = |want: &str| {
        detail
            .pointer("/error/details")
            .and_then(Value::as_array)
            .is_some_and(|d| {
                d.iter()
                    .any(|d| d.get("reason").and_then(Value::as_str) == Some(want))
            })
    };
    let message = detail.pointer("/error/message").and_then(|m| m.as_str());
    tracing::debug!(status, message, "gemini error");
    match status {
        400 if reason_is("API_KEY_INVALID") => ProviderError::Rejected(KEY_INVALID.into()),
        401 => ProviderError::Rejected(KEY_INVALID.into()),
        403 => ProviderError::Rejected(KEY_FORBIDDEN.into()),
        404 => ProviderError::Rejected(format!(
            "Gemini model {TRANSCRIBE_MODEL} or {ANALYZE_MODEL} is not available to this key."
        )),
        408 => ProviderError::Unavailable(TIMED_OUT.into()),
        429 => ProviderError::Unavailable(RATE_LIMITED.into()),
        500..=599 => ProviderError::Unavailable(SERVER_DOWN.into()),
        _ => ProviderError::Rejected(REQUEST_REFUSED.into()),
    }
}

#[cfg(test)]
mod tests;
