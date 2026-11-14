//! Final Cut Pro 7 XML (xmeml v5), the format Premiere Pro imports and exports. Tracks bottom-first; every clip
//! references a `file` element (full definition on first use, `<file id="…"/>` after).

use std::collections::BTreeMap;

use crate::model::{Clip, ClipKind, Rate, Timeline, Track, esc, file_url, url_path};
use crate::{Error, Result};

fn rate_xml(r: Rate) -> String {
    format!(
        "<rate><timebase>{}</timebase><ntsc>{}</ntsc></rate>",
        r.timebase(),
        if r.ntsc() { "TRUE" } else { "FALSE" }
    )
}

fn clipitem(c: &Clip, idx: usize, files: &mut BTreeMap<String, String>, r: Rate, audio: bool) -> String {
    let mut s = format!(
        "<clipitem id=\"clipitem-{idx}\"><name>{}</name><enabled>TRUE</enabled><duration>{}</duration>{}<start>{}</start><end>{}</end><in>{}</in><out>{}</out>",
        esc(&c.name),
        c.media_duration.unwrap_or(c.source_out()),
        rate_xml(r),
        c.start,
        c.end,
        c.source_in,
        c.source_out()
    );
    if let Some(p) = &c.path {
        let key = p.display().to_string();
        match files.get(&key) {
            Some(id) => s.push_str(&format!("<file id=\"{id}\"/>")),
            None => {
                let id = format!("file-{}", files.len() + 1);
                s.push_str(&format!(
                    "<file id=\"{id}\"><name>{}</name><pathurl>{}</pathurl>{}<duration>{}</duration><media>{}{}</media></file>",
                    esc(&p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
                    esc(&file_url(p)),
                    rate_xml(r),
                    c.media_duration.unwrap_or(c.source_out()),
                    if c.has_video { "<video><samplecharacteristics><width>1920</width><height>1080</height></samplecharacteristics></video>" } else { "" },
                    if c.has_audio || audio { "<audio><channelcount>2</channelcount></audio>" } else { "" }
                ));
                files.insert(key, id);
            }
        }
    }
    if (c.speed - 1.0).abs() > 1e-6 {
        s.push_str(&format!(
            "<filter><effect><name>Time Remap</name><effectid>timeremap</effectid><parameter><parameterid>speed</parameterid><value>{}</value></parameter></effect></filter>",
            c.speed * 100.0
        ));
    }
    if audio {
        s.push_str("<sourcetrack><mediatype>audio</mediatype><trackindex>1</trackindex></sourcetrack>");
    }
    s.push_str("</clipitem>");
    s
}

fn title_item(c: &Clip, idx: usize, r: Rate) -> String {
    format!(
        "<generatoritem id=\"generator-{idx}\"><name>{}</name><enabled>TRUE</enabled><duration>{}</duration>{}<start>{}</start><end>{}</end><in>0</in><out>{}</out><effect><name>Text</name><effectid>Text</effectid><effectcategory>Text</effectcategory><effecttype>generator</effecttype><mediatype>video</mediatype><parameter><parameterid>str</parameterid><name>Text</name><value>{}</value></parameter></effect></generatoritem>",
        esc(&c.name),
        c.duration(),
        rate_xml(r),
        c.start,
        c.end,
        c.duration(),
        esc(c.text.as_deref().unwrap_or(""))
    )
}

pub fn write(tl: &Timeline) -> String {
    let r = tl.rate;
    let mut files = BTreeMap::new();
    let mut idx = 0;
    let mut video = String::new();
    for tr in &tl.video {
        video.push_str("<track>");
        for c in &tr.clips {
            idx += 1;
            if c.kind == ClipKind::Title {
                video.push_str(&title_item(c, idx, r));
            } else if c.path.is_some() {
                video.push_str(&clipitem(c, idx, &mut files, r, false));
            }
        }
        video.push_str("</track>");
    }
    let mut audio = String::new();
    // Audio of V1 clips goes to A1 (linked by position), then the audio tracks.
    let v1_audio: Vec<Clip> = tl
        .video
        .first()
        .map(|t| t.clips.iter().filter(|c| c.has_audio && c.path.is_some()).cloned().collect())
        .unwrap_or_default();
    let mut audio_tracks: Vec<&[Clip]> = Vec::new();
    if !v1_audio.is_empty() {
        audio_tracks.push(&v1_audio);
    }
    audio_tracks.extend(tl.audio.iter().map(|t| t.clips.as_slice()));
    for clips in audio_tracks {
        audio.push_str("<track>");
        for c in clips {
            idx += 1;
            audio.push_str(&clipitem(c, idx, &mut files, r, true));
        }
        audio.push_str("</track>");
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE xmeml>\n<xmeml version=\"5\"><sequence id=\"sequence-1\"><name>{}</name><duration>{}</duration>{}<timecode>{}<string>00:00:00:00</string><frame>0</frame><displayformat>NDF</displayformat></timecode><media><video><format><samplecharacteristics>{}<width>{}</width><height>{}</height><pixelaspectratio>square</pixelaspectratio></samplecharacteristics></format>{video}</video><audio>{audio}</audio></media></sequence></xmeml>\n",
        esc(&tl.name),
        tl.duration(),
        rate_xml(r),
        rate_xml(r),
        rate_xml(r),
        tl.width,
        tl.height
    )
}

fn child_text<'a>(n: roxmltree::Node<'a, 'a>, name: &str) -> Option<&'a str> {
    n.children().find(|c| c.has_tag_name(name)).and_then(|c| c.text())
}

/// (path, duration frames, has video, has audio) of a `file` element.
type FileInfo = (Option<std::path::PathBuf>, Option<i64>, bool, bool);

pub fn read(text: &str) -> Result<Timeline> {
    let doc = roxmltree::Document::parse_with_options(text, crate::model::xml_options())
        .map_err(|e| Error::Parse(format!("not valid XML: {e}")))?;
    let root = doc.root_element();
    if root.tag_name().name() != "xmeml" {
        return Err(Error::Parse("the file is not Final Cut Pro 7 / Premiere XML".into()));
    }
    let seq =
        root.descendants().find(|n| n.has_tag_name("sequence")).ok_or_else(|| Error::Parse("no sequence".into()))?;
    let rate_node = seq.children().find(|n| n.has_tag_name("rate"));
    let tb: f64 = rate_node.and_then(|r| child_text(r, "timebase")).and_then(|t| t.parse().ok()).unwrap_or(30.0);
    let ntsc = rate_node.and_then(|r| child_text(r, "ntsc")).is_some_and(|t| t.eq_ignore_ascii_case("true"));
    let rate = if ntsc { Rate::from_fps(tb * 1000.0 / 1001.0) } else { Rate::from_fps(tb) };
    // File definitions by id (first full definition wins).
    let mut files: BTreeMap<String, FileInfo> = BTreeMap::new();
    for f in seq.descendants().filter(|n| n.has_tag_name("file")) {
        if let Some(id) = f.attribute("id")
            && child_text(f, "pathurl").is_some()
        {
            let media = f.children().find(|c| c.has_tag_name("media"));
            files.entry(id.to_string()).or_insert((
                child_text(f, "pathurl").map(url_path),
                child_text(f, "duration").and_then(|d| d.parse().ok()),
                media.is_some_and(|m| m.children().any(|c| c.has_tag_name("video"))),
                media.is_some_and(|m| m.children().any(|c| c.has_tag_name("audio"))),
            ));
        }
    }
    let num = |n: roxmltree::Node, k: &str| child_text(n, k).and_then(|v| v.trim().parse::<i64>().ok());
    let tracks = |kind: &str| -> Vec<Track> {
        // The sequence's own media/video|audio (not the media blocks inside file definitions).
        let Some(m) = seq
            .children()
            .find(|n| n.has_tag_name("media"))
            .and_then(|media| media.children().find(|n| n.has_tag_name(kind)))
        else {
            return vec![];
        };
        m.children()
            .filter(|n| n.has_tag_name("track"))
            .enumerate()
            .map(|(i, tr)| {
                let clips = tr
                    .children()
                    .filter(|n| n.has_tag_name("clipitem") || n.has_tag_name("generatoritem"))
                    .filter_map(|ci| {
                        let (start, end, inp, out) =
                            (num(ci, "start")?, num(ci, "end")?, num(ci, "in").unwrap_or(0), num(ci, "out"));
                        if start < 0 || end <= start {
                            return None;
                        }
                        if ci.has_tag_name("generatoritem") {
                            let text = ci
                                .descendants()
                                .find(|p| p.has_tag_name("parameter") && child_text(*p, "parameterid") == Some("str"))
                                .and_then(|p| child_text(p, "value"))
                                .unwrap_or("")
                                .to_string();
                            return Some(Clip {
                                name: child_text(ci, "name").unwrap_or("Title").into(),
                                id: None,
                                kind: ClipKind::Title,
                                path: None,
                                media_id: None,
                                start,
                                end,
                                source_in: 0,
                                speed: 1.0,
                                media_duration: None,
                                has_audio: false,
                                has_video: true,
                                text: Some(text),
                            });
                        }
                        let fid = ci.children().find(|c| c.has_tag_name("file")).and_then(|f| f.attribute("id"))?;
                        let (path, mdur, hv, ha) = files.get(fid).cloned().unwrap_or((None, None, true, true));
                        let speed =
                            out.map(|o| (o - inp) as f64 / (end - start) as f64).filter(|s| *s > 0.0).unwrap_or(1.0);
                        let is_audio = kind == "audio";
                        Some(Clip {
                            name: child_text(ci, "name").unwrap_or("").into(),
                            id: None,
                            kind: if is_audio { ClipKind::Audio } else { ClipKind::Video },
                            path,
                            media_id: None,
                            start,
                            end,
                            source_in: inp,
                            speed: (speed * 1000.0).round() / 1000.0,
                            media_duration: mdur,
                            has_audio: ha,
                            has_video: hv && !is_audio,
                            text: None,
                        })
                    })
                    .collect();
                Track { name: format!("{}{}", if kind == "audio" { "A" } else { "V" }, i + 1), clips }
            })
            .filter(|t: &Track| !t.clips.is_empty())
            .collect()
    };
    let video = tracks("video");
    let mut audio = tracks("audio");
    // A1 that only mirrors V1's linked audio is dropped (the video clips carry their audio).
    if let (Some(v1), Some(a1)) = (video.first(), audio.first())
        && a1.clips.len() == v1.clips.iter().filter(|c| c.has_audio).count()
        && a1.clips.iter().all(|a| v1.clips.iter().any(|v| v.start == a.start && v.path == a.path))
    {
        audio.remove(0);
    }
    Ok(Timeline {
        name: child_text(seq, "name").unwrap_or("Imported XML").to_string(),
        rate,
        width: 1920,
        height: 1080,
        video,
        audio,
    })
}
