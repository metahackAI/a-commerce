//! Voice input for the chat composer (docs/api/chat.md §4): the browser records a WAV, this
//! service hands it to Muse Voice Transcribe on the Meta Model API and answers with the text.
//! The text goes back into the composer; the person reads it and sends it as an ordinary turn,
//! so speaking never does anything that typing could not.

use std::sync::Arc;
use std::time::Duration;

use acom_contract::{ErrorCode, ToolError};
use axum::Json;
use axum::extract::{Multipart, Path, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use crate::chat::{App, caller, error, surface_of};

/// Where speech goes: `META_API_BASE_URL`, `META_API_KEY`, `VOICE_MODEL`, and optionally
/// `VOICE_LANGUAGES` (comma-separated language names the recognizer should expect).
pub struct Voice {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub languages: Vec<String>,
}

impl Voice {
    /// Voice input when all three variables are set; otherwise the names that are missing.
    pub fn from_env(env: &dyn Fn(&str) -> Option<String>) -> Result<Self, String> {
        let names = ["META_API_BASE_URL", "META_API_KEY", "VOICE_MODEL"];
        let values: Vec<Option<String>> = names.iter().map(|n| env(n)).collect();
        let missing: Vec<&str> = names
            .iter()
            .zip(&values)
            .filter(|(_, v)| v.is_none())
            .map(|(n, _)| *n)
            .collect();
        if !missing.is_empty() {
            return Err(format!("voice: set {}", missing.join(", ")));
        }
        let [base_url, api_key, model] = [0, 1, 2].map(|i| values[i].clone().unwrap_or_default());
        let languages = env("VOICE_LANGUAGES")
            .map(|v| {
                v.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            api_key,
            model,
            languages,
        })
    }
}

/// Words the recognizer is told to expect: at most this many, each at most this long (it refuses
/// the whole request when one keyword is longer than 30 characters).
const MOST_KEYWORDS: usize = 100;
const LONGEST_KEYWORD: usize = 30;
/// The recognizer's own limit is ten minutes; a composer message is far shorter.
const UPSTREAM_SECONDS: u64 = 30;

fn voice_unavailable(why: &str) -> ToolError {
    ToolError::new(
        ErrorCode::UPSTREAM_UNAVAILABLE,
        format!("Voice input is not available: {why}."),
    )
    .detail(json!({"upstream": "voice", "retryable": false}))
}

/// The words on the person's screen, cleaned: trimmed, no blanks or repeats, bounded.
fn keywords(raw: &[u8]) -> Result<Vec<String>, ToolError> {
    let list: Vec<String> = serde_json::from_slice(raw).map_err(|_| {
        ToolError::invalid_input("`keywords` is a JSON array of words or names.")
            .detail(json!({"field": "keywords"}))
    })?;
    let mut out: Vec<String> = Vec::new();
    for k in list {
        let k = k.split_whitespace().collect::<Vec<_>>().join(" ");
        if k.is_empty() || k.chars().count() > LONGEST_KEYWORD || out.contains(&k) {
            continue;
        }
        out.push(k);
        if out.len() == MOST_KEYWORDS {
            break;
        }
    }
    Ok(out)
}

/// A RIFF/WAVE header; the recognizer takes mono 16-bit PCM WAV only, and the browser makes it.
fn is_wav(audio: &[u8]) -> bool {
    audio.len() > 44 && &audio[0..4] == b"RIFF" && &audio[8..12] == b"WAVE"
}

/// `POST /chat/{surface}/transcriptions`: multipart `audio` (WAV) and optional `keywords`
/// (JSON array). Answers `{"transcript": "...", "audio_ms": 2400}`.
pub async fn transcribe(
    State(app): State<Arc<App>>,
    Path(surface): Path<String>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Response {
    match run(&app, &surface, &headers, multipart).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => error(&e),
    }
}

async fn run(
    app: &App,
    surface: &str,
    headers: &HeaderMap,
    mut multipart: Multipart,
) -> Result<Value, ToolError> {
    let surface = surface_of(surface)?;
    let c = caller(app, headers, surface)?;
    let voice = app.voice.as_ref().map_err(|why| voice_unavailable(why))?;

    let (mut audio, mut words) = (None, Vec::new());
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ToolError::invalid_input(format!("The recording did not arrive whole: {e}")))?
    {
        let name = field.name().unwrap_or_default().to_owned();
        let bytes = field.bytes().await.map_err(|e| {
            ToolError::invalid_input(format!("The recording did not arrive whole: {e}"))
        })?;
        match name.as_str() {
            "audio" => audio = Some(bytes),
            "keywords" => words = keywords(&bytes)?,
            _ => {}
        }
    }
    let audio = audio.filter(|a| is_wav(a)).ok_or_else(|| {
        ToolError::invalid_input("Send the recording as `audio`: a mono 16-bit PCM WAV file.")
            .detail(json!({"field": "audio"}))
    })?;

    let mut settings = json!({
        "model": voice.model,
        "audioEncoding": "WAV",
        "mode": "PUSH_TO_TALK",
        "keywords": words,
    });
    if !voice.languages.is_empty() {
        settings["languageBias"] = json!(voice.languages);
    }
    let part = |p: reqwest::multipart::Part, mime: &str| {
        p.mime_str(mime)
            .map_err(|_| voice_unavailable("the request could not be built"))
    };
    let form = reqwest::multipart::Form::new()
        .part(
            "request",
            part(
                reqwest::multipart::Part::text(settings.to_string()),
                "application/json",
            )?,
        )
        .part(
            "audio",
            part(
                reqwest::multipart::Part::bytes(audio.to_vec()).file_name("speech.wav"),
                "audio/wav",
            )?,
        );
    let started = std::time::Instant::now();
    let res = app
        .http
        .post(format!("{}/asr/transcribe", voice.base_url))
        .bearer_auth(&voice.api_key)
        .timeout(Duration::from_secs(UPSTREAM_SECONDS))
        .multipart(form)
        .send()
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, request_id = %c.request_id, "voice: no answer");
            ToolError::new(
                ErrorCode::UPSTREAM_UNAVAILABLE,
                "Voice input did not answer. Try again, or type the message.",
            )
            .detail(json!({"upstream": "voice", "retryable": true}))
        })?;
    let status = res.status();
    let body = res.bytes().await.unwrap_or_default();
    if !status.is_success() {
        tracing::warn!(
            status = status.as_u16(),
            request_id = %c.request_id,
            body = %String::from_utf8_lossy(&body[..body.len().min(300)]),
            "voice: refused"
        );
        let retryable = status.as_u16() == 429 || status.is_server_error();
        return Err(ToolError::new(
            ErrorCode::UPSTREAM_UNAVAILABLE,
            format!(
                "Voice input could not read the recording ({}). Try again, or type the message.",
                status.as_u16()
            ),
        )
        .detail(json!({"upstream": "voice", "retryable": retryable})));
    }
    let answer: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let transcript = answer["transcript"].as_str().unwrap_or_default().trim();
    let audio_ms = answer["audioDurationMs"].as_u64();
    tracing::info!(
        request_id = %c.request_id,
        audio_ms = ?audio_ms,
        keywords = words.len(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "voice: transcribed"
    );
    Ok(json!({"transcript": transcript, "audio_ms": audio_ms}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |name| {
            pairs
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
        }
    }

    #[test]
    fn voice_needs_its_three_variables_and_says_which_are_missing() {
        let err = Voice::from_env(&env_of(&[("META_API_KEY", "k")]))
            .err()
            .unwrap();
        assert_eq!(err, "voice: set META_API_BASE_URL, VOICE_MODEL");
        let v = Voice::from_env(&env_of(&[
            ("META_API_BASE_URL", "https://api.example.com/v1/"),
            ("META_API_KEY", "k"),
            ("VOICE_MODEL", "voice-model"),
            ("VOICE_LANGUAGES", "English, Spanish,"),
        ]))
        .unwrap();
        assert_eq!(v.base_url, "https://api.example.com/v1");
        assert_eq!(v.languages, ["English", "Spanish"]);
    }

    #[test]
    fn keywords_are_trimmed_unique_and_bounded() {
        let long = "x".repeat(LONGEST_KEYWORD + 1);
        let raw = json!(["  Cotton  utility jacket ", "XL", "", "XL", long]).to_string();
        assert_eq!(
            keywords(raw.as_bytes()).unwrap(),
            ["Cotton utility jacket", "XL"]
        );
        let many: Vec<String> = (0..MOST_KEYWORDS + 5).map(|i| format!("w{i}")).collect();
        let raw = serde_json::to_vec(&many).unwrap();
        assert_eq!(keywords(&raw).unwrap().len(), MOST_KEYWORDS);
        assert!(keywords(b"\"XL\"").is_err());
    }

    #[test]
    fn only_a_riff_wave_file_is_audio() {
        let mut wav = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        wav.resize(64, 0);
        assert!(is_wav(&wav));
        assert!(!is_wav(b"OggS\0\0\0\0\0\0\0\0"));
    }
}
