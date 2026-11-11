//! Diagnostics reported by `check` and by rendering.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

/// One problem found in a composition. `line`/`column` are 1-based positions in the source, when known.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub severity: Severity,
    /// Stable machine-readable code (`syntax`, `import`, `global`, `budget`, `memory`, `runtime`, `tag`, `style`, …).
    pub code: String,
    pub message: String,
    pub line: Option<u32>,
    pub column: Option<u32>,
}

impl Diagnostic {
    pub fn error(code: &str, message: impl Into<String>) -> Self {
        Self { severity: Severity::Error, code: code.into(), message: message.into(), line: None, column: None }
    }

    pub fn warning(code: &str, message: impl Into<String>) -> Self {
        Self { severity: Severity::Warning, code: code.into(), message: message.into(), line: None, column: None }
    }

    pub fn at(mut self, pos: Option<(u32, u32)>) -> Self {
        if let Some((l, c)) = pos {
            self.line = Some(l);
            self.column = Some(c);
        }
        self
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

/// Byte offset → 1-based (line, column).
pub(crate) fn line_col(src: &str, offset: usize) -> (u32, u32) {
    let offset = offset.min(src.len());
    let before = &src[..src.floor_char_boundary(offset)];
    let line = before.matches('\n').count() as u32 + 1;
    let col = before.rsplit('\n').next().map(|s| s.chars().count()).unwrap_or(0) as u32 + 1;
    (line, col)
}
