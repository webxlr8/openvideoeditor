//! Text-edit apply (design D6): `{oldText, newText}` edits against the printed projection. Each `oldText` must
//! occur exactly once, spans must not overlap, `newText: ""` deletes; edits are applied from the end backwards.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextEdit {
    pub old_text: String,
    pub new_text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditError {
    pub code: &'static str,
    /// Index of the offending edit.
    pub index: usize,
    /// 1-based line:column of each match (for ambiguous / overlapping edits).
    pub positions: Vec<(usize, usize)>,
    pub message: String,
}

fn line_col(text: &str, at: usize) -> (usize, usize) {
    let before = &text[..at];
    let line = before.matches('\n').count() + 1;
    let col = before.rsplit('\n').next().map(|s| s.chars().count()).unwrap_or(0) + 1;
    (line, col)
}

/// Applies edits to `printed`. All or nothing.
pub fn apply_edits(printed: &str, edits: &[TextEdit]) -> Result<String, EditError> {
    let mut spans: Vec<(usize, usize, usize)> = Vec::new(); // (start, end, edit index)
    for (i, e) in edits.iter().enumerate() {
        if e.old_text.is_empty() {
            return Err(EditError {
                code: "edit.empty",
                index: i,
                positions: vec![],
                message: format!("edit {i}: oldText is empty; anchor insertions on existing text"),
            });
        }
        let found: Vec<usize> = printed.match_indices(&e.old_text).map(|(at, _)| at).collect();
        match found.len() {
            0 => {
                return Err(EditError {
                    code: "edit.notFound",
                    index: i,
                    positions: vec![],
                    message: format!("edit {i}: oldText was not found in the current projection; print it again"),
                });
            }
            1 => spans.push((found[0], found[0] + e.old_text.len(), i)),
            _ => {
                let positions: Vec<_> = found.iter().map(|at| line_col(printed, *at)).collect();
                let at = positions.iter().map(|(l, c)| format!("{l}:{c}")).collect::<Vec<_>>().join(", ");
                return Err(EditError {
                    code: "edit.ambiguous",
                    index: i,
                    positions,
                    message: format!(
                        "edit {i}: oldText occurs {} times (at {at}); include more context so it matches once",
                        found.len()
                    ),
                });
            }
        }
    }
    spans.sort();
    for w in spans.windows(2) {
        if w[1].0 < w[0].1 {
            return Err(EditError {
                code: "edit.overlap",
                index: w[1].2,
                positions: vec![line_col(printed, w[0].0), line_col(printed, w[1].0)],
                message: format!("edits {} and {} overlap", w[0].2, w[1].2),
            });
        }
    }
    let mut out = printed.to_string();
    for (start, end, i) in spans.into_iter().rev() {
        out.replace_range(start..end, &edits[i].new_text);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(o: &str, n: &str) -> TextEdit {
        TextEdit { old_text: o.into(), new_text: n.into() }
    }

    #[test]
    fn edits() {
        let t = "a one\nb two\nc one\n";
        assert_eq!(apply_edits(t, &[e("b two", "b 2"), e("c ", "")]).unwrap(), "a one\nb 2\none\n");
        let amb = apply_edits(t, &[e("one", "1")]).unwrap_err();
        assert_eq!(amb.code, "edit.ambiguous");
        assert_eq!(amb.positions, vec![(1, 3), (3, 3)]);
        assert_eq!(apply_edits(t, &[e("b two", "x"), e("two\nc", "y")]).unwrap_err().code, "edit.overlap");
        assert_eq!(apply_edits(t, &[e("zzz", "x")]).unwrap_err().code, "edit.notFound");
        // Insert by anchoring on existing text.
        assert_eq!(apply_edits(t, &[e("b two\n", "b two\nb2 new\n")]).unwrap(), "a one\nb two\nb2 new\nc one\n");
    }
}
