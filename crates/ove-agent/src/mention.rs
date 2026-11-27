//! Mentions (change add-agent-chat-context-and-activity, design D1): what the person points at in a message — a
//! timeline item, a time range, a media file or a skill — resolved when the message is sent to the exact lines the
//! assistant would print, with the revision they came from. The assistant can then edit a mentioned item in the same
//! turn without printing the timeline first; an edit against an older revision is merged by the stale-tolerant apply.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use ove_store::project::Project;

/// Lines of a range mention sent in full; the rest are counted.
pub const MAX_RANGE_LINES: usize = 40;
/// Images attached to one message.
pub const MAX_IMAGES: usize = 5;
/// Long side of an attached image, in pixels.
pub const IMAGE_LONG_SIDE: u32 = 1568;

/// What the person picked in the chat input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MentionRef {
    /// `item`, `range`, `media` or `skill`.
    pub kind: String,
    /// The item, media or skill id.
    #[serde(default)]
    pub id: Option<String>,
    /// A range's start and end, seconds.
    #[serde(default)]
    pub start: Option<f64>,
    #[serde(default)]
    pub end: Option<f64>,
}

/// A mention as sent and as kept in the chat log: frozen at send time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mention {
    /// `item`, `range`, `media` or `skill`.
    pub kind: String,
    /// The items (or the media, or the skill) it names.
    pub ids: Vec<String>,
    /// The revision the excerpt was printed from (none for skills).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rev: Option<i64>,
    /// What the chip shows.
    pub label: String,
    /// The exact printed lines (item and range), the media table row, or the skill's text.
    pub excerpt: String,
    /// A range's start and end, seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<[f64; 2]>,
    /// Items in a range left out after [`MAX_RANGE_LINES`] lines.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub more: usize,
    /// An attached image (PNG, base64), for image media.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// An image the caller should fetch for a media mention: index into the mentions, the media id and the width that
/// keeps the long side at most [`IMAGE_LONG_SIDE`].
#[derive(Debug, Clone, PartialEq)]
pub struct ImageWant {
    pub index: usize,
    pub media: String,
    pub width: u32,
}

/// Mentions resolved against a project, before images are attached.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Resolved {
    pub mentions: Vec<Mention>,
    /// Images to attach, at most [`MAX_IMAGES`].
    pub images: Vec<ImageWant>,
    /// Plain-language notices for the chat (images left out, items no longer on the timeline).
    pub notices: Vec<String>,
}

fn secs(us: i64) -> f64 {
    us as f64 / 1e6
}

fn quoted(s: &str) -> String {
    let short: String = s.chars().take(40).collect();
    format!("\u{201c}{short}\u{201d}")
}

/// The display name of an item: its name, its text, or its kind.
fn item_label(p: &Project, id: &str) -> String {
    let Some(i) = p.doc().project.items.get(id) else { return id.to_string() };
    ["name", "textContent", "captionText"]
        .iter()
        .find_map(|k| i.fields.get(*k).and_then(Value::as_str).filter(|s| !s.trim().is_empty()))
        .map(str::to_string)
        .unwrap_or_else(|| format!("{} {id}", i.kind))
}

/// The media table header and row of `media` in a printed header.
fn media_rows(text: &str, media: &str) -> Option<String> {
    let mut lines = text.lines().skip_while(|l| l.trim() != "* media:").skip(1);
    let head = lines.next()?;
    let row = lines.take_while(|l| l.starts_with(" *   ")).find(|l| l.split_whitespace().any(|w| w == media))?;
    Some(format!("{}\n{}", head.trim_start_matches(" *").trim(), row.trim_start_matches(" *").trim()))
}

/// Resolves what the person picked against the open project: item lines and range lines from the current
/// projection with its revision, media rows, and which images to attach (at most [`MAX_IMAGES`], the rest named in
/// a notice). Skills are resolved by the caller ([`skill`]).
pub fn resolve(p: &Project, refs: &[MentionRef]) -> Resolved {
    let text = p.projection();
    let rev = p.rev();
    let spans = ove_tools::edits::element_spans(&text);
    let doc = p.doc();
    let mut out = Resolved::default();
    let mut left_out: Vec<String> = Vec::new();
    for r in refs {
        match (r.kind.as_str(), r.id.as_deref()) {
            ("item", Some(id)) => match spans.iter().find(|s| s.id == id) {
                Some(s) => out.mentions.push(Mention {
                    kind: "item".into(),
                    ids: vec![id.to_string()],
                    rev: Some(rev),
                    label: item_label(p, id),
                    excerpt: s.text.clone(),
                    range: None,
                    more: 0,
                    image: None,
                }),
                None => out.notices.push(format!("{id} is no longer on the timeline, so it was not mentioned.")),
            },
            ("range", _) => {
                let (Some(a), Some(b)) = (r.start, r.end) else { continue };
                let (start, end) = (a.min(b).max(0.0), a.max(b).max(0.0));
                let overlaps =
                    |id: &str| doc.project.items.get(id).is_some_and(|i| secs(i.start) < end && secs(i.end) > start);
                let (ids, shown, more) = capped(&spans, overlaps);
                out.mentions.push(Mention {
                    kind: "range".into(),
                    ids,
                    rev: Some(rev),
                    label: format!("{start:.2}\u{2013}{end:.2} s"),
                    excerpt: shown.join("\n"),
                    range: Some([start, end]),
                    more,
                    image: None,
                });
            }
            ("track", Some(id)) => {
                let Some(t) = doc.project.tracks.iter().find(|t| t.id == id) else {
                    out.notices.push(format!("{id} is no longer a track, so it was not mentioned."));
                    continue;
                };
                let on = |item: &str| doc.project.items.get(item).is_some_and(|i| i.track_id == id);
                let (ids, shown, more) = capped(&spans, on);
                out.mentions.push(Mention {
                    kind: "track".into(),
                    ids,
                    rev: Some(rev),
                    label: t.name.clone(),
                    excerpt: shown.join("\n"),
                    range: None,
                    more,
                    image: None,
                });
            }
            ("media", Some(id)) => {
                let Some(m) = doc.project.media.get(id) else {
                    out.notices.push(format!("{id} is no longer in the project's media, so it was not mentioned."));
                    continue;
                };
                let data = m.data();
                let get = |k: &str| data.and_then(|d| d.get(k));
                let name = m.file_name().unwrap_or(id).to_string();
                out.mentions.push(Mention {
                    kind: "media".into(),
                    ids: vec![id.to_string()],
                    rev: Some(rev),
                    label: name.clone(),
                    excerpt: media_rows(&text, id).unwrap_or_else(|| format!("{name} ({id})")),
                    range: None,
                    more: 0,
                    image: None,
                });
                if get("type").and_then(Value::as_str) == Some("image") {
                    if out.images.len() >= MAX_IMAGES {
                        left_out.push(name);
                        continue;
                    }
                    let (w, h) = (
                        get("width").and_then(Value::as_u64).unwrap_or(0) as u32,
                        get("height").and_then(Value::as_u64).unwrap_or(0) as u32,
                    );
                    out.images.push(ImageWant {
                        index: out.mentions.len() - 1,
                        media: id.to_string(),
                        width: fit_width(w, h),
                    });
                }
            }
            _ => {}
        }
    }
    if !left_out.is_empty() {
        out.notices.push(too_many_images(&left_out));
    }
    out
}

/// The elements for which `keep` holds, in print order, up to [`MAX_RANGE_LINES`] lines: their ids, their text, and
/// how many more were left out.
fn capped(spans: &[ove_tools::edits::ElementSpan], keep: impl Fn(&str) -> bool) -> (Vec<String>, Vec<String>, usize) {
    let (mut ids, mut shown, mut more, mut lines) = (Vec::new(), Vec::new(), 0, 0);
    for s in spans.iter().filter(|s| keep(&s.id)) {
        let n = s.last - s.first + 1;
        if lines + n > MAX_RANGE_LINES {
            more += 1;
            continue;
        }
        lines += n;
        ids.push(s.id.clone());
        shown.push(s.text.clone());
    }
    (ids, shown, more)
}

/// The notice for images over the limit.
pub fn too_many_images(names: &[String]) -> String {
    let list = names.iter().map(|n| quoted(n)).collect::<Vec<_>>().join(", ");
    format!(
        "Only {MAX_IMAGES} images can be attached to one message. Not attached: {list}. They are still mentioned by name."
    )
}

/// The width that keeps an image's long side at most [`IMAGE_LONG_SIDE`] (its own width when smaller; a default
/// when its size is unknown).
pub fn fit_width(w: u32, h: u32) -> u32 {
    if w == 0 || h == 0 {
        return IMAGE_LONG_SIDE;
    }
    let long = w.max(h);
    if long <= IMAGE_LONG_SIDE {
        return w;
    }
    ((w as f64) * (IMAGE_LONG_SIDE as f64) / (long as f64)).round().max(1.0) as u32
}

/// A skill mention: the skill's text, read before the turn so the assistant follows it from the start.
pub fn skill(id: &str, name: &str, text: &str) -> Mention {
    Mention {
        kind: "skill".into(),
        ids: vec![id.to_string()],
        rev: None,
        label: name.to_string(),
        excerpt: text.to_string(),
        range: None,
        more: 0,
        image: None,
    }
}

/// The content blocks that tell the assistant about a message's mentions: one text block with every excerpt, then
/// the attached images.
pub fn blocks(mentions: &[Mention]) -> Vec<Value> {
    if mentions.is_empty() {
        return Vec::new();
    }
    let mut t = String::new();
    let skills: Vec<&Mention> = mentions.iter().filter(|m| m.kind == "skill").collect();
    for s in &skills {
        t.push_str(&format!(
            "The person chose the skill {} ({}) for this message. Follow it; its text is below.\n<skill id=\"{}\">\n{}\n</skill>\n\n",
            quoted(&s.label),
            s.ids.first().map(String::as_str).unwrap_or_default(),
            s.ids.first().map(String::as_str).unwrap_or_default(),
            s.excerpt.trim_end()
        ));
    }
    let rest: Vec<&Mention> = mentions.iter().filter(|m| m.kind != "skill").collect();
    if !rest.is_empty() {
        let revs: Vec<i64> = rest.iter().filter_map(|m| m.rev).collect();
        let rev = revs.iter().max().copied().unwrap_or_default();
        t.push_str(&format!(
            "The person mentioned these, printed from the timeline at rev {rev}. To edit them, use these exact lines as `oldText` with `rev: {rev}` in apply_timeline; you do not need to print the timeline first.\n"
        ));
        for m in rest {
            match m.kind.as_str() {
                "item" => t.push_str(&format!("\n@{} (item {}):\n{}\n", quoted(&m.label), m.ids.join(", "), m.excerpt)),
                "range" | "track" => {
                    let n = m.ids.len() + m.more;
                    match m.range {
                        Some([a, b]) => t.push_str(&format!("\n@range {a:.2}\u{2013}{b:.2} s ({n} item(s)):\n")),
                        None => t.push_str(&format!("\n@track {} ({n} item(s)):\n", quoted(&m.label))),
                    }
                    if !m.excerpt.is_empty() {
                        t.push_str(&m.excerpt);
                        t.push('\n');
                    }
                    if m.more > 0 {
                        t.push_str(&format!(
                            "[{} more item(s) are not shown; print the timeline to see them]\n",
                            m.more
                        ));
                    }
                }
                "media" => t.push_str(&format!(
                    "\n@{} (media {}{}):\n{}\n",
                    quoted(&m.label),
                    m.ids.join(", "),
                    if m.image.is_some() { ", image attached" } else { "" },
                    m.excerpt
                )),
                _ => {}
            }
        }
    }
    let mut out = vec![json!({"type": "text", "text": t.trim_end()})];
    for m in mentions {
        if let Some(d) = &m.image {
            out.push(json!({"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": d}}));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ove_core::ops::Op;
    use ove_core::revision::Source;
    use std::sync::Arc;

    fn project(dir: &std::path::Path) -> Project {
        let lib = ove_store::Library::open(dir, Arc::new(ove_core::SystemClock)).unwrap();
        let mut p = lib.create("M").unwrap();
        let mut ops = vec![
            Op::AddTrack { track: json!({"id": "t_main", "name": "Main", "order": 0, "type": "video"}) },
            Op::AddMedia {
                media: json!({"id": "m_img", "data": {"fileName": "poster.png", "type": "image", "width": 4000, "height": 2000}}),
                entry: None,
            },
            Op::AddMedia {
                media: json!({"id": "m_vid", "data": {"fileName": "beach.mp4", "type": "video"}}),
                entry: None,
            },
        ];
        for i in 0..50 {
            ops.push(Op::AddItem {
                item: json!({"id": format!("i_{i:02}"), "trackId": "t_main", "type": "text", "startTime": i, "endTime": i + 1, "name": format!("Card {i}"), "textContent": "x"}),
            });
        }
        p.apply(ops, Source::Gui, None).unwrap();
        p
    }

    #[test]
    fn items_ranges_and_media_resolve_to_printed_lines_with_their_rev() {
        let d = tempfile::tempdir().unwrap();
        let p = project(d.path());
        let refs = vec![
            MentionRef { kind: "item".into(), id: Some("i_03".into()), start: None, end: None },
            MentionRef { kind: "range".into(), id: None, start: Some(10.0), end: Some(12.5) },
            MentionRef { kind: "media".into(), id: Some("m_vid".into()), start: None, end: None },
            MentionRef { kind: "item".into(), id: Some("gone".into()), start: None, end: None },
        ];
        let r = resolve(&p, &refs);
        assert_eq!(r.mentions.len(), 3);
        let text = p.projection();
        let item = &r.mentions[0];
        assert_eq!((item.rev, item.label.as_str()), (Some(p.rev()), "Card 3"));
        assert!(item.excerpt.contains("id=\"i_03\""), "{}", item.excerpt);
        assert!(text.contains(&item.excerpt), "the excerpt is exact printed text, usable as oldText");
        assert_eq!(text.matches(&item.excerpt).count(), 1, "and unique");
        let range = &r.mentions[1];
        assert_eq!(range.ids, ["i_10", "i_11", "i_12"]);
        assert_eq!(range.range, Some([10.0, 12.5]));
        assert!(text.contains(&range.excerpt));
        let media = &r.mentions[2];
        assert!(media.excerpt.contains("m_vid") && media.excerpt.starts_with("name"), "{}", media.excerpt);
        assert!(r.images.is_empty());
        assert_eq!(r.notices.len(), 1);
        assert!(r.notices[0].contains("gone"));
    }

    #[test]
    fn a_long_range_is_capped_at_forty_lines_with_a_count() {
        let d = tempfile::tempdir().unwrap();
        let p = project(d.path());
        let r = resolve(&p, &[MentionRef { kind: "range".into(), id: None, start: Some(0.0), end: Some(100.0) }]);
        let m = &r.mentions[0];
        assert_eq!(m.excerpt.lines().count(), MAX_RANGE_LINES);
        assert_eq!((m.ids.len(), m.more), (40, 10));
        let b = blocks(&r.mentions);
        assert!(b[0]["text"].as_str().unwrap().contains("10 more item(s)"));
        // A track mention (a panel's hand-off of "the caption track") is capped the same way.
        let t = resolve(&p, &[MentionRef { kind: "track".into(), id: Some("t_main".into()), start: None, end: None }]);
        let m = &t.mentions[0];
        assert_eq!((m.kind.as_str(), m.label.as_str(), m.ids.len(), m.more), ("track", "Main", 40, 10));
        assert!(blocks(&t.mentions)[0]["text"].as_str().unwrap().contains("@track \u{201c}Main\u{201d} (50 item(s))"));
    }

    #[test]
    fn images_are_downscaled_and_limited_to_five_with_a_notice() {
        assert_eq!(fit_width(4000, 2000), 1568);
        assert_eq!(fit_width(1000, 4000), 392);
        assert_eq!(fit_width(800, 600), 800);
        assert_eq!(fit_width(0, 0), IMAGE_LONG_SIDE);
        let d = tempfile::tempdir().unwrap();
        let p = project(d.path());
        let refs: Vec<MentionRef> = (0..7)
            .map(|_| MentionRef { kind: "media".into(), id: Some("m_img".into()), start: None, end: None })
            .collect();
        let r = resolve(&p, &refs);
        assert_eq!(r.mentions.len(), 7);
        assert_eq!(r.images.len(), MAX_IMAGES);
        assert_eq!(r.images[0], ImageWant { index: 0, media: "m_img".into(), width: 1568 });
        assert_eq!(r.notices.len(), 1);
        assert!(r.notices[0].contains("Only 5 images") && r.notices[0].contains("poster.png"), "{}", r.notices[0]);
        assert_eq!(r.notices[0].matches("poster.png").count(), 2, "both left-out images are named");
    }

    #[test]
    fn blocks_carry_the_rev_the_excerpts_and_the_images() {
        let mut m = skill("color-match", "Colour match", "Step 1: compare.");
        let item = Mention {
            kind: "item".into(),
            ids: vec!["i_1".into()],
            rev: Some(7),
            label: "Title".into(),
            excerpt: "      <text id=\"i_1\" />".into(),
            range: None,
            more: 0,
            image: Some("AAAA".into()),
        };
        m.excerpt.push('\n');
        let b = blocks(&[m, item]);
        let t = b[0]["text"].as_str().unwrap();
        assert!(t.contains("<skill id=\"color-match\">\nStep 1: compare.\n</skill>"), "{t}");
        assert!(t.contains("rev: 7") && t.contains("<text id=\"i_1\" />"), "{t}");
        assert_eq!(b[1]["type"], "image");
        assert!(blocks(&[]).is_empty());
    }
}
