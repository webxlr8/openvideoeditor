//! Check and apply (spec: Diagnostics, Three-way merge, Base revision availability): parse → read → merge → one
//! validated batch of document ops. The caller commits the ops (an empty batch means no revision).

use std::collections::BTreeSet;
use std::path::Path;

use serde::Serialize;

use ove_core::model::Document;
use ove_core::ops::{Op, apply_batch};

use crate::merge::{Conflict, Prefer, Removed, merge};
use crate::parse::{Diagnostic, Pos, Severity, parse};
use crate::read::{Resolve, read};

/// Why an apply was refused.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Refusal {
    pub code: String,
    pub message: String,
    pub diagnostics: Vec<Diagnostic>,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)?;
        for d in &self.diagnostics {
            write!(f, "\n  {d}")?;
        }
        Ok(())
    }
}

/// A base revision lookup failure.
#[derive(Debug, Clone, PartialEq)]
pub struct BaseError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
}

/// A planned apply.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub base_rev: i64,
    #[serde(skip)]
    pub ops: Vec<Op>,
    pub counts: Counts,
    pub conflicts: Vec<Conflict>,
    pub removed: Vec<Removed>,
    pub warnings: Vec<Diagnostic>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// One-line human summary.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if self.counts.added > 0 {
            parts.push(format!("added {} item{}", self.counts.added, if self.counts.added == 1 { "" } else { "s" }));
        }
        if self.counts.updated > 0 {
            parts.push(format!(
                "updated {} item{}",
                self.counts.updated,
                if self.counts.updated == 1 { "" } else { "s" }
            ));
        }
        if !self.removed.is_empty() {
            let names: Vec<String> = self
                .removed
                .iter()
                .map(|r| if r.name.is_empty() { r.id.clone() } else { format!("{} \"{}\"", r.id, r.name) })
                .collect();
            parts.push(format!(
                "removed {} item{} ({})",
                self.removed.len(),
                if self.removed.len() == 1 { "" } else { "s" },
                names.join(", ")
            ));
        }
        if parts.is_empty() && !self.ops.is_empty() {
            parts.push("updated tracks or settings".into());
        }
        if parts.is_empty() { "no changes".into() } else { parts.join(", ") }
    }
}

pub struct ApplyInput<'a> {
    pub text: &'a str,
    pub current: &'a Document,
    /// Reconstructs the document at a revision (the projection's base).
    pub base_for: &'a dyn Fn(i64) -> Result<Document, BaseError>,
    pub bundle: Option<&'a Path>,
    pub prefer: Prefer,
    pub now_ms: i64,
}

fn refuse(code: &str, message: impl Into<String>, diagnostics: Vec<Diagnostic>) -> Refusal {
    Refusal { code: code.into(), message: message.into(), diagnostics }
}

fn counts(ops: &[Op]) -> Counts {
    let mut c = Counts::default();
    let mut updated = BTreeSet::new();
    for op in ops {
        match op {
            Op::AddItem { .. } => c.added += 1,
            Op::RemoveItems { ids } => c.removed += ids.len(),
            Op::UpdateItem { id, .. } | Op::UnsetItemFields { id, .. } => {
                updated.insert(id.clone());
            }
            _ => {}
        }
    }
    c.updated = updated.len();
    c
}

fn at(pos: Option<&Pos>, code: &str, message: String) -> Diagnostic {
    let p = pos.copied().unwrap_or_default();
    Diagnostic { severity: Severity::Error, line: p.line.max(1), column: p.column.max(1), code: code.into(), message }
}

/// Checks a projection and plans its apply. `Err` means nothing may change.
pub fn plan(input: &ApplyInput) -> Result<Plan, Refusal> {
    let parsed = parse(input.text);
    let mut diags = parsed.diagnostics.clone();
    let rev = match &parsed.rev {
        None => {
            return Err(refuse(
                "rev.missing",
                "the projection has no `rev:` line in its header; print the timeline again",
                diags,
            ));
        }
        Some(Err(m)) => return Err(refuse("rev.malformed", format!("{m}; print the timeline again"), diags)),
        Some(Ok(r)) => *r,
    };
    if rev > input.current.project.rev {
        return Err(refuse(
            "rev.newer",
            format!(
                "the projection's rev {rev} is newer than the project's current revision {}; print the timeline again",
                input.current.project.rev
            ),
            diags,
        ));
    }
    let base = (input.base_for)(rev)
        .map_err(|e| refuse(&e.code, format!("{}; print the timeline again", e.message), diags.clone()))?;
    let Some(root) = &parsed.root else {
        return Err(refuse("check.failed", "the projection has errors", diags));
    };
    let (model, read_diags) = read(root, Resolve { base: &base, current: input.current, bundle: input.bundle });
    diags.extend(read_diags);
    diags.sort_by_key(|d| (d.line, d.column));
    if diags.iter().any(Diagnostic::is_error) {
        return Err(refuse("check.failed", "the projection has errors; nothing was applied", diags));
    }
    let merged = merge(&base, input.current, &model, input.prefer, input.now_ms);
    let ops = ove_core::diff::diff_documents(input.current, &merged.target);
    if !ops.is_empty()
        && let Err(e) = apply_batch(input.current, &ops)
    {
        let mut errs = Vec::new();
        for issue in &e.all {
            let id = issue
                .path
                .split('.')
                .next()
                .unwrap_or("")
                .trim_start_matches("items[")
                .trim_start_matches("tracks[")
                .trim_end_matches(']');
            errs.push(at(merged.positions.get(id), issue.code, issue.message.clone()));
        }
        errs.sort_by_key(|d| (d.line, d.column));
        return Err(refuse("check.failed", "the result would be invalid; nothing was applied", errs));
    }
    let warnings: Vec<Diagnostic> = diags.into_iter().filter(|d| !d.is_error()).collect();
    Ok(Plan {
        base_rev: rev,
        counts: counts(&ops),
        ops,
        conflicts: merged.conflicts,
        removed: merged.removed,
        warnings,
    })
}
