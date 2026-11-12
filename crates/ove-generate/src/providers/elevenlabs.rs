//! ElevenLabs (`xi-api-key` header): text to speech `POST /v1/text-to-speech/<voice>`, sound effects
//! `POST /v1/sound-generation`, music `POST /v1/music`, voices `GET /v2/voices`. All return MP3 bytes.

use serde::Serialize;
use serde_json::{Value, json};

use super::{GenError, Output, Result, Run, agent, base, download_to, http_error};
use crate::catalog::{Kind, Model};
use crate::job::Request;

const FORMAT: &str = "mp3_44100_128";
/// Largest audio accepted (10 minutes of 128 kbit/s MP3 is about 10 MB).
const MAX_AUDIO: u64 = 512 << 20;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Voice {
    pub id: String,
    pub name: String,
    pub category: Option<String>,
    pub labels: Value,
    pub preview_url: Option<String>,
}

/// Voices in the account (premade, cloned, …), optionally filtered by `search`.
pub fn voices(key: &str, search: Option<&str>) -> Result<Vec<Voice>> {
    ove_guard::register_secret(key);
    let root = base("OVE_ELEVENLABS_URL", "https://api.elevenlabs.io");
    let mut req = agent().get(&format!("{root}/v2/voices")).set("xi-api-key", key).query("page_size", "100");
    if let Some(s) = search.filter(|s| !s.is_empty()) {
        req = req.query("search", s);
    }
    let v: Value = req
        .call()
        .map_err(|e| http_error("ElevenLabs", e))?
        .into_json()
        .map_err(|e| GenError::new("provider.failed", e.to_string()))?;
    Ok(v.get("voices")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| {
                    Some(Voice {
                        id: x.get("voice_id")?.as_str()?.to_string(),
                        name: x.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
                        category: x.get("category").and_then(Value::as_str).map(str::to_string),
                        labels: x.get("labels").cloned().unwrap_or(Value::Null),
                        preview_url: x.get("preview_url").and_then(Value::as_str).map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default())
}

fn looks_like_id(s: &str) -> bool {
    s.len() >= 18 && s.chars().all(|c| c.is_ascii_alphanumeric())
}

/// A voice id from an id or a name (default: the first premade voice).
pub fn resolve_voice(key: &str, q: Option<&str>) -> Result<(String, String)> {
    if let Some(q) = q.filter(|q| looks_like_id(q)) {
        return Ok((q.to_string(), q.to_string()));
    }
    let list = voices(key, q)?;
    let pick = match q {
        Some(q) => list
            .iter()
            .find(|v| v.name.eq_ignore_ascii_case(q))
            .or_else(|| list.iter().find(|v| v.name.to_lowercase().starts_with(&q.to_lowercase()))),
        None => list.iter().find(|v| v.category.as_deref() == Some("premade")).or(list.first()),
    };
    pick.map(|v| (v.id.clone(), v.name.clone())).ok_or_else(|| {
        GenError::new("voice.notFound", format!("no voice {} in the ElevenLabs account", q.unwrap_or("at all")))
    })
}

pub fn run(model: &Model, req: &Request, r: &mut Run) -> Result<Output> {
    let root = base("OVE_ELEVENLABS_URL", "https://api.elevenlabs.io");
    (r.progress)(Some(0.1), None);
    let (url, body, details) = match model.kind {
        Kind::Voice => {
            let (id, name) = resolve_voice(r.key, req.voice.as_deref())?;
            let mut settings = json!({"stability": req.stability.unwrap_or(0.5), "similarity_boost": 0.75, "style": req.style.unwrap_or(0.0)});
            if let Some(s) = req.speed {
                settings["speed"] = json!(s.clamp(0.7, 1.2));
            }
            (
                format!("{root}/v1/text-to-speech/{id}"),
                json!({"text": req.prompt, "model_id": model.remote, "voice_settings": settings}),
                json!({"voiceId": id, "voice": name}),
            )
        }
        Kind::Sfx => {
            let mut b = json!({"text": req.prompt, "prompt_influence": 0.3});
            if let Some(d) = req.duration {
                b["duration_seconds"] = json!(d.clamp(0.5, 30.0));
            }
            (format!("{root}/v1/sound-generation"), b, json!({}))
        }
        Kind::Music => {
            let ms = (req.duration.unwrap_or(30.0).clamp(3.0, 600.0) * 1000.0).round() as i64;
            (
                format!("{root}/v1/music"),
                json!({"prompt": req.prompt, "music_length_ms": ms, "model_id": model.remote, "force_instrumental": req.instrumental}),
                json!({"lengthMs": ms}),
            )
        }
        _ => return Err(GenError::new("model.kind", "ElevenLabs models here make voice, music and sound effects")),
    };
    let resp = agent()
        .post(&url)
        .set("xi-api-key", r.key)
        .query("output_format", FORMAT)
        .send_json(body)
        .map_err(|e| http_error("ElevenLabs", e))?;
    let data = download_to(resp, r.out_dir, MAX_AUDIO)?;
    if (r.cancelled)() {
        return Err(GenError::new("cancelled", "cancelled"));
    }
    if data.len() < 64 {
        return Err(GenError::new("provider.failed", "ElevenLabs returned no audio"));
    }
    Ok(Output { data, ext: "mp3".into(), details })
}
