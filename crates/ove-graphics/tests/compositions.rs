//! Renders the built-in templates and sample compositions: golden frames, determinism, diagnostics and the
//! frame budget at 1080p. Set `OVE_UPDATE_GOLDEN=1` to rewrite the golden PNGs under `tests/golden/`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use ove_graphics::{CodeComposition, FrameTime, Limits, check, prepare, render, templates};
use serde_json::json;

const HD: (u32, u32) = (1920, 1080);

fn no_images(_: &str) -> Option<Arc<Vec<u8>>> {
    None
}

/// A 64×64 PNG: an orange disc on transparency, made here so the test needs no binary fixture.
fn logo_png() -> Arc<Vec<u8>> {
    let mut px = vec![0u8; 64 * 64 * 4];
    for y in 0..64 {
        for x in 0..64 {
            let (dx, dy) = (x as f32 - 31.5, y as f32 - 31.5);
            if dx * dx + dy * dy < 30.0 * 30.0 {
                px[(y * 64 + x) * 4..][..4].copy_from_slice(&[255, 140, 0, 255]);
            }
        }
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, 64, 64);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(&px).unwrap();
    }
    Arc::new(out)
}

fn images(id: &str) -> Option<Arc<Vec<u8>>> {
    (id == "media-logo").then(logo_png)
}

fn tpl(id: &str, props: serde_json::Value) -> CodeComposition {
    CodeComposition {
        template: Some(id.into()),
        props: props.as_object().cloned().unwrap_or_default(),
        ..CodeComposition::default()
    }
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

fn write_png(path: &PathBuf, w: u32, h: u32, rgba: &[u8]) {
    let f = std::fs::File::create(path).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(rgba).unwrap();
}

fn read_png(path: &PathBuf) -> Option<(u32, u32, Vec<u8>)> {
    let dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).ok()?));
    let mut r = dec.read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()?];
    let info = r.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    Some((info.width, info.height, buf))
}

/// Compares against the golden PNG: mean absolute channel difference must stay small (font rasterisation
/// differs by a few levels between platforms; a layout change moves far more).
fn golden(name: &str, w: u32, h: u32, rgba: &[u8]) {
    let path = golden_dir().join(format!("{name}.png"));
    if std::env::var_os("OVE_UPDATE_GOLDEN").is_some() || !path.exists() {
        std::fs::create_dir_all(golden_dir()).unwrap();
        write_png(&path, w, h, rgba);
        if std::env::var_os("OVE_UPDATE_GOLDEN").is_none() {
            panic!("golden {name} was missing and has been written; rerun");
        }
        return;
    }
    let (gw, gh, g) = read_png(&path).expect("golden decodes");
    assert_eq!((gw, gh), (w, h), "{name} size");
    let diff: u64 = g.iter().zip(rgba).map(|(a, b)| a.abs_diff(*b) as u64).sum();
    let mean = diff as f64 / g.len() as f64;
    assert!(mean < 1.5, "{name}: mean difference {mean:.3} from golden");
}

#[test]
fn every_template_checks_clean_and_matches_its_golden() {
    for t in templates::all() {
        let mut c = tpl(t.id, json!({}));
        if t.id == "logo-sting" {
            c.props.insert("logo".into(), json!("media-logo"));
        }
        let report = check(&c, HD, t.duration, &images);
        assert!(report.ok, "{}: {:?}", t.id, report.diagnostics);
        assert!(report.diagnostics.is_empty(), "{}: {:?}", t.id, report.diagnostics);
        let p = prepare(&c, HD).unwrap();
        // Golden at quarter size, two thirds of the way in (entrances done, exits not started).
        let r = render(
            &p,
            FrameTime { time: t.duration * 0.6, duration: t.duration, fps: 30.0 },
            0.25,
            &images,
            Limits::default(),
        )
        .unwrap();
        assert_eq!((r.frame.width, r.frame.height), (480, 270));
        let painted = r.frame.rgba.chunks(4).filter(|p| p[3] > 0).count();
        assert!(painted > 500, "{} paints something ({painted} px)", t.id);
        golden(&format!("template-{}", t.id), r.frame.width, r.frame.height, &r.frame.rgba);
    }
}

#[test]
fn frames_animate_and_are_deterministic() {
    let c = tpl("kinetic-text", json!({"text": "one two three four"}));
    let p = prepare(&c, HD).unwrap();
    let at = |time: f64| {
        render(&p, FrameTime { time, duration: 4.0, fps: 30.0 }, 0.25, &no_images, Limits::default())
            .unwrap()
            .frame
            .rgba
    };
    let (a, b) = (at(0.5), at(0.5));
    assert_eq!(a, b, "same time → identical pixels");
    assert_ne!(at(0.1), at(1.5), "different times differ");

    // Randomness and the clock are pinned, so even "random" compositions repeat exactly.
    let src = CodeComposition {
        source: Some(
            "export default function C({ time }) {
               const dots = Array.from({ length: 40 }, (_, i) => ({ x: Math.random() * 1800, y: random(i) * 1000 }));
               return <Fill style={{ backgroundColor: '#123' }}>{dots.map((d) => (
                 <div style={{ position: 'absolute', left: d.x, top: d.y, width: 40, height: 40, borderRadius: 20, backgroundColor: 'white', opacity: (Date.now() + time) % 1 }} />
               ))}</Fill>;
             }"
            .into(),
        ),
        ..CodeComposition::default()
    };
    let p = prepare(&src, HD).unwrap();
    let r = |_: ()| {
        render(&p, FrameTime { time: 0.5, duration: 1.0, fps: 30.0 }, 0.25, &no_images, Limits::default())
            .unwrap()
            .frame
            .rgba
    };
    assert_eq!(r(()), r(()));
}

#[test]
fn source_compositions_report_problems() {
    let bad = |src: &str| {
        let c = CodeComposition { source: Some(src.into()), ..CodeComposition::default() };
        check(&c, HD, 2.0, &no_images)
    };
    let r = bad("export default function C() { return <svg><circle /></svg>; }");
    assert!(r.ok && r.diagnostics.iter().any(|d| d.code == "tag"), "{:?}", r.diagnostics);
    let r = bad("export default function C() { return <div style={{ color: 'red', wobble: 3 }}>x</div>; }");
    assert!(r.diagnostics.iter().any(|d| d.code == "style" && d.message.contains("wobble")), "{:?}", r.diagnostics);
    let r = bad("export default function C({ time }) { if (time > 0.5) throw new Error('boom'); return null; }");
    assert!(
        !r.ok && r.diagnostics.iter().any(|d| d.code == "runtime" && d.message.contains("boom")),
        "{:?}",
        r.diagnostics
    );
    let r = bad("export default function C() { for (;;) {} }");
    assert!(!r.ok && r.diagnostics[0].code == "budget", "{:?}", r.diagnostics);
    let r = bad("export default function C() { console.log('hello', { a: 1 }); return <div>x</div>; }");
    assert!(r.ok && r.logs == ["log: hello {\"a\":1}"], "{:?}", r.logs);
    let r = bad("import x from './x'; export default () => null;");
    assert!(!r.ok && r.diagnostics[0].code == "import");
    let r = check(&tpl("nope", json!({})), HD, 1.0, &no_images);
    assert!(!r.ok && r.diagnostics[0].code == "template");
}

#[test]
fn fonts_resolve_by_family_and_weight() {
    let c = CodeComposition {
        source: Some(
            "export default function C({ props }) {
               return <Fill style={{ backgroundColor: 'black', alignItems: 'center', justifyContent: 'center' }}>
                 <div style={{ fontFamily: props.family, fontWeight: props.weight, fontSize: 200 }}>Wg</div>
               </Fill>;
             }"
            .into(),
        ),
        ..CodeComposition::default()
    };
    let ink = |family: &str, weight: u32| {
        let mut c = c.clone();
        c.props.insert("family".into(), json!(family));
        c.props.insert("weight".into(), json!(weight));
        let p = prepare(&c, HD).unwrap();
        let r =
            render(&p, FrameTime { time: 0.0, duration: 1.0, fps: 30.0 }, 0.25, &no_images, Limits::default()).unwrap();
        r.frame.rgba.chunks(4).filter(|p| p[0] > 128).count()
    };
    assert!(ink("Inter", 900) > ink("Inter", 200), "variable weight axis applies");
    assert_ne!(ink("Playfair Display", 400), ink("Bebas Neue", 400), "families differ");
}

/// Frame budget at 1080p (printed; run with `--nocapture`). The assertion is loose so debug CI machines pass;
/// the printed numbers are what the report quotes.
#[test]
fn frame_budget_at_1080p() {
    let mut worst: f64 = 0.0;
    for t in templates::all() {
        let mut c = tpl(t.id, json!({}));
        if t.id == "logo-sting" {
            c.props.insert("logo".into(), json!("media-logo"));
        }
        let p = prepare(&c, HD).unwrap();
        let mut script = Vec::new();
        let mut paint = Vec::new();
        let mut total = Vec::new();
        for i in 0..15 {
            let time = t.duration * i as f64 / 15.0;
            let s = Instant::now();
            let r = render(&p, FrameTime { time, duration: t.duration, fps: 30.0 }, 1.0, &images, Limits::default())
                .unwrap();
            total.push(s.elapsed().as_secs_f64() * 1e3);
            script.push(r.script_ms);
            paint.push(r.paint_ms);
        }
        let avg = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
        let max = total.iter().cloned().fold(0.0, f64::max);
        worst = worst.max(avg(&total));
        println!(
            "{:<14} 1920x1080  avg {:>6.2} ms (script {:>5.2}, layout+paint {:>6.2})  max {:>6.2} ms",
            t.id,
            avg(&total),
            avg(&script),
            avg(&paint),
            max
        );
    }
    assert!(worst < 500.0, "average frame {worst:.1} ms");
}
