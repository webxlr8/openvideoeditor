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
#[serde(rename_all = "camelCase")]
pub struct ModelFile {
    /// File name inside the model's folder.
    pub name: String,
    /// Immutable download location (of the archive, when the file is an archive member).
    pub url: String,
    /// Lower-case hex SHA-256 of the installed file.
    pub sha256: String,
    /// Size of the installed file.
    pub bytes: u64,
    /// Set when the file is one member of a zip archive (for example a library inside a Python wheel): the archive
    /// at `url` is downloaded and checked against its own pins, then this member is extracted and checked against
    /// `sha256` and `bytes`. Files of one model that share an archive share one download.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_member: Option<ArchiveMember>,
}

/// Where a file sits inside the archive it is extracted from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveMember {
    /// Path of the member inside the archive, with `/`.
    pub path: String,
    /// Lower-case hex SHA-256 of the whole archive.
    pub archive_sha256: String,
    /// Size of the whole archive.
    pub archive_bytes: u64,
}

impl ModelFile {
    /// A plain file, downloaded as it is.
    pub fn new(name: &str, url: &str, sha256: &str, bytes: u64) -> ModelFile {
        ModelFile { name: name.into(), url: url.into(), sha256: sha256.into(), bytes, archive_member: None }
    }

    /// The name of what is downloaded: the archive's file name (the last part of its URL) for an archive member,
    /// otherwise the file's own name.
    pub fn download_name(&self) -> &str {
        match &self.archive_member {
            Some(_) => archive_name(&self.url),
            None => &self.name,
        }
    }

    /// Size and SHA-256 of what is downloaded.
    pub fn download_pin(&self) -> (u64, &str) {
        match &self.archive_member {
            Some(a) => (a.archive_bytes, &a.archive_sha256),
            None => (self.bytes, &self.sha256),
        }
    }
}

/// The last path segment of a URL.
pub fn archive_name(url: &str) -> &str {
    let path = url.split(['?', '#']).next().unwrap_or_default();
    path.rsplit('/').next().unwrap_or_default()
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
    /// Size of the installed files.
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.bytes).sum()
    }

    /// Bytes to download: each file, or each archive once however many members come from it.
    pub fn download_bytes(&self) -> u64 {
        self.downloads().iter().map(|f| f.download_pin().0).sum()
    }

    /// The files to download, one per archive: the first file of each archive stands for it.
    pub fn downloads(&self) -> Vec<&ModelFile> {
        let mut out: Vec<&ModelFile> = Vec::new();
        for f in &self.files {
            if f.archive_member.is_none() || !out.iter().any(|g| g.archive_member.is_some() && g.url == f.url) {
                out.push(f);
            }
        }
        out
    }

    /// The host the files come from, for the first-download prompt (for example `files.pythonhosted.org`).
    pub fn host(&self) -> Option<&str> {
        let f = self.files.first()?;
        f.url.strip_prefix("https://")?.split('/').next()
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
            if let Some(a) = &f.archive_member {
                let archive = archive_name(&f.url);
                if !plain_name(archive) || archive.ends_with(".part") || self.files.iter().any(|g| g.name == archive) {
                    return bad(format!("the archive of {} has no file name of its own", f.name));
                }
                if a.path.is_empty() || a.path.split('/').any(|s| s.is_empty() || s == "." || s == "..") {
                    return bad(format!("the archive member {:?} of {} is not a plain relative path", a.path, f.name));
                }
                if !is_sha256(&a.archive_sha256) || a.archive_bytes == 0 {
                    return bad(format!("the archive of {} has no valid SHA-256 or size", f.name));
                }
                let pinned_differently = self.files[..i].iter().any(|g| {
                    g.url == f.url
                        && g.archive_member
                            .as_ref()
                            .is_none_or(|b| b.archive_sha256 != a.archive_sha256 || b.archive_bytes != a.archive_bytes)
                });
                if pinned_differently {
                    return bad(format!("the archive of {} is pinned differently by another file", f.name));
                }
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
    // The Python package index stores each file under its own content hash: `/packages/<2>/<2>/<60 hex>/<file>`.
    if segments.len() == 6
        && segments[0] == "files.pythonhosted.org"
        && segments[1] == "packages"
        && segments[4].len() == 60
        && segments[4].bytes().all(|b| b.is_ascii_hexdigit())
    {
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
            files: vec![ModelFile::new(self.file, &format!("{WHISPER_REPO}/{}", self.file), self.sha256, self.bytes)],
            runtime: RuntimeKind::WhisperCpp,
            in_process: false,
        }
    }
}

/// The id of the inference runtime (ONNX Runtime) in the model store.
pub const RUNTIME_ID: &str = "onnxruntime";
/// The pinned ONNX Runtime version. Its macOS library needs macOS 14 or later on Apple silicon (the wheel's platform
/// tag is `macosx_14_0_arm64`), the same minimum as the app.
pub const RUNTIME_VERSION: &str = "1.32.0";

/// A platform the runtime library is pinned for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimePlatform {
    WindowsX64,
    MacosArm64,
}

impl RuntimePlatform {
    /// The platform this build runs on, when the runtime is pinned for it.
    pub fn current() -> Option<RuntimePlatform> {
        if cfg!(all(windows, target_arch = "x86_64")) {
            Some(RuntimePlatform::WindowsX64)
        } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            Some(RuntimePlatform::MacosArm64)
        } else {
            None
        }
    }
}

/// Pins of one runtime wheel and the members taken from it.
struct RuntimePin {
    url: &'static str,
    wheel: (&'static str, u64),
    library: (&'static str, &'static str, u64),
    licence: (&'static str, u64),
    notices: (&'static str, u64),
}

const RUNTIME_WINDOWS_X64: RuntimePin = RuntimePin {
    url: "https://files.pythonhosted.org/packages/a9/3e/27039badc39a1b01628b6b40b1ff1bf4a886e6dfb2f311c483606b7c86f7/onnxruntime-1.32.0-cp312-cp312-win_amd64.whl",
    wheel: ("efe7456952c243a05a53d5b8732953625144642c2f3989094277f6217f0c0c40", 16_698_767),
    library: ("onnxruntime.dll", "443c0065fcdba17618ddef05050308f4b471893c5c7da0337d6957c9073ad8a7", 22_668_640),
    licence: ("c250d6278f0b47a6439fb7592b08b58a55eb9f535aa49a1db63211c3f982b674", 1_094),
    notices: ("bf8073fd6a112ac2a656bb60799652127e881689e51d37ad20df446cd5188b2d", 341_120),
};

const RUNTIME_MACOS_ARM64: RuntimePin = RuntimePin {
    url: "https://files.pythonhosted.org/packages/6f/a4/480cd367bc135dd9951c89d461cc6af6be9302cd2ce91e231f6cb0e7ee1b/onnxruntime-1.32.0-cp312-cp312-macosx_14_0_arm64.whl",
    wheel: ("d52510bbbcc880c13b826f7d75cdf33ac5cf0bb864d960000fc29ce73f0cb83e", 22_519_308),
    library: (
        "libonnxruntime.1.32.0.dylib",
        "fa585fabbea65b823b9a8f7e27d46aee3989d1be7e36b528203d3a1fd613aae7",
        35_272_344,
    ),
    licence: ("2f07c72751aed99790b8a4869cf2311df85a860b22ded05fa22803587a48922c", 1_073),
    notices: ("d8696258afcf9df7827a5e01d6b6d55ee9b461b4e82020f5f48242deb9cc2270", 334_795),
};

/// ONNX Runtime from the official Python wheels on the Python package index (file locations there are content
/// addressed, so they never change). The library, the wheel's licence and its third-party notices are extracted into
/// the store next to each other; the wheel itself is deleted after extraction.
pub fn runtime_manifest(platform: RuntimePlatform) -> Manifest {
    let pin = match platform {
        RuntimePlatform::WindowsX64 => RUNTIME_WINDOWS_X64,
        RuntimePlatform::MacosArm64 => RUNTIME_MACOS_ARM64,
    };
    let member = |name: &str, path: &str, (sha256, bytes): (&str, u64)| ModelFile {
        name: name.into(),
        url: pin.url.into(),
        sha256: sha256.into(),
        bytes,
        archive_member: Some(ArchiveMember {
            path: path.into(),
            archive_sha256: pin.wheel.0.into(),
            archive_bytes: pin.wheel.1,
        }),
    };
    let (lib, lib_sha, lib_bytes) = pin.library;
    Manifest {
        id: RUNTIME_ID.into(),
        version: RUNTIME_VERSION.into(),
        purpose: "Inference runtime for voice activity and other local models (ONNX Runtime)".into(),
        licence: "MIT".into(),
        files: vec![
            member(lib, &format!("onnxruntime/capi/{lib}"), (lib_sha, lib_bytes)),
            member("LICENSE", "onnxruntime/LICENSE", pin.licence),
            member("ThirdPartyNotices.txt", "onnxruntime/ThirdPartyNotices.txt", pin.notices),
        ],
        runtime: RuntimeKind::Library,
        in_process: false,
    }
}

/// The voice-activity model's id (add-transcript-editing: dead air and filler detection).
pub const VAD_ID: &str = "silero-vad";

/// Silero VAD (ONNX export from the Silero repository at a fixed commit), run through the shared inference runtime
/// in the editor process (2.2 MB). Chosen over the recognition library's built-in voice activity by measurement
/// (add-transcript-editing task 1.1, design.md R1: 37 times faster, no false speech in a music bed). Weights licence
/// MIT: the repository's `LICENSE` ("MIT License, Copyright (c) 2020-present Silero Team") and README ("Published
/// under permissive license (MIT)"); a README badge's alt text says "CC BY-NC 4.0" but the badge itself reads MIT
/// and links to that `LICENSE`.
pub fn vad_manifest() -> Manifest {
    Manifest {
        id: VAD_ID.into(),
        version: "6-446da2a".into(),
        purpose: "Voice activity, for removing dead air and finding fillers".into(),
        licence: "MIT".into(),
        files: vec![ModelFile::new(
            "silero_vad.onnx",
            "https://raw.githubusercontent.com/snakers4/silero-vad/446da2ad95fc90d67a3b12e12ea3f5410b9b3ec4/src/silero_vad/data/silero_vad.onnx",
            "f6f4407b41e05e277e7f5cfd9b9ae957b1dfe763c6076a934734c0b448e09b31",
            2_204_103,
        )],
        runtime: RuntimeKind::Onnx,
        in_process: true,
    }
}

/// Every model this build knows: the speech models, voice activity, and the inference runtime for this platform.
pub fn catalog() -> Vec<Manifest> {
    let mut all: Vec<Manifest> = WHISPER.iter().map(WhisperPin::manifest).collect();
    all.push(vad_manifest());
    all.extend(RuntimePlatform::current().map(runtime_manifest));
    all
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
            files: vec![ModelFile::new(
                "demo.onnx",
                "https://example.org/org/repo/resolve/0123456789abcdef0123456789abcdef01234567/demo.onnx",
                &"a".repeat(64),
                10,
            )],
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
    fn both_runtime_manifests_are_valid_and_download_one_wheel() {
        for p in [RuntimePlatform::WindowsX64, RuntimePlatform::MacosArm64] {
            let m = runtime_manifest(p);
            m.validate().unwrap();
            assert_eq!(m.downloads().len(), 1, "one wheel per platform");
            assert_eq!(m.host(), Some("files.pythonhosted.org"));
            assert!(m.download_bytes() < m.total_bytes());
            assert!(m.files.iter().all(|f| f.archive_member.is_some()));
        }
        assert_eq!(runtime_manifest(RuntimePlatform::WindowsX64).download_bytes(), 16_698_767);
        assert_eq!(runtime_manifest(RuntimePlatform::MacosArm64).download_bytes(), 22_519_308);
    }

    #[test]
    fn archive_members_are_checked() {
        let mut m = runtime_manifest(RuntimePlatform::WindowsX64);
        m.files[0].archive_member.as_mut().unwrap().path = "../onnxruntime.dll".into();
        assert!(m.validate().is_err());
        let mut m = runtime_manifest(RuntimePlatform::WindowsX64);
        m.files[1].archive_member.as_mut().unwrap().archive_sha256 = "b".repeat(64);
        assert!(m.validate().unwrap_err().to_string().contains("pinned differently"));
        let mut m = runtime_manifest(RuntimePlatform::WindowsX64);
        m.files[2].archive_member.as_mut().unwrap().archive_bytes = 0;
        assert!(m.validate().is_err());
    }

    #[test]
    fn content_addressed_package_index_files_are_accepted() {
        let hash = "27039badc39a1b01628b6b40b1ff1bf4a886e6dfb2f311c483606b7c86f7";
        check_pinned_url(&format!("https://files.pythonhosted.org/packages/a9/3e/{hash}/x.whl")).unwrap();
        assert!(check_pinned_url("https://files.pythonhosted.org/packages/a9/3e/x.whl").is_err());
        assert!(check_pinned_url(&format!("https://example.org/packages/a9/3e/{hash}/x.whl")).is_err());
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
