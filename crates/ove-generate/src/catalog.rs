//! The models people can pick. Each names its provider and the provider's own model id; a `fal:<endpoint>` id
//! that is not listed is passed through, so new fal models work without an update.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Image,
    Video,
    Voice,
    Music,
    Sfx,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Image => "image",
            Kind::Video => "video",
            Kind::Voice => "voice",
            Kind::Music => "music",
            Kind::Sfx => "sfx",
        }
    }

    /// Item type of the clip it makes.
    pub fn item_type(self) -> &'static str {
        match self {
            Kind::Image => "image",
            Kind::Video => "video",
            _ => "audio",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        Some(match s {
            "image" => Kind::Image,
            "video" => Kind::Video,
            "voice" => Kind::Voice,
            "music" => Kind::Music,
            "sfx" => Kind::Sfx,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Fal,
    OpenAi,
    ElevenLabs,
}

impl Provider {
    pub const ALL: [Provider; 3] = [Provider::Fal, Provider::OpenAi, Provider::ElevenLabs];

    pub fn id(self) -> &'static str {
        match self {
            Provider::Fal => "fal",
            Provider::OpenAi => "openai",
            Provider::ElevenLabs => "elevenlabs",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Provider::Fal => "fal.ai",
            Provider::OpenAi => "OpenAI",
            Provider::ElevenLabs => "ElevenLabs",
        }
    }

    /// Where people create a key.
    pub fn key_url(self) -> &'static str {
        match self {
            Provider::Fal => "https://fal.ai/dashboard/keys",
            Provider::OpenAi => "https://platform.openai.com/api-keys",
            Provider::ElevenLabs => "https://elevenlabs.io/app/settings/api-keys",
        }
    }

    pub fn parse(s: &str) -> Option<Provider> {
        Provider::ALL.into_iter().find(|p| p.id() == s.to_ascii_lowercase())
    }
}

/// How a model is priced, for the estimate shown before spending (list prices; the provider's bill is what counts).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "per")]
pub enum Price {
    /// Per generated image.
    Image { usd: f64 },
    /// Per second of output.
    Second { usd: f64 },
    /// Per 1000 characters of text.
    KiloChar { usd: f64 },
    /// Not known (a passed-through model).
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    /// Our id: `<provider>:<name>`.
    pub id: String,
    pub provider: Provider,
    pub kind: Kind,
    pub label: String,
    /// The provider's model / endpoint id.
    pub remote: String,
    /// For image-to-video: the provider endpoint that takes a first frame.
    pub remote_from_image: Option<String>,
    /// Allowed durations in seconds (video); empty = free choice within `max_seconds`.
    pub durations: Vec<f64>,
    pub max_seconds: f64,
    pub aspects: Vec<String>,
    pub price: Price,
    /// The default for its kind.
    pub default: bool,
}

fn m(id: &str, provider: Provider, kind: Kind, label: &str, remote: &str) -> Model {
    Model {
        id: id.into(),
        provider,
        kind,
        label: label.into(),
        remote: remote.into(),
        remote_from_image: None,
        durations: vec![],
        max_seconds: 0.0,
        aspects: vec![],
        price: Price::Unknown,
        default: false,
    }
}

fn aspects(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

/// The listed models.
pub fn models() -> Vec<Model> {
    use Kind::*;
    use Provider::*;
    let img = aspects(&["16:9", "9:16", "1:1", "4:3", "3:4"]);
    vec![
        Model {
            price: Price::Image { usd: 0.025 },
            aspects: img.clone(),
            default: true,
            ..m("fal:flux-dev", Fal, Image, "FLUX.1 [dev]", "fal-ai/flux/dev")
        },
        Model {
            price: Price::Image { usd: 0.003 },
            aspects: img.clone(),
            ..m("fal:flux-schnell", Fal, Image, "FLUX.1 [schnell] (fast)", "fal-ai/flux/schnell")
        },
        Model {
            price: Price::Image { usd: 0.04 },
            aspects: aspects(&["16:9", "9:16", "1:1", "4:3", "3:4"]),
            ..m("fal:imagen4", Fal, Image, "Imagen 4", "fal-ai/imagen4/preview")
        },
        Model {
            price: Price::Image { usd: 0.04 },
            aspects: aspects(&["1:1", "3:2", "2:3"]),
            ..m("openai:gpt-image-1", OpenAi, Image, "GPT Image 1", "gpt-image-1")
        },
        Model {
            price: Price::Second { usd: 0.056 },
            durations: vec![5.0, 10.0],
            max_seconds: 10.0,
            aspects: aspects(&["16:9", "9:16", "1:1"]),
            remote_from_image: Some("fal-ai/kling-video/v2.1/standard/image-to-video".into()),
            default: true,
            ..m("fal:kling-2.1", Fal, Video, "Kling 2.1", "fal-ai/kling-video/v2.1/master/text-to-video")
        },
        Model {
            price: Price::Second { usd: 0.4 },
            durations: vec![8.0],
            max_seconds: 8.0,
            aspects: aspects(&["16:9", "9:16"]),
            remote_from_image: Some("fal-ai/veo3/fast/image-to-video".into()),
            ..m("fal:veo3-fast", Fal, Video, "Veo 3 Fast (with sound)", "fal-ai/veo3/fast")
        },
        Model {
            price: Price::KiloChar { usd: 0.3 },
            default: true,
            ..m("elevenlabs:multilingual-v2", ElevenLabs, Voice, "Eleven Multilingual v2", "eleven_multilingual_v2")
        },
        Model {
            price: Price::KiloChar { usd: 0.3 },
            ..m("elevenlabs:v3", ElevenLabs, Voice, "Eleven v3 (expressive)", "eleven_v3")
        },
        Model {
            price: Price::Second { usd: 0.01 },
            max_seconds: 300.0,
            default: true,
            ..m("elevenlabs:music", ElevenLabs, Music, "Eleven Music", "music_v1")
        },
        Model {
            price: Price::Second { usd: 0.004 },
            max_seconds: 30.0,
            default: true,
            ..m("elevenlabs:sound-effects", ElevenLabs, Sfx, "Sound effects", "sound-generation")
        },
    ]
}

/// A model by our id, or a passed-through `fal:<endpoint>` for `kind`.
pub fn find(id: Option<&str>, kind: Kind) -> Result<Model, String> {
    let all = models();
    match id {
        None => all
            .into_iter()
            .find(|m| m.kind == kind && m.default)
            .ok_or_else(|| format!("no default {} model", kind.as_str())),
        Some(q) => {
            if let Some(found) = all.iter().find(|m| m.id == q || m.remote == q) {
                if found.kind != kind {
                    return Err(format!("{q} makes {}, not {}", found.kind.as_str(), kind.as_str()));
                }
                return Ok(found.clone());
            }
            match q.strip_prefix("fal:") {
                Some(endpoint) if matches!(kind, Kind::Image | Kind::Video) && endpoint.contains('/') => {
                    Ok(Model { max_seconds: 30.0, ..m(q, Provider::Fal, kind, endpoint, endpoint) })
                }
                _ => Err(format!(
                    "unknown model {q} (`ove generate models` lists them; any fal endpoint works as fal:<owner>/<model>)"
                )),
            }
        }
    }
}

/// Estimated cost in USD for a request, when the model's price is known.
pub fn estimate(model: &Model, seconds: f64, chars: usize, count: u32) -> Option<f64> {
    let v = match model.price {
        Price::Image { usd } => usd * count.max(1) as f64,
        Price::Second { usd } => usd * seconds.max(0.0),
        Price::KiloChar { usd } => usd * chars as f64 / 1000.0,
        Price::Unknown => return None,
    };
    Some((v * 1000.0).round() / 1000.0)
}
