//! Revision records (design D2/D3) and the session that applies batches with undo/redo (design D7).

use serde::{Deserialize, Serialize};

use crate::diff::diff_documents;
use crate::model::Document;
use crate::ops::{BatchError, Op, apply_batch, summarise};
use crate::schema::Issue;

/// Where a batch came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Gui,
    Cli,
    Mcp,
    Agent,
    Migration,
    Undo,
    Redo,
    Restore,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Gui => "gui",
            Source::Cli => "cli",
            Source::Mcp => "mcp",
            Source::Agent => "agent",
            Source::Migration => "migration",
            Source::Undo => "undo",
            Source::Redo => "redo",
            Source::Restore => "restore",
        }
    }
}

/// One committed batch, as stored in `.revs/<rev>.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionRecord {
    pub rev: i64,
    pub parent: i64,
    /// Wall-clock time of the commit, ms since the epoch.
    pub time: i64,
    pub source: Source,
    pub summary: String,
    pub ops: Vec<Op>,
    pub inverse: Vec<Op>,
}

/// Source of wall-clock time, injectable for tests.
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> i64;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
    }
}

/// `rev = max(now_ms, previous_rev + 1)`: millisecond-based like the reference, strictly increasing under clock skew
/// and bursts.
pub fn next_rev(now_ms: i64, prev_rev: i64) -> i64 {
    now_ms.max(prev_rev.saturating_add(1))
}

/// A batch ready to be made durable. Nothing changes in the session until [`Session::commit`].
#[derive(Debug, Clone)]
pub struct Prepared {
    pub doc: Document,
    pub record: RevisionRecord,
    pub warnings: Vec<Issue>,
    kind: StackAction,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum StackAction {
    Normal,
    Undo,
    Redo,
}

#[derive(Debug, Clone)]
struct StackEntry {
    rev: i64,
    forward: Vec<Op>,
    inverse: Vec<Op>,
    summary: String,
}

/// An open document with session-scoped undo/redo stacks. One batch = one undo step, whatever its source.
#[derive(Debug, Clone)]
pub struct Session {
    doc: Document,
    undo: Vec<StackEntry>,
    redo: Vec<StackEntry>,
}

impl Session {
    pub fn new(doc: Document) -> Self {
        Session { doc, undo: Vec::new(), redo: Vec::new() }
    }

    pub fn doc(&self) -> &Document {
        &self.doc
    }

    pub fn rev(&self) -> i64 {
        self.doc.project.rev
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Summary of the batch Undo would revert.
    pub fn undo_summary(&self) -> Option<&str> {
        self.undo.last().map(|e| e.summary.as_str())
    }

    fn prepare_ops(
        &self,
        ops: Vec<Op>,
        source: Source,
        summary: Option<String>,
        now_ms: i64,
        kind: StackAction,
    ) -> Result<Prepared, BatchError> {
        if ops.is_empty() {
            return Err(BatchError {
                op_index: None,
                code: "batch.empty",
                path: String::new(),
                message: "the batch has no operations".into(),
                all: vec![],
            });
        }
        let applied = apply_batch(&self.doc, &ops)?;
        let rev = next_rev(now_ms, self.doc.project.rev);
        let mut doc = applied.doc;
        doc.project.rev = rev;
        doc.project.updated_at = now_ms;
        let summary = summary.unwrap_or_else(|| summarise(&ops));
        Ok(Prepared {
            doc,
            record: RevisionRecord {
                rev,
                parent: self.doc.project.rev,
                time: now_ms,
                source,
                summary,
                ops,
                inverse: applied.inverse,
            },
            warnings: applied.warnings,
            kind,
        })
    }

    /// Validates and applies a batch to a copy. Call [`Session::commit`] once the record is durable.
    pub fn prepare(
        &self,
        ops: Vec<Op>,
        source: Source,
        summary: Option<String>,
        clock: &dyn Clock,
    ) -> Result<Prepared, BatchError> {
        self.prepare_ops(ops, source, summary, clock.now_ms(), StackAction::Normal)
    }

    /// Prepares undo of the most recent batch on the undo stack, or `None` when there is nothing to undo.
    pub fn prepare_undo(&self, clock: &dyn Clock) -> Option<Result<Prepared, BatchError>> {
        let e = self.undo.last()?;
        Some(self.prepare_ops(
            e.inverse.clone(),
            Source::Undo,
            Some(format!("undo: {}", e.summary)),
            clock.now_ms(),
            StackAction::Undo,
        ))
    }

    /// Prepares redo of the most recently undone batch, or `None`.
    pub fn prepare_redo(&self, clock: &dyn Clock) -> Option<Result<Prepared, BatchError>> {
        let e = self.redo.last()?;
        Some(self.prepare_ops(
            e.forward.clone(),
            Source::Redo,
            Some(format!("redo: {}", e.summary)),
            clock.now_ms(),
            StackAction::Redo,
        ))
    }

    /// Prepares a batch that turns the current document into `target` (the document at an earlier revision).
    /// Returns `None` when the content is already equal.
    pub fn prepare_restore(
        &self,
        target: &Document,
        target_rev: i64,
        clock: &dyn Clock,
    ) -> Option<Result<Prepared, BatchError>> {
        let ops = diff_documents(&self.doc, target);
        if ops.is_empty() {
            return None;
        }
        Some(self.prepare_ops(
            ops,
            Source::Restore,
            Some(format!("restore revision {target_rev}")),
            clock.now_ms(),
            StackAction::Normal,
        ))
    }

    /// Makes a prepared batch current and updates the undo/redo stacks.
    pub fn commit(&mut self, p: Prepared) -> RevisionRecord {
        let r = p.record;
        match p.kind {
            StackAction::Normal => {
                self.redo.clear();
                self.undo.push(StackEntry {
                    rev: r.rev,
                    forward: r.ops.clone(),
                    inverse: r.inverse.clone(),
                    summary: r.summary.clone(),
                });
            }
            StackAction::Undo => {
                let mut e = self.undo.pop().expect("undo entry");
                e.rev = r.rev;
                self.redo.push(e);
            }
            StackAction::Redo => {
                let mut e = self.redo.pop().expect("redo entry");
                e.rev = r.rev;
                self.undo.push(e);
            }
        }
        self.doc = p.doc;
        r
    }

    /// Replaces the document without touching the stacks (used on open after replay).
    pub fn reset(&mut self, doc: Document) {
        self.doc = doc;
        self.undo.clear();
        self.redo.clear();
    }

    /// Revision ids on the undo stack, oldest first.
    pub fn undo_revs(&self) -> Vec<i64> {
        self.undo.iter().map(|e| e.rev).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rev_is_monotonic_with_frozen_and_backwards_clock() {
        assert_eq!(next_rev(1000, 999), 1000);
        assert_eq!(next_rev(1000, 1000), 1001);
        assert_eq!(next_rev(500, 1000), 1001);
        let mut prev = 0;
        for now in [5, 5, 5, 3, 2, 10, 10, 1] {
            let r = next_rev(now, prev);
            assert!(r > prev);
            prev = r;
        }
    }
}
