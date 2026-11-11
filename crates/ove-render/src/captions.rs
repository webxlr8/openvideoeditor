//! Caption rendering with the caption styles (`captions_presets`): per frame at item-relative time `rel_us` it picks
//! the segment being spoken, lays its words out, and draws each word with the style's mark (colour, box, ring or
//! underline on the spoken word) and animation (pop, pulse, reveal, stacked groups, spotlight).

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use ove_core::time::{Micros, json_to_micros};
use serde_json::{Map, Value};

use crate::canvas::{DrawParams, Place, Rgba, draw};
use crate::captions_presets::{Align, Anim, CaptionPreset, Case, DEFAULT_PRESET, Mark, Position, preset};
use crate::text::{Align as TextAlign, TextStyle, parse_color, render_words};

/// One word with item-relative times. Words without timings get times spread over their segment.
#[derive(Debug, Clone, PartialEq)]
pub struct TimedWord {
    pub text: String,
    pub start: Micros,
    pub end: Micros,
}

/// The words of the segment active at `rel_us`, with timings (`None` when the item has no timed segments).
pub fn timed_words(fields: &Map<String, Value>, rel_us: Micros) -> Option<Vec<TimedWord>> {
    let segs = fields.get("captionSegments")?.as_array()?;
    for s in segs {
        let (Some(a), Some(b)) =
            (s.get("startTime").and_then(json_to_micros), s.get("endTime").and_then(json_to_micros))
        else {
            continue;
        };
        if rel_us < a || rel_us >= b {
            continue;
        }
        if let Some(list) = s.get("wordTimings").and_then(Value::as_array).filter(|w| !w.is_empty()) {
            let words: Vec<TimedWord> = list
                .iter()
                .filter_map(|w| {
                    Some(TimedWord {
                        text: w.get("word")?.as_str()?.to_string(),
                        start: w.get("start").and_then(json_to_micros)?,
                        end: w.get("end").and_then(json_to_micros)?,
                    })
                })
                .collect();
            if !words.is_empty() {
                return Some(words);
            }
        }
        // No word timings: spread the segment's time over its words by length.
        let text = s.get("text").and_then(Value::as_str).unwrap_or("");
        let words: Vec<&str> = text.split_whitespace().collect();
        let total: usize = words.iter().map(|w| w.chars().count() + 1).sum();
        let mut t = a as f64;
        let per = (b - a) as f64 / total.max(1) as f64;
        return Some(
            words
                .iter()
                .map(|w| {
                    let start = t;
                    t += per * (w.chars().count() + 1) as f64;
                    TimedWord { text: w.to_string(), start: start as Micros, end: t as Micros }
                })
                .collect(),
        );
    }
    None
}

/// Rasterised words, reused across frames.
#[derive(Default)]
pub struct CaptionCache {
    words: HashMap<u64, Arc<Rgba>>,
}

impl CaptionCache {
    fn word(&mut self, text: &str, st: &TextStyle) -> Arc<Rgba> {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut h);
        format!("{st:?}").hash(&mut h);
        let key = h.finish();
        if let Some(img) = self.words.get(&key) {
            return img.clone();
        }
        if self.words.len() > 600 {
            self.words.clear();
        }
        let img = Arc::new(render_words(&[text.to_string()], st));
        self.words.insert(key, img.clone());
        img
    }
}

/// Where and how big: output pixels per reference pixel, item offset and scale.
#[derive(Debug, Clone, Copy)]
pub struct CaptionPlacement {
    pub k: f32,
    pub dx: f64,
    pub dy: f64,
    pub scale: f64,
    pub opacity: f32,
}

fn color(f: &Map<String, Value>, k: &str) -> Option<[u8; 4]> {
    f.get(k).and_then(Value::as_str).and_then(parse_color)
}

/// The preset of an item: its `captionPresetId`, else the default style.
pub fn item_preset(fields: &Map<String, Value>) -> &'static CaptionPreset {
    fields
        .get("captionPresetId")
        .and_then(Value::as_str)
        .and_then(preset)
        .unwrap_or_else(|| preset(DEFAULT_PRESET).expect("default preset"))
}

/// The item's style: the preset with the item's own overrides (size, colours, position).
pub fn resolve(fields: &Map<String, Value>) -> CaptionPreset {
    let mut p = *item_preset(fields);
    if let Some(s) = fields.get("captionFontSize").and_then(Value::as_f64).filter(|s| *s > 0.0) {
        p.size = s as f32;
    }
    if let Some(c) = color(fields, "captionTextColor") {
        p.color = c;
    }
    if let Some(c) = color(fields, "captionHighlightColor") {
        p.mark = p.mark.with_color(c);
    }
    let pos = match fields.get("captionPosition") {
        Some(Value::String(s)) => Position::parse(s),
        Some(Value::Object(o)) => {
            o.get("preset").or_else(|| o.get("position")).and_then(Value::as_str).and_then(Position::parse)
        }
        _ => None,
    };
    if let Some(pos) = pos {
        p.position = pos;
    }
    p
}

fn ease_out(p: f32) -> f32 {
    let p = p.clamp(0.0, 1.0);
    1.0 - (1.0 - p) * (1.0 - p) * (1.0 - p)
}

/// 0 → 1 with a small overshoot.
fn back_out(p: f32) -> f32 {
    let p = p.clamp(0.0, 1.0);
    let c1 = 1.70158;
    let c3 = c1 + 1.0;
    1.0 + c3 * (p - 1.0).powi(3) + c1 * (p - 1.0).powi(2)
}

fn cased(s: &str, case: Case) -> String {
    match case {
        Case::AsWritten => s.to_string(),
        Case::Upper => s.to_uppercase(),
        Case::Lower => s.to_lowercase(),
    }
}

/// An anti-aliased elliptical ring (the marker circle).
fn ring(w: u32, h: u32, thickness: f32, c: [u8; 4]) -> Rgba {
    let mut img = Rgba::new(w, h);
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let (rx, ry) = (cx - thickness, cy - thickness);
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = ((x as f32 + 0.5 - cx) / rx, (y as f32 + 0.5 - cy) / ry);
            // Distance to the ellipse, approximately in pixels.
            let d = ((dx * dx + dy * dy).sqrt() - 1.0) * rx.min(ry);
            let cov = (thickness / 2.0 + 0.5 - d.abs()).clamp(0.0, 1.0);
            if cov > 0.0 {
                let i = ((y * w + x) * 4) as usize;
                img.data[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], (c[3] as f32 * cov) as u8]);
            }
        }
    }
    img
}

/// Fills a (rotated, rounded) box centred at (cx, cy) of size (w, h).
fn rect(canvas: &mut Rgba, (cx, cy, w, h): (f64, f64, f64, f64), radius: f32, rotation: f64, c: [u8; 4], opacity: f32) {
    if w < 1.0 || h < 1.0 || c[3] == 0 {
        return;
    }
    let img = Rgba::filled(w.ceil() as u32, h.ceil() as u32, c);
    let place = Place { cx, cy, sx: 1.0, sy: 1.0, rotation };
    draw(canvas, &img, place, &DrawParams { opacity, radius, ..Default::default() });
}

struct Laid {
    text: String,
    start: Micros,
    end: Micros,
    size: f32,
    /// Ink box in layout coordinates (top-left origin).
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    index: usize,
}

/// How long the mark stays on a word after it is spoken when the next word is not yet due.
const HOLD_US: Micros = 600_000;

/// Keeps the spoken-word mark on a word through short gaps until the next word starts (and briefly after the last),
/// so the mark does not flicker off between words.
fn hold_marks(words: &mut [TimedWord]) {
    for i in 0..words.len() {
        let held = words[i].end + HOLD_US;
        let until = match words.get(i + 1) {
            Some(next) => held.min(next.start),
            None => held,
        };
        words[i].end = words[i].end.max(until);
    }
}

/// The group (for stacked styles) shown at `rel_us`: words split into runs of `n`, breaking after punctuation.
fn group_at(words: &[TimedWord], n: usize, rel_us: Micros) -> std::ops::Range<usize> {
    let mut groups: Vec<std::ops::Range<usize>> = Vec::new();
    let mut start = 0;
    for (i, w) in words.iter().enumerate() {
        let len = i + 1 - start;
        let stop = w.text.ends_with(['.', ',', '?', '!', ';', ':']);
        if len >= n.max(1) || stop {
            groups.push(start..i + 1);
            start = i + 1;
        }
    }
    if start < words.len() {
        groups.push(start..words.len());
    }
    // The last group whose first word has started.
    groups.iter().rev().find(|g| words[g.start].start <= rel_us).or(groups.first()).cloned().unwrap_or(0..0)
}

/// Draws an item's caption onto the canvas. `fallback` is the untimed text when the item has no timed segment.
pub fn draw_caption(
    canvas: &mut Rgba,
    cache: &mut CaptionCache,
    fields: &Map<String, Value>,
    fallback: &[String],
    rel_us: Micros,
    at: CaptionPlacement,
) {
    let p = resolve(fields);
    let (fw, fh) = (canvas.w as f32, canvas.h as f32);
    let orient = if fw > fh { p.landscape_scale } else { 1.0 };
    let size = p.size * at.k * orient;
    if size < 1.0 {
        return;
    }
    let timed = timed_words(fields, rel_us);
    let animated = timed.is_some();
    let mut all: Vec<TimedWord> = timed.unwrap_or_else(|| {
        fallback.iter().map(|w| TimedWord { text: w.clone(), start: Micros::MIN, end: Micros::MIN }).collect()
    });
    if all.is_empty() {
        return;
    }
    if animated {
        hold_marks(&mut all);
    }
    let range = if p.anim == Anim::Stacked && animated { group_at(&all, p.group, rel_us) } else { 0..all.len() };
    let words = &all[range.clone()];
    let stacked = p.anim == Anim::Stacked;

    let style = |sz: f32, c: [u8; 4]| TextStyle {
        size: sz,
        color: c,
        bold: p.weight >= 600,
        align: TextAlign::Center,
        line_height: p.line_height,
        letter_spacing: p.letter_spacing * sz,
        stroke: p.stroke.map(|(c, w)| (c, (w * sz).max(1.0))),
        shadow: p.shadow.map(|(c, x, y)| (c, x * sz, y * sz)),
        background: None,
        max_width: 1e9,
        highlight: None,
        font: None,
    };
    let pad_of = |st: &TextStyle| {
        let sw = st.stroke.map(|s| s.1).unwrap_or(0.0);
        let (sx, sy) = st.shadow.map(|s| (s.1, s.2)).unwrap_or((0.0, 0.0));
        sw + sx.abs().max(sy.abs()) + 2.0
    };

    // Layout.
    let longest = words.iter().map(|w| w.text.chars().count()).max().unwrap_or(0);
    let mut laid: Vec<Laid> = Vec::new();
    let max_w = fw * p.max_width / at.scale.max(0.01) as f32;
    let mut lines: Vec<(f32, f32)> = Vec::new(); // (width, height)
    let (mut x, mut y, mut line_h) = (0.0f32, 0.0f32, 0.0f32);
    for (i, w) in words.iter().enumerate() {
        let text = cased(&w.text, p.case);
        let sz =
            if stacked && p.emphasis > 1.0 && w.text.chars().count() == longest { size * p.emphasis } else { size };
        let st = style(sz, p.color);
        let img = cache.word(&text, &st);
        let pad = pad_of(&st);
        let (ww, wh) = ((img.w as f32 - 2.0 * pad).max(1.0), (img.h as f32 - 2.0 * pad).max(1.0));
        let space = sz * 0.3 + p.letter_spacing * sz;
        let new_line = !laid.is_empty() && (stacked || x + space + ww > max_w);
        if new_line {
            lines.push((x, line_h));
            y += line_h;
            x = 0.0;
            line_h = 0.0;
        }
        let wx = if x > 0.0 { x + space } else { 0.0 };
        laid.push(Laid { text, start: w.start, end: w.end, size: sz, x: wx, y, w: ww, h: wh, index: range.start + i });
        x = wx + ww;
        line_h = line_h.max(wh);
    }
    lines.push((x, line_h));
    let block_w = lines.iter().map(|l| l.0).fold(0.0, f32::max);
    let block_h: f32 = lines.iter().map(|l| l.1).sum();
    // Line index of each word, to align lines.
    let mut line_of = Vec::with_capacity(laid.len());
    {
        let mut li = 0;
        let mut ly = 0.0;
        for l in &laid {
            while l.y > ly + 0.01 && li + 1 < lines.len() {
                ly += lines[li].1;
                li += 1;
            }
            line_of.push(li);
        }
    }

    // Placement of the block's centre.
    let s = at.scale as f32;
    let (bw, bh) = (block_w * s, block_h * s);
    let cx = fw / 2.0 + at.dx as f32;
    let anchor = match p.position {
        Position::Top => fh * 0.10 + bh / 2.0,
        Position::Center => fh * 0.5,
        Position::Bottom => fh * 0.88 - bh / 2.0,
    };
    let cy = anchor + p.offset_y * at.k + at.dy as f32;
    let (left, top) = (cx - bw / 2.0, cy - bh / 2.0);

    // Whole-caption panel.
    if let Some(panel) = p.panel.filter(|pn| !pn.per_word) {
        // The panel grows with the words shown so far (words that appear as spoken keep their layout slots).
        let appears = animated && matches!(p.anim, Anim::Reveal | Anim::Stacked);
        let shown = laid.iter().zip(&line_of).filter(|(l, _)| !appears || rel_us >= l.start).map(|(_, li)| *li).max();
        let Some(last_line) = shown else { return };
        let vis_w = lines[..=last_line].iter().map(|l| l.0).fold(0.0, f32::max) * s;
        let vis_h = lines[..=last_line].iter().map(|l| l.1).sum::<f32>() * s;
        let (px, py) = (size * 0.4 * s, size * 0.2 * s);
        let (w, h) = (vis_w + 2.0 * px, vis_h + 2.0 * py);
        let cx = match p.align {
            Align::Center => cx,
            Align::Left => left + vis_w / 2.0,
        };
        let cy = top + vis_h / 2.0;
        if let Some((c, dx, dy)) = panel.shadow {
            let (ox, oy) = (dx * size * s, dy * size * s);
            rect(
                canvas,
                ((cx + ox) as f64, (cy + oy) as f64, w as f64, h as f64),
                panel.radius * size,
                0.0,
                c,
                at.opacity,
            );
        }
        rect(canvas, (cx as f64, cy as f64, w as f64, h as f64), panel.radius * size, 0.0, panel.fill, at.opacity);
    }

    for (n, l) in laid.iter().enumerate() {
        let line_w = lines[line_of[n]].0;
        let lx = match p.align {
            Align::Center => (block_w - line_w) / 2.0,
            Align::Left => 0.0,
        };
        let line_h = lines[line_of[n]].1;
        let mut wx = left + (lx + l.x + l.w / 2.0) * s;
        let mut wy = top + (l.y + line_h / 2.0) * s;

        let spoken = !animated || rel_us >= l.start;
        let active = animated && rel_us >= l.start && rel_us < l.end;
        let p_in =
            if animated { ((rel_us - l.start) as f32 / (p.reveal_ms as f32 * 1000.0)).clamp(0.0, 1.0) } else { 1.0 };
        let mut alpha = 1.0f32;
        let mut scale = 1.0f32;
        match p.anim {
            Anim::Reveal => {
                alpha = if spoken { ease_out(p_in) } else { 0.0 };
                wy += (1.0 - ease_out(p_in)) * l.size * 0.3 * s;
            }
            Anim::Stacked => {
                alpha = if spoken { ease_out(p_in * 1.5) } else { 0.0 };
                scale = if spoken { 0.82 + 0.18 * back_out(p_in) } else { 1.0 };
            }
            Anim::WordPop if active => scale = 1.0 + 0.16 * back_out(p_in),
            Anim::Pulse if active => scale = 1.0 + 0.2 * (1.0 - ease_out(p_in)),
            _ => {}
        }
        if animated && !active && p.dim < 1.0 {
            alpha *= p.dim;
        }
        if alpha <= 0.0 {
            continue;
        }
        let opacity = alpha * at.opacity;
        let wscale = (s * scale) as f64;
        // Cut-out styles: each word on its own tilted paper box.
        let mut rotation = 0.0;
        let mut text_color = p.color;
        if let Some(panel) = p.panel.filter(|pn| pn.per_word) {
            rotation = if l.index % 2 == 0 { -2.5 } else { 2.0 };
            let fill = if active {
                p.mark.color().unwrap_or(panel.fill)
            } else if l.index % 3 == 1 {
                panel.alt
            } else {
                panel.fill
            };
            let (bw, bh) = ((l.w + l.size * 0.36) as f64 * wscale, (l.h * 0.92) as f64 * wscale);
            if let Some((c, dx, dy)) = panel.shadow {
                let (ox, oy) = (dx * l.size * s, dy * l.size * s);
                rect(canvas, ((wx + ox) as f64, (wy + oy) as f64, bw, bh), panel.radius * l.size, rotation, c, opacity);
            }
            rect(canvas, (wx as f64, wy as f64, bw, bh), panel.radius * l.size, rotation, fill, opacity);
            if active && let Mark::Box(_, t) = p.mark {
                text_color = t;
            }
        } else if active {
            match p.mark {
                Mark::None => {}
                Mark::Color(c) => text_color = c,
                Mark::Box(c, t) => {
                    let (bw, bh) = ((l.w + l.size * 0.4) as f64 * wscale, (l.h * 0.95) as f64 * wscale);
                    rect(canvas, (wx as f64, wy as f64, bw, bh), l.size * 0.18, 0.0, c, opacity);
                    text_color = t;
                }
                Mark::Ring(c) => {
                    let (rw, rh) = (((l.w + l.size * 0.7) * s) as u32, ((l.h + l.size * 0.25) * s) as u32);
                    let r = ring(rw.max(4), rh.max(4), (l.size * 0.07 * s).max(2.0), c);
                    let place =
                        Place { cx: wx as f64, cy: wy as f64, sx: scale as f64, sy: scale as f64, rotation: -4.0 };
                    draw(
                        canvas,
                        &r,
                        place,
                        &DrawParams { opacity: opacity * ease_out(p_in * 2.0), ..Default::default() },
                    );
                }
                Mark::Underline(c) => {
                    let bar_h = (l.size * 0.07 * s).max(2.0) as f64;
                    let uy = wy as f64 + (l.h * 0.42 * s) as f64;
                    let bw = (l.w * s) as f64 * ease_out(p_in * 1.5) as f64;
                    rect(canvas, (wx as f64, uy, bw, bar_h), 0.0, 0.0, c, opacity);
                }
            }
        }
        let mut st = style(l.size, text_color);
        if active && matches!(p.mark, Mark::Box(..)) {
            // Text on a box needs no outline or shadow.
            st.shadow = None;
            st.stroke = None;
        }
        let img = cache.word(&l.text, &st);
        // The rasteriser pads the ink; keep the ink centred on its slot.
        if rotation == 0.0 {
            wx = wx.round();
            wy = wy.round();
        }
        let place = Place { cx: wx as f64, cy: wy as f64, sx: wscale, sy: wscale, rotation };
        draw(canvas, &img, place, &DrawParams { opacity, ..Default::default() });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::captions_presets::PRESETS;
    use serde_json::json;

    fn fields(preset: &str) -> Map<String, Value> {
        json!({
            "captionPresetId": preset,
            "captionSegments": [{"id": "s", "text": "make every video easy", "startTime": 0, "endTime": 2,
                "wordTimings": [
                    {"word": "make", "start": 0.0, "end": 0.4}, {"word": "every", "start": 0.4, "end": 0.9},
                    {"word": "video", "start": 0.9, "end": 1.5}, {"word": "easy", "start": 1.5, "end": 2.0}]}]
        })
        .as_object()
        .unwrap()
        .clone()
    }

    fn at() -> CaptionPlacement {
        CaptionPlacement { k: 0.5, dx: 0.0, dy: 0.0, scale: 1.0, opacity: 1.0 }
    }

    fn count(img: &Rgba, f: impl Fn([u8; 4]) -> bool) -> usize {
        img.data.chunks_exact(4).filter(|p| f([p[0], p[1], p[2], p[3]])).count()
    }

    #[test]
    fn every_preset_draws_ink() {
        assert_eq!(PRESETS.len(), 21);
        let mut cache = CaptionCache::default();
        for p in &PRESETS {
            let mut c = Rgba::filled(540, 960, [40, 40, 40, 255]);
            draw_caption(&mut c, &mut cache, &fields(p.id), &[], 1_000_000, at());
            let changed = count(&c, |px| px != [40, 40, 40, 255]);
            assert!(changed > 400, "{}: {changed} px", p.id);
        }
    }

    #[test]
    fn reveal_hides_words_not_yet_spoken() {
        let mut cache = CaptionCache::default();
        let mut early = Rgba::filled(540, 960, [0, 0, 0, 255]);
        draw_caption(&mut early, &mut cache, &fields("rapid-reveal"), &[], 300_000, at());
        let mut late = Rgba::filled(540, 960, [0, 0, 0, 255]);
        draw_caption(&mut late, &mut cache, &fields("rapid-reveal"), &[], 1_900_000, at());
        let ink = |c: &Rgba| count(c, |p| p[0] > 128);
        assert!(ink(&late) > ink(&early) * 2, "{} vs {}", ink(&late), ink(&early));
    }

    #[test]
    fn stacked_shows_the_current_group() {
        let words: Vec<TimedWord> = ["one", "two", "three.", "four", "five"]
            .iter()
            .enumerate()
            .map(|(i, w)| TimedWord { text: w.to_string(), start: i as i64 * 100, end: i as i64 * 100 + 100 })
            .collect();
        assert_eq!(group_at(&words, 2, 50), 0..2);
        assert_eq!(group_at(&words, 2, 250), 2..3);
        assert_eq!(group_at(&words, 2, 450), 3..5);
    }

    #[test]
    fn the_mark_holds_through_short_gaps() {
        let t = |s: i64, e: i64| TimedWord { text: "w".into(), start: s, end: e };
        let mut words = vec![t(0, 100_000), t(400_000, 500_000), t(2_000_000, 2_100_000)];
        hold_marks(&mut words);
        assert_eq!(words[0].end, 400_000, "until the next word");
        assert_eq!(words[1].end, 1_100_000, "a long pause: held briefly");
        assert_eq!(words[2].end, 2_700_000);
    }

    #[test]
    fn overrides_and_untimed_segments() {
        let mut f = fields("bold");
        f.insert("captionHighlightColor".into(), json!("#00ff00"));
        f.insert("captionPosition".into(), json!("top"));
        let p = resolve(&f);
        assert_eq!(p.mark, Mark::Color([0, 255, 0, 255]));
        assert_eq!(p.position, Position::Top);
        assert_eq!(item_preset(&Map::new()).id, DEFAULT_PRESET);
        let untimed = json!({"captionSegments": [{"text": "aa bb", "startTime": 1, "endTime": 2}]});
        let w = timed_words(untimed.as_object().unwrap(), 1_200_000).unwrap();
        assert_eq!(w.len(), 2);
        assert_eq!((w[0].start, w[1].end), (1_000_000, 2_000_000));
    }
}
