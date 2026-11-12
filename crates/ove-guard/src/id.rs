//! Ids that become file names (`chats/<id>.jsonl`, `jobs/<id>.json`, `<id>.cancel`, `generated/<id>.<ext>`).
//!
//! Ids come from files inside a project bundle, which may have been made by someone else, and from command-line
//! arguments, so every id is checked before it is joined onto a path: ASCII letters, digits, `_` and `-`, 1 to 64
//! characters, and not a reserved Windows device name.

/// Longest id accepted.
pub const MAX_ID_LEN: usize = 64;

const WINDOWS_DEVICES: &[&str] = &["con", "prn", "aux", "nul", "conin$", "conout$"];

/// True when `id` is safe to use as (part of) a file name: `[A-Za-z0-9_-]{1,64}`, not a Windows device name.
pub fn is_safe_id(id: &str) -> bool {
    if id.is_empty() || id.len() > MAX_ID_LEN {
        return false;
    }
    if !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
        return false;
    }
    let lower = id.to_ascii_lowercase();
    let device = WINDOWS_DEVICES.contains(&lower.as_str())
        || ((lower.starts_with("com") || lower.starts_with("lpt"))
            && lower.len() == 4
            && lower.as_bytes()[3].is_ascii_digit());
    !device
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_generated_ids() {
        assert!(is_safe_id("c_18f3a9b2c41"));
        assert!(is_safe_id("g_18f3a9b2c41_0a1b2c3d"));
        assert!(is_safe_id("A-z_09"));
        assert!(is_safe_id(&"a".repeat(64)));
    }

    #[test]
    fn rejects_paths_and_oddities() {
        for bad in [
            "",
            "..",
            "../x",
            "a/b",
            "a\\b",
            "/etc/passwd",
            "C:x",
            "a.b",
            "a b",
            "a\0b",
            "é",
            "con",
            "NUL",
            "com1",
            "LPT9",
            &"a".repeat(65),
        ] {
            assert!(!is_safe_id(bad), "{bad:?} accepted");
        }
        assert!(is_safe_id("com10"));
        assert!(is_safe_id("console"));
    }
}
