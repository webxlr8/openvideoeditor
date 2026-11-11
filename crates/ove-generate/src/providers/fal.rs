//! fal.ai queue API: `POST https://queue.fal.run/<endpoint>` with `Authorization: Key <key>`, then poll the returned
//! `status_url` (IN_QUEUE → IN_PROGRESS → COMPLETED) and read `response_url`. Cancel with `PUT cancel_url`.

use serde_json::{Map, Value, json};

use super::{GenError, Output, Result, Run, agent, base, data_uri, ext_for, fetch, http_error, poll_interval};
use crate::catalog::{Kind, Model};
use crate::job::Request;

/// FLUX-family `image_size` for an aspect ratio.
fn image_size(aspect: &str) -> &'static str {
    match aspect {
        "9:16" => "portrait_16_9",
        "3:4" | "4:5" | "2:3" => "portrait_4_3",
        "1:1" => "square_hd",
        "4:3" | "3:2" => "landscape_4_3",
        _ => "landscape_16_9",
    }
}

pub fn input(model: &Model, req: &Request, aspect: &str) -> Result<(String, Value)> {
    let mut i = Map::new();
    i.insert("prompt".into(), req.prompt.clone().into());
    if let Some(s) = req.seed {
        i.insert("seed".into(), s.into());
    }
    if let Some(n) = req.negative.as_deref().filter(|n| !n.is_empty()) {
        i.insert("negative_prompt".into(), n.into());
    }
    let mut endpoint = model.remote.clone();
    match model.kind {
        Kind::Image => {
            if model.remote.starts_with("fal-ai/flux") {
                i.insert("image_size".into(), image_size(aspect).into());
                i.insert("num_images".into(), 1.into());
                i.insert("output_format".into(), "png".into());
            } else {
                i.insert("aspect_ratio".into(), aspect.into());
            }
            if let Some(img) = &req.image {
                i.insert("image_url".into(), data_uri(img)?.into());
            }
        }
        Kind::Video => {
            if let Some(img) = &req.image {
                endpoint = model.remote_from_image.clone().unwrap_or(endpoint);
                i.insert("image_url".into(), data_uri(img)?.into());
            }
            let secs = req.duration.or_else(|| model.durations.first().copied()).unwrap_or(5.0);
            let d = if model.remote.contains("veo") {
                format!("{}s", secs.round() as i64)
            } else {
                format!("{}", secs.round() as i64)
            };
            i.insert("duration".into(), d.into());
            i.insert("aspect_ratio".into(), aspect.into());
        }
        _ => return Err(GenError::new("model.kind", "fal models here make images and video")),
    }
    Ok((endpoint, Value::Object(i)))
}

/// The output URL in a result body (image, video or audio models).
pub fn output_url(v: &Value) -> Option<&str> {
    v.pointer("/images/0/url")
        .or_else(|| v.pointer("/video/url"))
        .or_else(|| v.pointer("/image/url"))
        .or_else(|| v.pointer("/audio/url"))
        .or_else(|| v.pointer("/videos/0/url"))
        .and_then(Value::as_str)
}

pub fn run(model: &Model, req: &Request, aspect: &str, r: &mut Run) -> Result<Output> {
    let (endpoint, body) = input(model, req, aspect)?;
    let root = base("OVE_FAL_QUEUE_URL", "https://queue.fal.run");
    let auth = format!("Key {}", r.key);
    let a = agent();
    let sub: Value = a
        .post(&format!("{root}/{endpoint}"))
        .set("Authorization", &auth)
        .send_json(body)
        .map_err(|e| http_error("fal.ai", e))?
        .into_json()
        .map_err(|e| GenError::new("provider.failed", format!("fal.ai: {e}")))?;
    let field = |k: &str| sub.get(k).and_then(Value::as_str).map(str::to_string);
    let (Some(status_url), Some(response_url)) = (field("status_url"), field("response_url")) else {
        return Err(GenError::new("provider.failed", format!("fal.ai: unexpected submit reply {sub}")));
    };
    let request_id = field("request_id");
    (r.progress)(Some(0.0), request_id.as_deref());
    loop {
        if (r.cancelled)() {
            if let Some(c) = field("cancel_url") {
                let _ = a.put(&c).set("Authorization", &auth).call();
            }
            return Err(GenError::new("cancelled", "cancelled"));
        }
        std::thread::sleep(poll_interval());
        let st: Value = a
            .get(&status_url)
            .set("Authorization", &auth)
            .call()
            .map_err(|e| http_error("fal.ai", e))?
            .into_json()
            .map_err(|e| GenError::new("provider.failed", format!("fal.ai: {e}")))?;
        match st.get("status").and_then(Value::as_str) {
            Some("COMPLETED") => {
                if let Some(e) = st.get("error").filter(|e| !e.is_null()) {
                    return Err(GenError::new(
                        "provider.failed",
                        format!("fal.ai: {}", e.as_str().map(str::to_string).unwrap_or_else(|| e.to_string())),
                    ));
                }
                break;
            }
            Some("IN_PROGRESS") => (r.progress)(Some(0.5), None),
            Some("IN_QUEUE") => (r.progress)(Some(0.05), None),
            other => return Err(GenError::new("provider.failed", format!("fal.ai: status {other:?}"))),
        }
    }
    let result: Value = a
        .get(&response_url)
        .set("Authorization", &auth)
        .call()
        .map_err(|e| http_error("fal.ai", e))?
        .into_json()
        .map_err(|e| GenError::new("provider.failed", format!("fal.ai: {e}")))?;
    let url = output_url(&result)
        .ok_or_else(|| GenError::new("provider.failed", format!("fal.ai: no output in {result}")))?;
    let (bytes, mime) = fetch(url)?;
    let fallback = if model.kind == Kind::Video { "mp4" } else { "png" };
    Ok(Output {
        ext: ext_for(&mime, url, fallback),
        bytes,
        details: json!({"endpoint": endpoint, "requestId": request_id, "seed": result.get("seed")}),
    })
}
