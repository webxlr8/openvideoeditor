//! add-transcript-editing task 1.4: transcripts are keyed by the media's content identity, so a renamed (or moved)
//! file keeps its transcript and a changed file does not.
#![allow(clippy::unwrap_used, reason = "test code: a failure here is a failed test")]

use ove_transcribe::Transcript;
use ove_transcribe::cache::{lookup, store, transcript_key};

#[test]
fn a_renamed_file_keeps_its_transcript_and_a_changed_file_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = dir.path().join("P.ove");
    let media = dir.path().join("interview.wav");
    std::fs::write(&media, vec![7u8; 50_000]).unwrap();
    let t = Transcript { language: "en".into(), model: "base".into(), words: vec![], duration_us: 3_000_000 };
    let cid = ove_store::media::content_id(&media).unwrap();
    store(&bundle, &transcript_key(&cid, "base", None, false), &t);

    // Renamed: same content id, same transcript.
    let renamed = dir.path().join("renamed.wav");
    std::fs::rename(&media, &renamed).unwrap();
    let cid2 = ove_store::media::content_id(&renamed).unwrap();
    assert_eq!(cid, cid2);
    assert_eq!(lookup(&bundle, &renamed, Some(&cid2), "base", None, false), Some(t.clone()));

    // Changed: a new content id, no transcript.
    let mut bytes = std::fs::read(&renamed).unwrap();
    bytes[25_000] ^= 1;
    std::fs::write(&renamed, bytes).unwrap();
    let cid3 = ove_store::media::content_id(&renamed).unwrap();
    assert_ne!(cid, cid3);
    assert_eq!(lookup(&bundle, &renamed, Some(&cid3), "base", None, false), None);
    // Another model or language is another transcript.
    assert_eq!(lookup(&bundle, &renamed, Some(&cid2), "small", None, false), None);
}
