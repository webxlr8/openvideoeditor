//! A text-only manifest update keeps existing media indexes (add-semantic-media-search, condition 2 of the switch to
//! the half-precision text encoder). Index files record the image encoder's SHA-256; the update changes only the text
//! encoder, so an index built before the update is still current after it, through the real model store. Cached text
//! query embeddings, keyed by the text encoder, are not reused.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code: a failure here is a failed test")]

use std::path::Path;

use ove_models::manifest::{SIGLIP2_ID, SIGLIP2_TEXT, SIGLIP2_TOKENIZER, SIGLIP2_VISION, siglip2_manifest};
use ove_models::{Manifest, Store};
use ove_vision::index::{DIM, IndexData, Lookup, index_path, lookup, normalise, write};
use ove_vision::model::{ModelFiles, installed, provenance_of};
use sha2::{Digest, Sha256};

fn sha(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}

/// The real manifest with stand-in file contents: (name, bytes) per file, in manifest order.
fn manifest(version: &str, files: &[(&str, &[u8])]) -> Manifest {
    let mut m = siglip2_manifest();
    m.version = version.into();
    for (f, (name, b)) in m.files.iter_mut().zip(files) {
        f.name = (*name).into();
        f.sha256 = sha(b);
        f.bytes = b.len() as u64;
    }
    m
}

/// Installs `m` offline from `folder`, where each file sits under its download name.
fn install(root: &Path, folder: &Path, m: &Manifest, files: &[(&str, &[u8])]) -> Store {
    std::fs::create_dir_all(folder).unwrap();
    for (name, b) in files {
        std::fs::write(folder.join(name), b).unwrap();
    }
    let store = Store::with_catalog(root, vec![m.clone()]);
    store.install_from_folder(SIGLIP2_ID, folder, &mut |_| {}).unwrap();
    store
}

#[test]
fn an_index_survives_the_text_encoder_update() {
    let d = tempfile::tempdir().unwrap();
    let (root, bundle) = (d.path().join("models"), d.path().join("p.ove"));
    let image: &[u8] = b"image encoder stand-in";
    let tok: &[u8] = b"tokenizer stand-in";
    let old_files = [
        (SIGLIP2_VISION, image),
        ("text_model_q4f16.onnx", b"4-bit text encoder".as_slice()),
        (SIGLIP2_TOKENIZER, tok),
    ];
    let new_files =
        [(SIGLIP2_VISION, image), (SIGLIP2_TEXT, b"half-precision text encoder".as_slice()), (SIGLIP2_TOKENIZER, tok)];

    // Before the update: install, then index a media file with the installed model's provenance.
    // The previous build named the 4-bit text file, so its model files are read with that name.
    let old_manifest = manifest("old", &old_files);
    let before = install(&root, &d.path().join("dl-old"), &old_manifest, &old_files);
    let old_model = ModelFiles {
        vision: before.file_path(SIGLIP2_ID, SIGLIP2_VISION).unwrap(),
        text: before.file_path(SIGLIP2_ID, "text_model_q4f16.onnx").unwrap(),
        tokenizer: before.file_path(SIGLIP2_ID, SIGLIP2_TOKENIZER).unwrap(),
        vision_sha256: old_manifest.files[0].sha256.clone(),
        text_sha256: old_manifest.files[1].sha256.clone(),
    };
    let mut row: Vec<f32> = (0..DIM).map(|i| (i % 7) as f32).collect();
    normalise(&mut row);
    let data = IndexData {
        provenance: provenance_of(&old_model),
        content_id: "c1".into(),
        duration: 10.0,
        embeddings: row,
        times: vec![5.0],
        row_shot: vec![0],
        shots: vec![(0.0, 10.0)],
        cuts: vec![],
        cut_scores: vec![],
        frames: 250,
    };
    write(&index_path(&bundle, "c1"), &data).unwrap();

    // The update: only the text encoder changes.
    let after = install(&root, &d.path().join("dl-new"), &manifest("new", &new_files), &new_files);
    let new_model = installed(&after).unwrap();
    assert_eq!(new_model.vision_sha256, old_model.vision_sha256);
    assert_ne!(new_model.text_sha256, old_model.text_sha256);
    assert!(new_model.text.ends_with(SIGLIP2_TEXT));
    match lookup(&bundle, "c1", Some(&provenance_of(&new_model))) {
        Lookup::Ready(i) => assert_eq!(i.rows(), 1),
        other => panic!("the index must stay current after a text-only update, got {other:?}"),
    }
    assert!(!root.join(SIGLIP2_ID).join("old").exists(), "the previous install's files are gone");
}
