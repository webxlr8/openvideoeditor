//! Model manifests: what a model is made of and where each file comes from.
//!
//! A manifest lists one or more files, each pinned by size and SHA-256 and fetched from an immutable location (a
//! commit or a versioned release asset, never a moving branch). The built-in manifests are compiled into the binary;
//! feature changes add theirs to [`catalog`].

use serde::{Deserialize, Serialize};

use crate::error::{ModelError, Result};

/// How a model is run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeKind {
    /// whisper.cpp ggml weights, run by the speech recogniser.
    WhisperCpp,
    /// An ONNX graph, run through the inference adapter.
    Onnx,
    /// A native inference runtime library, loaded at run time.
    Library,
}

/// One file of a model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelFile {
    /// File name inside the model's folder.
    pub name: String,
    /// Immutable download location.
    pub url: String,
    /// Lower-case hex SHA-256 of the whole file.
    pub sha256: String,
    pub bytes: u64,
}

/// A model: identity, purpose, licence and files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub id: String,
    pub version: String,
    /// What the model does, for the Storage list.
    pub purpose: String,
    /// Licence of the weights (not only of the code that runs them).
    pub licence: String,
    pub files: Vec<ModelFile>,
    pub runtime: RuntimeKind,
    /// Small models (10 MB or less) may run inside the editor process; others run in the model worker.
    pub in_process: bool,
}

/// Models up to this size may run inside the editor process.
pub const IN_PROCESS_MAX_BYTES: u64 = 10 * 1_000_000;

impl Manifest {
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.bytes).sum()
    }

    pub fn file(&self, name: &str) -> Option<&ModelFile> {
        self.files.iter().find(|f| f.name == name)
    }

    /// Checks the rules every manifest must follow: a plain id and version, at least one file, plain file names,
    /// sizes, well-formed hashes and immutable HTTPS locations.
    pub fn validate(&self) -> Result<()> {
        let bad = |reason: String| Err(ModelError::InvalidManifest { id: self.id.clone(), reason });
        if !plain_name(&self.id) {
            return bad(format!("the id {:?} must be letters, digits, '-', '_' or '.'", self.id));
        }
        if !plain_name(&self.version) {
            return bad(format!("the version {:?} must be letters, digits, '-', '_' or '.'", self.version));
        }
        if self.files.is_empty() {
            return bad("it lists no files".into());
        }
        if self.in_process && self.total_bytes() > IN_PROCESS_MAX_BYTES {
            return bad(format!("it is marked in-process but is larger than {IN_PROCESS_MAX_BYTES} bytes"));
        }
        for (i, f) in self.files.iter().enumerate() {
            if !plain_name(&f.name) || f.name.ends_with(".part") {
                return bad(format!("the file name {:?} is not a plain file name", f.name));
            }
            if self.files[..i].iter().any(|g| g.name == f.name) {
                return bad(format!("the file {} is listed twice", f.name));
            }
            if !is_sha256(&f.sha256) {
                return bad(format!("the file {} has no valid SHA-256 (64 lower-case hex digits)", f.name));
            }
            if f.bytes == 0 {
                return bad(format!("the file {} has no size", f.name));
            }
            if let Err(reason) = check_pinned_url(&f.url) {
                return bad(format!("the file {}: {reason}", f.name));
            }
        }
        Ok(())
    }
}

fn plain_name(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn is_sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn is_commit(segment: &str) -> bool {
    segment.len() == 40 && segment.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A download location is accepted only when it names an immutable revision: a 40-digit commit in the path, or a
/// versioned release asset (`/releases/download/<tag>/`). Moving names (`main`, `master`, `latest`, `HEAD`) are refused.
pub fn check_pinned_url(url: &str) -> std::result::Result<(), String> {
    let Some(rest) = url.strip_prefix("https://") else {
        return Err(format!("{url} is not an https:// location"));
    };
    let path = rest.split(['?', '#']).next().unwrap_or_default();
    let segments: Vec<&str> = path.split('/').collect();
    const MOVING: [&str; 5] = ["main", "master", "latest", "head", "trunk"];
    if segments.iter().skip(1).any(|s| MOVING.contains(&s.to_ascii_lowercase().as_str())) {
        return Err(format!("{url} points at a moving branch or the latest release, not a fixed revision"));
    }
    if segments.iter().any(|s| is_commit(s)) {
        return Ok(());
    }
    if let Some(i) = segments.windows(2).position(|w| w == ["releases", "download"])
        && segments.get(i + 2).is_some_and(|tag| !tag.is_empty())
    {
        return Ok(());
    }
    Err(format!("{url} does not name a commit or a versioned release, so its content could change"))
}

/// The whisper.cpp model repository at a fixed commit. The sizes and hashes below are the files at this commit.
pub const WHISPER_REPO: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1";

/// A pinned whisper.cpp model file.
#[derive(Debug, Clone, Copy)]
pub struct WhisperPin {
    /// Manifest id.
    pub id: &'static str,
    pub file: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
    pub purpose: &'static str,
}

/// The speech models (multilingual), smallest first.
pub const WHISPER: [WhisperPin; 3] = [
    WhisperPin {
        id: "whisper-tiny",
        file: "ggml-tiny.bin",
        bytes: 77_691_713,
        sha256: "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
        purpose: "Speech to text for captions (fast, lower accuracy)",
    },
    WhisperPin {
        id: "whisper-base",
        file: "ggml-base.bin",
        bytes: 147_951_465,
        sha256: "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
        purpose: "Speech to text for captions (balanced)",
    },
    WhisperPin {
        id: "whisper-small",
        file: "ggml-small.bin",
        bytes: 487_601_967,
        sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
        purpose: "Speech to text for captions (most accurate, slower)",
    },
];

impl WhisperPin {
    pub fn manifest(&self) -> Manifest {
        Manifest {
            id: self.id.into(),
            version: "ggml-1".into(),
            purpose: self.purpose.into(),
            licence: "MIT".into(),
            files: vec![ModelFile {
                name: self.file.into(),
                url: format!("{WHISPER_REPO}/{}", self.file),
                sha256: self.sha256.into(),
                bytes: self.bytes,
            }],
            runtime: RuntimeKind::WhisperCpp,
            in_process: false,
        }
    }
}

/// Every model this build knows.
pub fn catalog() -> Vec<Manifest> {
    WHISPER.iter().map(WhisperPin::manifest).collect()
}

/// The built-in manifest with this id.
pub fn find(id: &str) -> Option<Manifest> {
    catalog().into_iter().find(|m| m.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Manifest {
        Manifest {
            id: "demo".into(),
            version: "1".into(),
            purpose: "test".into(),
            licence: "MIT".into(),
            files: vec![ModelFile {
                name: "demo.onnx".into(),
                url: "https://example.org/org/repo/resolve/0123456789abcdef0123456789abcdef01234567/demo.onnx".into(),
                sha256: "a".repeat(64),
                bytes: 10,
            }],
            runtime: RuntimeKind::Onnx,
            in_process: true,
        }
    }

    #[test]
    fn built_in_manifests_are_valid_and_unique() {
        let all = catalog();
        for m in &all {
            m.validate().unwrap();
        }
        for (i, m) in all.iter().enumerate() {
            assert!(all[..i].iter().all(|n| n.id != m.id), "duplicate id {}", m.id);
        }
    }

    #[test]
    fn a_valid_manifest_passes() {
        sample().validate().unwrap();
    }

    #[test]
    fn a_moving_branch_url_is_refused() {
        for url in [
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin",
            "https://github.com/org/repo/raw/master/model.onnx",
            "https://github.com/org/repo/releases/latest/download/model.onnx",
            "https://example.org/files/model.onnx",
            "http://example.org/org/repo/resolve/0123456789abcdef0123456789abcdef01234567/demo.onnx",
        ] {
            let mut m = sample();
            m.files[0].url = url.into();
            let e = m.validate().unwrap_err();
            assert_eq!(e.code(), "model.manifest", "{url}");
        }
    }

    #[test]
    fn release_assets_and_commits_are_accepted() {
        for url in [
            "https://github.com/org/repo/releases/download/v1.2.3/model.onnx",
            "https://huggingface.co/org/repo/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/model.onnx",
            "https://raw.githubusercontent.com/org/repo/5359861c739e955e79d9a303bcbc70fb988958b1/m.onnx",
        ] {
            check_pinned_url(url).unwrap();
        }
    }

    #[test]
    fn a_missing_or_malformed_hash_is_refused() {
        for sha in ["", "abc", &"A".repeat(64), &"g".repeat(64)] {
            let mut m = sample();
            m.files[0].sha256 = sha.to_string();
            assert!(m.validate().unwrap_err().to_string().contains("SHA-256"), "{sha:?}");
        }
    }

    #[test]
    fn other_rules_are_enforced() {
        let mut m = sample();
        m.files.clear();
        assert!(m.validate().is_err());
        let mut m = sample();
        m.files[0].bytes = 0;
        assert!(m.validate().is_err());
        let mut m = sample();
        m.files[0].name = "../escape".into();
        assert!(m.validate().is_err());
        let mut m = sample();
        m.files.push(m.files[0].clone());
        assert!(m.validate().is_err());
        let mut m = sample();
        m.files[0].bytes = IN_PROCESS_MAX_BYTES + 1;
        assert!(m.validate().is_err());
        let mut m = sample();
        m.id = "a/b".into();
        assert!(m.validate().is_err());
    }
}
