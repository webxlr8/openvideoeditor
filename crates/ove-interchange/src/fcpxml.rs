//! FCPXML 1.11 (Final Cut Pro, DaVinci Resolve). V1 is the primary storyline (spine) with gaps; higher video tracks
//! and audio tracks are connected clips on lanes (1, 2, … above; -1, -2, … below), attached to the spine element that
//! covers their start. Titles use the Basic Title effect. Speed changes are written as `timeMap` retimes.

use std::collections::BTreeMap;

use crate::model::{Clip, ClipKind, Rate, Timeline, Track, distinct_media, esc, file_url, url_path};
use crate::{Error, Result};

const BASIC_TITLE: &str = ".../Titles.localized/Bumper:Opener.localized/Basic Title.localized/Basic Title.moti";

/// Rational time string for a frame count: `N*den/num s`, reduced.
pub fn t(frames: i64, rate: Rate) -> String {
    if frames == 0 {
        return "0s".into();
    }
    let (mut n, mut d) = (frames * rate.den as i64, rate.num as i64);
    let g = gcd(n.abs(), d);
    n /= g;
    d /= g;
    if d == 1 { format!("{n}s") } else { format!("{n}/{d}s") }
}

fn gcd(a: i64, b: i64) -> i64 {
    if b == 0 { a.max(1) } else { gcd(b, a % b) }
}

/// Parses `1001/30000s`, `5s`, `0s` to seconds.
pub fn parse_t(s: &str) -> Option<f64> {
    let s = s.trim().strip_suffix('s')?;
    match s.split_once('/') {
        Some((a, b)) => Some(a.parse::<f64>().ok()? / b.parse::<f64>().ok()?),
        None => s.parse().ok(),
    }
}

struct Writer<'a> {
    rate: Rate,
    assets: BTreeMap<String, String>, // path → asset id
    out: &'a mut String,
}

impl Writer<'_> {
    fn clip_xml(&mut self, c: &Clip, offset: i64, lane: Option<i32>, children: &str) {
        let lane_attr = lane.map(|l| format!(" lane=\"{l}\"")).unwrap_or_default();
        let r = self.rate;
        match (&c.kind, &c.path) {
            (ClipKind::Title, _) => {
                let text = esc(c.text.as_deref().unwrap_or(&c.name));
                self.out.push_str(&format!(
                    "<title ref=\"rT\"{lane_attr} name=\"{}\" offset=\"{}\" start=\"3600s\" duration=\"{}\"><text><text-style ref=\"ts{}\">{text}</text-style></text><text-style-def id=\"ts{}\"><text-style font=\"Helvetica\" fontSize=\"72\" fontColor=\"1 1 1 1\" alignment=\"center\"/></text-style-def>{children}</title>",
                    esc(&c.name),
                    t(offset, r),
                    t(c.duration(), r),
                    c.start,
                    c.start
                ));
            }
            (_, Some(p)) => {
                let id = self.assets.get(&p.display().to_string()).cloned().unwrap_or_default();
                let retime = if (c.speed - 1.0).abs() > 1e-6 {
                    format!(
                        "<timeMap><timept time=\"0s\" value=\"{}\" interp=\"linear\"/><timept time=\"{}\" value=\"{}\" interp=\"linear\"/></timeMap>",
                        t(c.source_in, r),
                        t(c.duration(), r),
                        t(c.source_out(), r)
                    )
                } else {
                    String::new()
                };
                let audio_role = if c.kind == ClipKind::Audio { " audioRole=\"dialogue\"" } else { "" };
                self.out.push_str(&format!(
                    "<asset-clip ref=\"{id}\"{lane_attr} name=\"{}\" offset=\"{}\" start=\"{}\" duration=\"{}\"{audio_role}>{retime}{children}</asset-clip>",
                    esc(&c.name),
                    t(offset, r),
                    t(c.source_in, r),
                    t(c.duration(), r)
                ));
            }
            _ => {}
        }
    }
}

pub fn write(tl: &Timeline) -> String {
    let r = tl.rate;
    let mut res = String::new();
    res.push_str(&format!(
        "<format id=\"r1\" name=\"FFVideoFormat{}p{}\" frameDuration=\"{}\" width=\"{}\" height=\"{}\"/>",
        tl.height,
        if r.ntsc() { format!("{:.2}", r.fps()).replace('.', "") } else { r.timebase().to_string() },
        t(1, r),
        tl.width,
        tl.height
    ));
    res.push_str(&format!("<effect id=\"rT\" name=\"Basic Title\" uid=\"{BASIC_TITLE}\"/>"));
    let mut assets = BTreeMap::new();
    for (n, (key, c)) in distinct_media(tl).into_iter().enumerate() {
        let id = format!("a{}", n + 1);
        let path = c.path.as_ref().unwrap();
        res.push_str(&format!(
            "<asset id=\"{id}\" name=\"{}\" start=\"0s\" duration=\"{}\" hasVideo=\"{}\" hasAudio=\"{}\" format=\"r1\"><media-rep kind=\"original-media\" src=\"{}\"/></asset>",
            esc(&c.stem()),
            t(c.media_duration.unwrap_or(c.source_out()).max(1), r),
            u8::from(c.has_video),
            u8::from(c.has_audio || c.kind == ClipKind::Audio),
            esc(&file_url(path))
        ));
        assets.insert(key, id);
    }

    // Spine: V1 with gaps; every element knows its [offset, end) on the timeline.
    let empty = Track { name: String::new(), clips: vec![] };
    let v1 = tl.video.first().unwrap_or(&empty);
    let total = tl.duration().max(1);
    let mut spine: Vec<(i64, i64, Option<&Clip>)> = Vec::new();
    let mut cursor = 0;
    for c in &v1.clips {
        if c.start > cursor {
            spine.push((cursor, c.start, None));
        }
        spine.push((c.start, c.end, Some(c)));
        cursor = c.end;
    }
    if cursor < total {
        spine.push((cursor, total, None));
    }
    // Connected clips per spine element.
    let mut connected: Vec<Vec<(i32, &Clip)>> = vec![Vec::new(); spine.len()];
    let lanes = tl
        .video
        .iter()
        .skip(1)
        .enumerate()
        .map(|(i, tr)| (i as i32 + 1, tr))
        .chain(tl.audio.iter().enumerate().map(|(i, tr)| (-(i as i32) - 1, tr)));
    for (lane, tr) in lanes {
        for c in &tr.clips {
            let host = spine
                .iter()
                .position(|(s, e, _)| *s <= c.start && c.start < *e)
                .unwrap_or(spine.len().saturating_sub(1));
            connected[host].push((lane, c));
        }
    }

    let mut body = String::new();
    {
        let mut w = Writer { rate: r, assets, out: &mut body };
        for (i, (s, e, clip)) in spine.iter().enumerate() {
            // Children are positioned in the parent's source time: parent start + (timeline time - parent offset).
            let parent_start = clip
                .map(|c| if c.kind == ClipKind::Title { r.timebase() as i64 * 3600 } else { c.source_in })
                .unwrap_or(0);
            let mut kids = String::new();
            {
                let mut cw = Writer { rate: r, assets: w.assets.clone(), out: &mut kids };
                for (lane, c) in &connected[i] {
                    cw.clip_xml(c, parent_start + (c.start - s), Some(*lane), "");
                }
            }
            match clip {
                Some(c) => w.clip_xml(c, *s, None, &kids),
                None => w.out.push_str(&format!(
                    "<gap name=\"Gap\" offset=\"{}\" start=\"0s\" duration=\"{}\">{kids}</gap>",
                    t(*s, r),
                    t(e - s, r)
                )),
            }
        }
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE fcpxml>\n<fcpxml version=\"1.11\"><resources>{res}</resources><library><event name=\"{name}\"><project name=\"{name}\"><sequence format=\"r1\" duration=\"{}\" tcStart=\"0s\" tcFormat=\"NDF\" audioLayout=\"stereo\" audioRate=\"48k\"><spine>{body}</spine></sequence></project></event></library></fcpxml>\n",
        t(total, r),
        name = esc(&tl.name)
    )
}

/// (name, source path, duration seconds, has video, has audio) of an `asset` resource.
type AssetInfo = (String, Option<std::path::PathBuf>, Option<f64>, bool, bool);

pub fn read(text: &str) -> Result<Timeline> {
    let doc = roxmltree::Document::parse_with_options(text, crate::model::xml_options())
        .map_err(|e| Error::Parse(format!("not valid XML: {e}")))?;
    let root = doc.root_element();
    if root.tag_name().name() != "fcpxml" {
        return Err(Error::Parse("the file is not FCPXML".into()));
    }
    let find = |name: &str| root.descendants().find(|n| n.has_tag_name(name));
    let format = find("format");
    let fd = format.and_then(|f| f.attribute("frameDuration")).and_then(parse_t).unwrap_or(1.0 / 30.0);
    let rate = Rate::from_fps(1.0 / fd);
    let (width, height) = (
        format.and_then(|f| f.attribute("width")).and_then(|w| w.parse().ok()).unwrap_or(1920),
        format.and_then(|f| f.attribute("height")).and_then(|h| h.parse().ok()).unwrap_or(1080),
    );
    let mut assets: BTreeMap<String, AssetInfo> = BTreeMap::new();
    for a in root.descendants().filter(|n| n.has_tag_name("asset")) {
        let src = a
            .attribute("src")
            .or_else(|| a.descendants().find(|n| n.has_tag_name("media-rep")).and_then(|m| m.attribute("src")));
        assets.insert(
            a.attribute("id").unwrap_or("").to_string(),
            (
                a.attribute("name").unwrap_or("").to_string(),
                src.map(url_path),
                a.attribute("duration").and_then(parse_t),
                a.attribute("hasVideo") != Some("0"),
                a.attribute("hasAudio") == Some("1"),
            ),
        );
    }
    let fr = |s: Option<&str>| s.and_then(parse_t).map(|x| (x * rate.fps()).round() as i64).unwrap_or(0);
    let spine = find("spine").ok_or_else(|| Error::Parse("FCPXML has no spine".into()))?;
    let name = find("project").and_then(|p| p.attribute("name")).unwrap_or("Imported FCPXML").to_string();
    let mut lanes: BTreeMap<i32, Track> = BTreeMap::new();
    let make = |n: roxmltree::Node, start: i64| -> Option<Clip> {
        let dur = fr(n.attribute("duration"));
        let tag = n.tag_name().name();
        let src_start = fr(n.attribute("start"));
        let retime = n.children().find(|c| c.has_tag_name("timeMap")).map(|tm| {
            let pts: Vec<(f64, f64)> = tm
                .children()
                .filter(|c| c.has_tag_name("timept"))
                .filter_map(|p| Some((parse_t(p.attribute("time")?)?, parse_t(p.attribute("value")?)?)))
                .collect();
            match pts.as_slice() {
                [a, .., b] if b.0 > a.0 => (b.1 - a.1) / (b.0 - a.0),
                _ => 1.0,
            }
        });
        match tag {
            "asset-clip" | "clip" | "video" | "audio" => {
                let r = n.attribute("ref").or_else(|| {
                    n.descendants().find(|c| c.attribute("ref").is_some()).and_then(|c| c.attribute("ref"))
                })?;
                let (aname, path, adur, hv, ha) = assets.get(r)?.clone();
                let is_audio = tag == "audio" || (!hv && ha);
                Some(Clip {
                    name: n.attribute("name").map(str::to_string).unwrap_or(aname),
                    id: None,
                    kind: if is_audio { ClipKind::Audio } else { ClipKind::Video },
                    path,
                    media_id: None,
                    start,
                    end: start + dur,
                    source_in: src_start,
                    speed: retime.unwrap_or(1.0),
                    media_duration: adur.map(|d| (d * rate.fps()).round() as i64),
                    has_audio: ha,
                    has_video: hv && !is_audio,
                    text: None,
                })
            }
            "title" => {
                let text: String = n
                    .descendants()
                    .filter(|c| c.has_tag_name("text-style") && c.parent().is_some_and(|p| p.has_tag_name("text")))
                    .filter_map(|c| c.text())
                    .collect();
                Some(Clip {
                    name: n.attribute("name").unwrap_or("Title").to_string(),
                    id: None,
                    kind: ClipKind::Title,
                    path: None,
                    media_id: None,
                    start,
                    end: start + dur,
                    source_in: 0,
                    speed: 1.0,
                    media_duration: None,
                    has_audio: false,
                    has_video: true,
                    text: Some(text),
                })
            }
            _ => None,
        }
    };
    for el in spine.children().filter(|n| n.is_element()) {
        let off = fr(el.attribute("offset"));
        let el_start = fr(el.attribute("start"));
        if let Some(c) = make(el, off) {
            lanes.entry(0).or_insert_with(|| Track { name: "V1".into(), clips: vec![] }).clips.push(c);
        }
        for k in el.children().filter(|n| n.is_element() && n.attribute("lane").is_some()) {
            let lane: i32 = k.attribute("lane").and_then(|l| l.parse().ok()).unwrap_or(1);
            let at = off + fr(k.attribute("offset")) - el_start;
            if let Some(c) = make(k, at) {
                lanes
                    .entry(lane)
                    .or_insert_with(|| Track {
                        name: if lane > 0 { format!("V{}", lane + 1) } else { format!("A{}", -lane) },
                        clips: vec![],
                    })
                    .clips
                    .push(c);
            }
        }
    }
    let mut video = vec![];
    let mut audio = vec![];
    for (lane, mut tr) in lanes {
        tr.clips.sort_by_key(|c| c.start);
        if lane >= 0 { video.push(tr) } else { audio.insert(0, tr) }
    }
    Ok(Timeline { name, rate, width, height, video, audio })
}
