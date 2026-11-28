//! The edit contract (design D2, D3): text edits made against an older revision are applied to the text printed for
//! that revision and then merged onto the current document; every edit returns a receipt with the printed lines of
//! what it added or changed, so an agent can make its next edit without printing again.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use std::sync::{Arc, Mutex};

use ove_core::model::{Document, Item};
use ove_projection::textedit::{TextEdit, apply_edits};
use ove_store::project::Project;
use serde_json::{Value, json};

use crate::fail::Fail;

/// Printed revisions kept per project.
pub const KEEP_PRINTS: usize = 16;
/// Changed items listed one by one in a receipt.
pub const MAX_CHANGED: usize = 30;
/// Items on one track moved by the same amount that make a shift rule instead of rows.
pub const MIN_SHIFT: usize = 3;

struct Printed {
    bundle: PathBuf,
    rev: i64,
    text: Option<Arc<String>>,
    doc: Option<Document>,
}

/// Printed revisions and the documents at them, most recently used first.
static PRINTS: Mutex<Vec<Printed>> = Mutex::new(Vec::new());

fn key(p: &Path) -> PathBuf {
    dunce::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

fn store(bundle: &Path, rev: i64, text: Option<&str>, doc: Option<&Document>) {
    let bundle = key(bundle);
    let Ok(mut cache) = PRINTS.lock() else { return };
    let mut e = match cache.iter().position(|e| e.bundle == bundle && e.rev == rev) {
        Some(at) => cache.remove(at),
        None => Printed { bundle: bundle.clone(), rev, text: None, doc: None },
    };
    if let Some(t) = text {
        e.text = Some(Arc::new(t.to_string()));
    }
    if let Some(d) = doc {
        e.doc = Some(d.clone());
    }
    cache.insert(0, e);
    let mut seen = 0;
    cache.retain(|e| {
        if e.bundle != bundle {
            return true;
        }
        seen += 1;
        seen <= KEEP_PRINTS
    });
}

/// Remembers the text printed for a revision (what an agent saw), keeping the newest 16 revisions per project.
pub fn remember(bundle: &Path, rev: i64, text: &str) {
    store(bundle, rev, Some(text), None);
}

/// Remembers the document at a revision, so a later edit against it needs no replay.
pub fn remember_doc(bundle: &Path, rev: i64, doc: &Document) {
    store(bundle, rev, None, Some(doc));
}

fn recall(bundle: &Path, rev: i64) -> Option<Arc<String>> {
    let bundle = key(bundle);
    let mut cache = PRINTS.lock().ok()?;
    let at = cache.iter().position(|e| e.bundle == bundle && e.rev == rev && e.text.is_some())?;
    let e = cache.remove(at);
    let text = e.text.clone();
    cache.insert(0, e);
    text
}

/// The document at revision `rev`: cached, or rebuilt from the revision log.
pub fn doc_at(p: &Project, rev: i64) -> ove_store::Result<Document> {
    if rev == p.rev() {
        return Ok(p.doc().clone());
    }
    let bundle = key(p.bundle());
    if let Some(d) = PRINTS
        .lock()
        .ok()
        .and_then(|c| c.iter().find(|e| e.bundle == bundle && e.rev == rev).and_then(|e| e.doc.clone()))
    {
        return Ok(d);
    }
    let d = p.doc_at(rev)?;
    remember_doc(p.bundle(), rev, &d);
    Ok(d)
}

/// How many printed revisions of a project are cached (tests).
pub fn cached(bundle: &Path) -> usize {
    let bundle = key(bundle);
    PRINTS.lock().map(|c| c.iter().filter(|e| e.bundle == bundle).count()).unwrap_or(0)
}

/// The projection printed for revision `rev` of the project: the text an agent saw when it printed then (cached), or
/// the document at that revision printed again.
pub fn printed_at(p: &Project, rev: i64) -> Result<Arc<String>, Fail> {
    if let Some(t) = recall(p.bundle(), rev) {
        return Ok(t);
    }
    let text = if rev == p.rev() {
        p.projection()
    } else {
        let doc = doc_at(p, rev).map_err(|e| {
            let code = match e.code() {
                "revision.newer" => "rev.newer",
                "revision.missing" => "rev.pruned",
                c => c,
            };
            Fail::new(
                code,
                format!(
                    "The timeline's revision {rev} is not available (it is now rev {}). Print it again and redo your edits against the new text.",
                    p.rev()
                ),
            )
        })?;
        ove_projection::print(&doc, &p.print_context())
    };
    remember(p.bundle(), rev, &text);
    Ok(Arc::new(text))
}

/// The projection text for a text-edit apply: the edits applied to the text printed for `rev` (the current revision
/// when `rev` is not given). Its header names `rev`, so the apply merges it onto the current document.
pub fn edited_text(p: &Project, rev: Option<i64>, edits: &[TextEdit]) -> Result<String, Fail> {
    let rev = rev.unwrap_or_else(|| p.rev());
    let printed = printed_at(p, rev)?;
    apply_edits(&printed, edits).map_err(|e| {
        let code = if e.code == "edit.notFound" { "edit.anchor" } else { e.code };
        let message = if code == "edit.anchor" {
            format!("edit {}: oldText was not found in the timeline printed for rev {rev}; print it again", e.index)
        } else {
            format!("{}. Print the timeline again if it changed.", e.message)
        };
        Fail::new(code, message).details(json!({"index": e.index, "positions": e.positions}))
    })
}

/// The printed element of each item in a projection text: id → its full text (one line, or the lines of an
/// element whose attributes are written one per line). Each is unique in the text, so it can be the `oldText` of
/// the next edit.
pub fn elements(text: &str) -> BTreeMap<String, String> {
    element_spans(text).into_iter().map(|s| (s.id, s.text)).collect()
}

/// One item's printed element in a projection text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElementSpan {
    pub id: String,
    /// First and last line of the element, 1-based (the numbering `print_timeline`'s `from` uses).
    pub first: usize,
    pub last: usize,
    /// The element's lines.
    pub text: String,
}

/// The printed elements of a projection text in print order (tracks top to bottom, then start time), with the
/// lines each one spans. Mentions quote them; activity maps a printed line range to the items it shows.
pub fn element_spans(text: &str) -> Vec<ElementSpan> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim_start();
        let is_item =
            t.starts_with('<') && !t.starts_with("</") && !t.starts_with("<track") && !t.starts_with("<timeline");
        if !is_item {
            i += 1;
            continue;
        }
        let start = i;
        if !t.ends_with("/>") {
            // Attributes one per line, closed by a `/>` line.
            while i < lines.len() && lines[i].trim() != "/>" {
                i += 1;
            }
        }
        let end = i.min(lines.len().saturating_sub(1));
        let block = lines[start..=end].join("\n");
        if let Some(id) = attr_id(&block) {
            out.push(ElementSpan { id, first: start + 1, last: end + 1, text: block });
        }
        i = end + 1;
    }
    out
}

fn attr_id(block: &str) -> Option<String> {
    // `id` is the first attribute: `id="…"`, or `id={"…"}` for a string that needs escapes.
    let at = block.find(" id=")? + 4;
    let rest = &block[at..];
    if let Some(r) = rest.strip_prefix('"') {
        return Some(r[..r.find('"')?].to_string());
    }
    let r = rest.strip_prefix('{')?;
    let mut de = serde_json::Deserializer::from_str(r).into_iter::<String>();
    de.next()?.ok()
}

fn secs(us: i64) -> f64 {
    us as f64 / 1e6
}

/// True when `after` is `before` moved in time (same track, same length, every other field equal).
fn pure_shift(before: &Item, after: &Item) -> Option<i64> {
    let d = after.start - before.start;
    let same = d != 0
        && after.end - before.end == d
        && after.track_id == before.track_id
        && after.kind == before.kind
        && after.fields == before.fields;
    same.then_some(d)
}

/// The printed elements of some items of a document (printed alone: the lines are those of the full print).
fn print_items(doc: &Document, ids: &BTreeSet<&str>) -> BTreeMap<String, String> {
    let mut only = doc.clone();
    only.project.items = doc
        .project
        .items
        .iter()
        .filter(|(id, _)| ids.contains(id.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    elements(&ove_projection::print(&only, &ove_projection::PrintContext::default()))
}

/// The receipt fields of an edit that turned `before` into `after`:
/// `changedItems` (id and printed element of each added or changed item, at most 30, then `moreChanged`),
/// `removedItems` (id and name), `shifted` (one rule per group of 3 or more items moved together on a track) and
/// `trackOrderChanged`.
pub fn receipt(before: &Document, after: &Document) -> Value {
    let mut removed = Vec::new();
    for (id, i) in &before.project.items {
        if !after.project.items.contains_key(id) {
            let name = i.fields.get("name").and_then(Value::as_str).unwrap_or_default();
            removed.push(json!({"id": id, "name": name}));
        }
    }
    // Pure moves, grouped by track and amount.
    let mut moves: BTreeMap<(String, i64), Vec<(&str, i64)>> = BTreeMap::new();
    let mut changed: Vec<&Item> = Vec::new();
    for (id, a) in &after.project.items {
        match before.project.items.get(id) {
            None => changed.push(a),
            Some(b) if b == a => {}
            Some(b) => match pure_shift(b, a) {
                Some(d) => moves.entry((a.track_id.clone(), d)).or_default().push((id.as_str(), b.start)),
                None => changed.push(a),
            },
        }
    }
    let mut shifted = Vec::new();
    for ((track, by), items) in &moves {
        if items.len() >= MIN_SHIFT {
            let from = items.iter().map(|(_, s)| *s).min().unwrap_or(0);
            shifted.push(json!({"track": track, "from": secs(from), "by": secs(*by), "count": items.len()}));
        } else {
            changed.extend(items.iter().filter_map(|(id, _)| after.item(id)));
        }
    }
    // In print order: tracks top to bottom, then start time.
    let track_at: BTreeMap<&str, usize> =
        after.project.tracks.iter().enumerate().map(|(n, t)| (t.id.as_str(), n)).collect();
    changed.sort_by(|a, b| {
        (track_at.get(a.track_id.as_str()), a.start, &a.id).cmp(&(track_at.get(b.track_id.as_str()), b.start, &b.id))
    });
    let shown: BTreeSet<&str> = changed.iter().take(MAX_CHANGED).map(|i| i.id.as_str()).collect();
    let printed = print_items(after, &shown);
    let listed: Vec<Value> = changed
        .iter()
        .take(MAX_CHANGED)
        .map(|i| json!({"id": i.id, "text": printed.get(&i.id).cloned().unwrap_or_default()}))
        .collect();
    let common: BTreeSet<&str> = before
        .project
        .tracks
        .iter()
        .map(|t| t.id.as_str())
        .filter(|id| after.project.tracks.iter().any(|t| t.id == *id))
        .collect();
    let order = |d: &Document| -> Vec<String> {
        d.project.tracks.iter().filter(|t| common.contains(t.id.as_str())).map(|t| t.id.clone()).collect()
    };
    json!({
        "changedItems": listed,
        "moreChanged": changed.len().saturating_sub(MAX_CHANGED),
        "removedItems": removed,
        "shifted": shifted,
        "trackOrderChanged": order(before) != order(after),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elements_are_found_on_one_line_or_many() {
        let text = "/*\n * rev: 1\n */\nexport default (\n  <timeline aspect=\"16:9\">\n    <track id=\"t\" name=\"T\" kind=\"video\">\n      <text id=\"a\" name=\"A\" start={0} end={1} />\n      <text\n        id=\"b\"\n        name=\"B\"\n      />\n    </track>\n  </timeline>\n);\n";
        let e = elements(text);
        assert_eq!(e["a"], "      <text id=\"a\" name=\"A\" start={0} end={1} />");
        assert_eq!(e["b"], "      <text\n        id=\"b\"\n        name=\"B\"\n      />");
        assert_eq!(e.len(), 2);
    }

    #[test]
    fn the_print_cache_keeps_sixteen_per_project() {
        let d = std::env::temp_dir().join(format!("ove-tools-cache-{}", std::process::id()));
        for r in 0..40 {
            remember(&d, r, "x");
        }
        assert_eq!(cached(&d), KEEP_PRINTS);
        assert!(recall(&d, 39).is_some());
        assert!(recall(&d, 10).is_none());
    }
}
