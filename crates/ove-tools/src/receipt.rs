//! What a tool call returns: JSON for programs, a line of text for people, and PNG images for agents that can see.

use serde_json::Value;

/// One PNG image.
#[derive(Debug, Clone, PartialEq)]
pub struct Png {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// The result of a successful tool call.
#[derive(Debug, Clone, PartialEq)]
pub struct Receipt {
    /// The structured result (MCP `structuredContent`, `--json` output).
    pub json: Value,
    /// The human-readable result (command-line output).
    pub text: String,
    /// Images, in order (frames, sheets, waveforms).
    pub images: Vec<Png>,
}

impl Receipt {
    pub fn new(json: Value, text: impl Into<String>) -> Receipt {
        Receipt { json, text: text.into(), images: Vec::new() }
    }

    pub fn with_images(mut self, images: Vec<Png>) -> Receipt {
        self.images = images;
        self
    }
}

/// Base64 (standard alphabet, padded) for image blocks.
pub fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_pads() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
}
