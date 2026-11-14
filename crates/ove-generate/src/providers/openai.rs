//! OpenAI Images: `POST /v1/images/generations` with `Authorization: Bearer <key>`; GPT Image models answer with
//! base64 PNG data (`data[0].b64_json`).

use serde_json::{Value, json};

use super::{Data, GenError, Output, Result, Run, agent, b64_decode, base, http_error};
use crate::catalog::{Kind, Model};
use crate::job::Request;

pub fn size(aspect: &str) -> &'static str {
    match aspect {
        "1:1" => "1024x1024",
        "9:16" | "3:4" | "4:5" | "2:3" => "1024x1536",
        _ => "1536x1024",
    }
}

pub fn run(model: &Model, req: &Request, aspect: &str, r: &mut Run) -> Result<Output> {
    if model.kind != Kind::Image {
        return Err(GenError::new("model.kind", "OpenAI models here make images"));
    }
    let root = base("OVE_OPENAI_URL", "https://api.openai.com");
    (r.progress)(Some(0.1), None);
    let v: Value = agent()
        .post(&format!("{root}/v1/images/generations"))
        .set("Authorization", &format!("Bearer {}", r.key))
        .send_json(json!({"model": model.remote, "prompt": req.prompt, "size": size(aspect), "n": 1}))
        .map_err(|e| http_error("OpenAI", e))?
        .into_json()
        .map_err(|e| GenError::new("provider.failed", format!("OpenAI: {e}")))?;
    if (r.cancelled)() {
        return Err(GenError::new("cancelled", "cancelled"));
    }
    let b64 = v
        .pointer("/data/0/b64_json")
        .and_then(Value::as_str)
        .ok_or_else(|| GenError::new("provider.failed", "OpenAI: no image in the reply"))?;
    Ok(Output {
        data: Data::Bytes(b64_decode(b64)?),
        ext: "png".into(),
        details: json!({"size": size(aspect), "revisedPrompt": v.pointer("/data/0/revised_prompt")}),
    })
}
