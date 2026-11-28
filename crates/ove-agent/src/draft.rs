//! Drafting a skill from a project with the configured assistant (add-agent-skills-and-connectors D6): one request
//! with our drafting instructions and the project's context pack (`ove_tools::skills::draft`). The draft is returned
//! for the person to review; nothing is saved here.

use std::sync::atomic::AtomicBool;

use serde_json::json;

use crate::api::{ApiError, Client, StreamEvent, Usage, pricing_model};
use ove_tools::skills::draft::{INSTRUCTIONS, Pack, finish, request};

/// What drafting returns: the cleaned draft and what the request cost.
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    pub text: String,
    pub usage: Usage,
    pub cost_usd: f64,
}

/// Asks `model` for a skill drafted from `pack`.
pub fn draft_skill(client: &Client, model: &str, pack: &Pack, cancel: &AtomicBool) -> Result<Draft, ApiError> {
    let body = json!({
        "model": model,
        "max_tokens": 8000,
        "stream": true,
        "system": INSTRUCTIONS,
        "messages": [{"role": "user", "content": request(pack)}],
        "thinking": {"type": "disabled"},
        "output_config": {"effort": "medium"},
    });
    let mut usage = Usage::default();
    let mut cost = 0.0;
    let resp = client.stream(
        &body,
        cancel,
        &mut |e| {
            if let StreamEvent::Usage { model: served, usage: u } = e {
                usage.add(&u);
                cost += u.cost_usd(pricing_model(&served, model));
            }
        },
        &mut |_, _| {},
    )?;
    let text: String = resp
        .content
        .iter()
        .filter(|b| b["type"] == "text")
        .filter_map(|b| b["text"].as_str())
        .collect::<Vec<_>>()
        .join("");
    let text = finish(&text).map_err(|f| ApiError::new("draft.invalid", f.message))?;
    Ok(Draft { text, usage, cost_usd: cost })
}
