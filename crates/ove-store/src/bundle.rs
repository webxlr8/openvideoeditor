//! Bundle folder names and layout (spec: Project bundle creation, Bundle folder names).

use std::path::{Path, PathBuf};

/// Bundle folder extension (working name until the product is named, change 16).
pub const BUNDLE_EXT: &str = "ove";

/// Sub-folders every bundle has.
pub const BUNDLE_DIRS: [&str; 5] = ["media", "cache", "proxies", ".revs", ".scratch"];

const MAX_NAME_CHARS: usize = 120;
const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2",
    "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Sanitises a project name for use as a folder name (without extension or collision suffix):
/// removes `\ / : * ? " < > |` and control characters, leading dots, trailing dots and spaces; at most 120
/// characters; "Untitled Project" when empty; Windows reserved device names get a suffix.
pub fn sanitise_name(name: &str) -> String {
    let mut s: String = name
        .chars()
        .filter(|c| !matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') && !c.is_control())
        .collect();
    s = s.trim_start_matches('.').to_string();
    s = s.chars().take(MAX_NAME_CHARS).collect();
    s = s.trim_end_matches(['.', ' ']).to_string();
    // Leading spaces are allowed by both file systems; leading dots would hide the folder.
    if s.trim().is_empty() {
        return "Untitled Project".into();
    }
    let stem = s.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
    if RESERVED.contains(&stem.as_str()) {
        s.push_str(" Project");
    }
    s
}

/// The bundle path for a name in a library: `<Name>.ove`, or `<Name> 2.ove`, `<Name> 3.ove`, … if taken.
/// Comparison is case-insensitive so names that collide on case-insensitive volumes also get a suffix.
pub fn unique_bundle_path(library: &Path, name: &str) -> PathBuf {
    let base = sanitise_name(name);
    let existing: Vec<String> = std::fs::read_dir(library)
        .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().to_lowercase()).collect())
        .unwrap_or_default();
    let taken = |candidate: &str| {
        let file = format!("{candidate}.{BUNDLE_EXT}");
        existing.contains(&file.to_lowercase()) || library.join(&file).exists()
    };
    if !taken(&base) {
        return library.join(format!("{base}.{BUNDLE_EXT}"));
    }
    let mut n = 2u64;
    loop {
        let candidate = format!("{base} {n}");
        if !taken(&candidate) {
            return library.join(format!("{candidate}.{BUNDLE_EXT}"));
        }
        n += 1;
    }
}

/// True when a path looks like a bundle folder.
pub fn is_bundle(path: &Path) -> bool {
    path.is_dir() && path.extension().is_some_and(|e| e.eq_ignore_ascii_case(BUNDLE_EXT))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitisation_table() {
        let cases = [
            ("Q3: \"Launch\" / Final?", "Q3 Launch  Final"),
            ("Promo", "Promo"),
            ("", "Untitled Project"),
            ("   ", "Untitled Project"),
            ("...hidden", "hidden"),
            ("trailing. . ", "trailing"),
            ("tab\tand\nnewline", "tabandnewline"),
            ("a<b>c|d*e", "abcde"),
            ("CON", "CON Project"),
            ("nul", "nul Project"),
            ("COM1", "COM1 Project"),
            ("LPT9.txt", "LPT9.txt Project"),
            ("CONSOLE", "CONSOLE"),
            ("Ünïcödé 🎬", "Ünïcödé 🎬"),
            ("////", "Untitled Project"),
        ];
        for (input, want) in cases {
            assert_eq!(sanitise_name(input), want, "{input:?}");
        }
        let long = "x".repeat(300);
        assert_eq!(sanitise_name(&long).chars().count(), 120);
        let long_trailing = format!("{}{}", "y".repeat(119), " .z");
        assert_eq!(sanitise_name(&long_trailing), "y".repeat(119));
    }

    #[test]
    fn collisions_get_numeric_suffix() {
        let dir = tempfile::tempdir().unwrap();
        let p1 = unique_bundle_path(dir.path(), "Promo");
        assert_eq!(p1.file_name().unwrap(), "Promo.ove");
        std::fs::create_dir(&p1).unwrap();
        assert_eq!(unique_bundle_path(dir.path(), "Promo").file_name().unwrap(), "Promo 2.ove");
        std::fs::create_dir(dir.path().join("promo 2.ove")).unwrap();
        assert_eq!(unique_bundle_path(dir.path(), "Promo").file_name().unwrap(), "Promo 3.ove");
    }
}
