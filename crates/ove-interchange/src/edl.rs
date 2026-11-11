//! CMX3600 EDL. One video track (V1, the base layer) plus the audio of V1 clips ("B" events) and A1 audio ("A"
//! events). Reels are `AX` with `* FROM CLIP NAME:` comments, the common convention for file-based media. Speed changes
//! are written as M2 lines. Record timecode starts at 01:00:00:00 unless asked otherwise.

use crate::model::{Clip, ClipKind, Rate, Timeline, Track, url_path};
use crate::{Error, Result};

pub const RECORD_START: i64 = 3600; // seconds → 01:00:00:00

pub fn timecode(frames: i64, rate: Rate) -> String {
    let tb = rate.timebase() as i64;
    let f = frames.max(0);
    format!("{:02}:{:02}:{:02}:{:02}", f / (tb * 3600), (f / (tb * 60)) % 60, (f / tb) % 60, f % tb)
}

pub fn parse_timecode(s: &str, rate: Rate) -> Option<i64> {
    let p: Vec<i64> = s.split([':', ';', '.']).map(|x| x.parse().ok()).collect::<Option<_>>()?;
    let [h, m, sec, f] = p[..] else { return None };
    let tb = rate.timebase() as i64;
    Some(((h * 60 + m) * 60 + sec) * tb + f)
}

fn event(n: usize, chan: &str, c: &Clip, rate: Rate, offset: i64, out: &mut String) {
    out.push_str(&format!(
        "{n:03}  AX       {chan:<5} C        {} {} {} {}\n",
        timecode(c.source_in, rate),
        timecode(c.source_out(), rate),
        timecode(c.start + offset, rate),
        timecode(c.end + offset, rate)
    ));
    if (c.speed - 1.0).abs() > 1e-6 {
        out.push_str(&format!("M2   AX       {:05.1}    {}\n", rate.fps() * c.speed, timecode(c.source_in, rate)));
    }
    out.push_str(&format!(
        "* FROM CLIP NAME: {}\n",
        c.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| c.name.clone())
    ));
    if let Some(p) = &c.path {
        out.push_str(&format!("* SOURCE FILE: {}\n", p.display()));
    }
    out.push('\n');
}

/// Writes the EDL.
pub fn write(t: &Timeline) -> String {
    let offset = RECORD_START * t.rate.timebase() as i64;
    let mut out = format!("TITLE: {}\nFCM: NON-DROP FRAME\n\n", t.name.replace('\n', " "));
    let mut n = 0;
    if let Some(v1) = t.video.first() {
        for c in v1.clips.iter().filter(|c| c.path.is_some() && c.kind != ClipKind::Title) {
            n += 1;
            event(n, if c.has_audio { "B" } else { "V" }, c, t.rate, offset, &mut out);
        }
    }
    if let Some(a1) = t.audio.first() {
        for c in a1.clips.iter().filter(|c| c.path.is_some()) {
            n += 1;
            event(n, "A2", c, t.rate, offset, &mut out);
        }
    }
    out
}

/// Reads an EDL into V1 / A tracks. Clips carry the `FROM CLIP NAME` and, when present, `SOURCE FILE`.
pub fn read(text: &str, rate: Rate) -> Result<Timeline> {
    let mut name = "Imported EDL".to_string();
    let mut video = Track { name: "V1".into(), clips: vec![] };
    let mut audio = Track { name: "A1".into(), clips: vec![] };
    let mut last: Option<(bool, bool)> = None; // (to video, to audio) of the previous event
    let mut speed_pending: Option<f64> = None;
    let mut min_rec = i64::MAX;
    for (ln, line) in text.lines().enumerate() {
        let l = line.trim_end();
        if let Some(t) = l.strip_prefix("TITLE:") {
            name = t.trim().to_string();
            continue;
        }
        if l.starts_with("M2") {
            let parts: Vec<&str> = l.split_whitespace().collect();
            if let Some(fps) = parts.get(2).and_then(|x| x.parse::<f64>().ok()) {
                speed_pending = Some(fps / rate.fps());
            }
            if let (Some((v, a)), Some(s)) = (last, speed_pending) {
                if v && let Some(c) = video.clips.last_mut() {
                    c.speed = s;
                }
                if a && !v
                    && let Some(c) = audio.clips.last_mut()
                {
                    c.speed = s;
                }
            }
            continue;
        }
        // Comments describe the event above them.
        let mut targets = |f: &mut dyn FnMut(&mut Clip)| {
            if let Some((v, a)) = last {
                if v && let Some(c) = video.clips.last_mut() {
                    f(c);
                }
                if a && let Some(c) = audio.clips.last_mut() {
                    f(c);
                }
            }
        };
        if let Some(rest) = l.strip_prefix("* FROM CLIP NAME:") {
            let n = rest.trim().to_string();
            targets(&mut |c| {
                if c.name.is_empty() {
                    c.name = n.clone();
                }
            });
            continue;
        }
        // `* SOURCE FILE: /path` (ours, Resolve) or `* FROM CLIP: file:///…` (OpenTimelineIO, others).
        let source = l.strip_prefix("* SOURCE FILE:").or_else(|| l.strip_prefix("* FROM CLIP:")).map(str::trim);
        if let Some(rest) =
            source.filter(|r| r.starts_with("file:") || r.starts_with('/') || r.chars().nth(1) == Some(':'))
        {
            let p = if rest.starts_with("file:") { url_path(rest) } else { std::path::PathBuf::from(rest) };
            targets(&mut |c| c.path = Some(p.clone()));
            continue;
        }
        let parts: Vec<&str> = l.split_whitespace().collect();
        if parts.len() >= 8 && parts[0].chars().all(|c| c.is_ascii_digit()) {
            let chan = parts[2];
            let tc = &parts[parts.len() - 4..];
            let parse = |s: &str| {
                parse_timecode(s, rate).ok_or_else(|| Error::Parse(format!("line {}: bad timecode {s}", ln + 1)))
            };
            let (si, so, ri, ro) = (parse(tc[0])?, parse(tc[1])?, parse(tc[2])?, parse(tc[3])?);
            let has_v = chan.starts_with('V') || chan == "B";
            let has_a = chan.starts_with('A') || chan == "B";
            min_rec = min_rec.min(ri);
            let clip = Clip {
                name: String::new(),
                id: None,
                kind: if has_v { ClipKind::Video } else { ClipKind::Audio },
                path: None,
                media_id: None,
                start: ri,
                end: ro,
                source_in: si,
                speed: if ro > ri { ((so - si) as f64 / (ro - ri) as f64).max(0.01) } else { 1.0 },
                media_duration: None,
                has_audio: has_a,
                has_video: has_v,
                text: None,
            };
            if has_v {
                video.clips.push(clip)
            } else {
                audio.clips.push(clip)
            }
            last = Some((has_v, has_a && !has_v));
            speed_pending = None;
        }
    }
    // Rebase record time so the first event starts at 0 when the EDL uses an hour offset.
    let base = if min_rec == i64::MAX {
        0
    } else {
        (min_rec / (rate.timebase() as i64 * 3600)) * rate.timebase() as i64 * 3600
    };
    for c in video.clips.iter_mut().chain(audio.clips.iter_mut()) {
        c.start -= base;
        c.end -= base;
    }
    let mut v = vec![];
    if !video.clips.is_empty() {
        v.push(video);
    }
    let mut a = vec![];
    if !audio.clips.is_empty() {
        a.push(audio);
    }
    Ok(Timeline { name, rate, width: 1920, height: 1080, video: v, audio: a })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timecodes() {
        let r = Rate::FPS30;
        assert_eq!(timecode(30 * 3661 + 5, r), "01:01:01:05");
        assert_eq!(parse_timecode("01:01:01:05", r), Some(30 * 3661 + 5));
    }
}
