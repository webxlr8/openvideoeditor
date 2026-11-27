//! Keyframes on effect parameters through the projection (task 4.2): `effects.<effect id>.<parameter>` tracks apply
//! when the effect has that numeric parameter, and apply refuses, naming the effect and the parameter, when not.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test code: a failure here is a failed test; clippy.toml only covers #[test] functions, not helpers"
)]

use std::path::Path;
use std::process::{Command, Output};

use serde_json::json;

fn ove(lib: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ove"))
        .arg("--library")
        .arg(lib)
        .args(args)
        .env_remove("OVE_LIBRARY")
        .output()
        .unwrap()
}

#[test]
fn effect_parameter_keyframes_apply_or_are_refused_by_name() {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("lib");
    let library = ove_store::Library::open(&lib, std::sync::Arc::new(ove_core::SystemClock)).unwrap();
    let mut p = library.create("Fx").unwrap();
    let ops: Vec<ove_core::Op> = serde_json::from_value(json!([
        {"op": "addTrack", "track": {"id": "main", "name": "Main", "order": 0, "type": "video"}},
        {"op": "addItem", "item": {"id": "a", "trackId": "main", "type": "video", "startTime": 0, "endTime": 2,
         "pending": true, "pendingDescription": "shot", "effects": [{"id": "b", "type": "zoom-blur", "intensity": 30}]}}
    ]))
    .unwrap();
    p.apply(ops, ove_core::Source::Gui, None).unwrap();
    p.flush_projection();
    let text = std::fs::read_to_string(p.bundle().join("timeline.tsx")).unwrap();
    p.close().unwrap();
    let with = |prop: &str| {
        let kf = format!(
            r#"pendingDescription="shot" keyframeTracks={{[{{property: "{prop}", keyframes: [{{time: 0, value: 0}}, {{time: 1, value: 20}}]}}]}}"#
        );
        let file = d.path().join(format!("{}.tsx", prop.replace('.', "_")));
        std::fs::write(&file, text.replacen(r#"pendingDescription="shot""#, &kf, 1)).unwrap();
        file
    };

    let bad = ove(&lib, &["source", "apply", "Fx", with("effects.b.radius").to_str().unwrap()]);
    assert_eq!(bad.status.code(), Some(1), "{}", String::from_utf8_lossy(&bad.stdout));
    let msg = format!("{}{}", String::from_utf8_lossy(&bad.stdout), String::from_utf8_lossy(&bad.stderr));
    assert!(msg.contains("zoom-blur") && msg.contains("radius"), "names the effect and the parameter: {msg}");

    let missing = ove(&lib, &["source", "apply", "Fx", with("effects.nope.intensity").to_str().unwrap()]);
    assert_eq!(missing.status.code(), Some(1));

    let good = ove(&lib, &["source", "apply", "Fx", with("effects.b.intensity").to_str().unwrap()]);
    assert!(good.status.success(), "{}", String::from_utf8_lossy(&good.stderr));
    let printed = String::from_utf8_lossy(&ove(&lib, &["source", "print", "Fx"]).stdout).to_string();
    assert!(printed.contains("effects.b.intensity"), "{printed}");
}
