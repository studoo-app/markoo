//! Executing a [`Plan`]: the only place where user files are written.
//!
//! Before touching an entry, the applier re-reads both sides; if either
//! changed since the scan, the entry is skipped and will be replanned at the
//! next sync. Overwritten files are backed up and deleted files go to the
//! trash, both under `<data>/`.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::entry::{ContentHash, EntryPath, Side};
use crate::fs::{Fs, FsError};
use crate::kb::{Dialect, DialectError};
use crate::pair::Pair;
use crate::plan::{Action, Plan, PlannedEntry};
use crate::scan::{ReadError, read_canonical};
use crate::state::{BaseEntry, IssueKind, PendingIssue, State, StateError};

/// A file on one side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileRef {
    /// Side of the file.
    pub side: Side,
    /// The entry.
    pub path: EntryPath,
}

/// An entry waiting for the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IssueRef {
    /// The entry.
    pub path: EntryPath,
    /// Why it is waiting.
    pub kind: IssueKind,
}

/// An entry that could not be applied; the next sync will replan it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Skipped {
    /// The entry.
    pub path: EntryPath,
    /// Why it was skipped.
    pub reason: String,
}

/// Outcome of applying a plan.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Report {
    /// Files written (copies, merges, renames).
    pub written: Vec<FileRef>,
    /// Files moved to the trash.
    pub trashed: Vec<FileRef>,
    /// Entries waiting for the user.
    pub pending: Vec<IssueRef>,
    /// Entries left alone because they only exist in a one-way target.
    pub not_in_source: Vec<EntryPath>,
    /// Entries skipped (changed since the scan, I/O error…).
    pub skipped: Vec<Skipped>,
}

/// Why an entry could not be applied.
#[derive(Debug, thiserror::Error)]
enum EntryError {
    #[error("changed since the scan")]
    ChangedSinceScan,
    #[error("{0}")]
    Fs(#[from] FsError),
    #[error("{0}")]
    Dialect(#[from] DialectError),
    #[error("{0}")]
    Unreadable(String),
    #[error(transparent)]
    State(#[from] StateError),
}

pub(crate) struct Applier<'a, F: Fs> {
    pub fs: &'a F,
    pub state: &'a State,
    pub pair: &'a Pair,
    pub kb_dialect: &'a dyn Dialect,
    pub repo_dialect: &'a dyn Dialect,
    pub data_dir: &'a Path,
}

impl<F: Fs> Applier<'_, F> {
    /// Applies a whole plan, then clears the issues it no longer contains:
    /// the user solved those by hand.
    pub fn apply(&self, plan: &Plan) -> Result<Report, StateError> {
        let report = self.apply_entries(&plan.entries)?;
        for issue in self.state.pending(&self.pair.name)? {
            let still_pending = plan.entries.iter().any(|entry| {
                entry.path == issue.path && matches!(entry.action, Action::Pending { .. })
            });
            if !still_pending {
                self.state.clear_pending(&self.pair.name, &issue.path)?;
            }
        }
        Ok(report)
    }

    /// Applies some entries, leaving the other pending issues as they are.
    pub fn apply_entries(&self, entries: &[PlannedEntry]) -> Result<Report, StateError> {
        let run = run_id();
        let mut report = Report::default();
        for entry in entries {
            match self.apply_entry(entry, &run, &mut report) {
                Ok(()) => {}
                Err(EntryError::State(e)) => return Err(e),
                Err(e) => report.skipped.push(Skipped {
                    path: entry.path.clone(),
                    reason: e.to_string(),
                }),
            }
        }
        Ok(report)
    }

    fn apply_entry(
        &self,
        entry: &PlannedEntry,
        run: &str,
        report: &mut Report,
    ) -> Result<(), EntryError> {
        let path = &entry.path;
        match &entry.action {
            Action::Pending {
                issue,
                kb_content,
                repo_content,
            } => {
                self.state.set_pending(
                    &self.pair.name,
                    &PendingIssue {
                        path: path.clone(),
                        kind: *issue,
                        kb: kb_content.clone(),
                        repo: repo_content.clone(),
                    },
                )?;
                report.pending.push(IssueRef {
                    path: path.clone(),
                    kind: *issue,
                });
                return Ok(());
            }
            Action::NotInSource { .. } => {
                report.not_in_source.push(path.clone());
                return Ok(());
            }
            _ => {}
        }

        self.ensure_unchanged(entry, Side::Kb)?;
        self.ensure_unchanged(entry, Side::Repo)?;

        match &entry.action {
            Action::Copy { to, content } => {
                self.write(*to, path, content, run)?;
                report.written.push(FileRef {
                    side: *to,
                    path: path.clone(),
                });
                self.record(path, content)?;
            }
            Action::Merge { content } => {
                let merged = ContentHash::of(content);
                for side in [Side::Kb, Side::Repo] {
                    if entry.seen(side) != Some(&merged) {
                        self.write(side, path, content, run)?;
                        report.written.push(FileRef {
                            side,
                            path: path.clone(),
                        });
                    }
                }
                self.record(path, content)?;
            }
            Action::Trash { side } => {
                self.trash(*side, path, run)?;
                report.trashed.push(FileRef {
                    side: *side,
                    path: path.clone(),
                });
                self.forget(path)?;
            }
            Action::RecordBase { content } => self.record(path, content)?,
            Action::ForgetBase => self.forget(path)?,
            Action::Rename { side, to, content } => {
                let destination = to.under(self.pair.root(*side));
                if self.fs.meta(&destination)?.is_some() {
                    return Err(EntryError::ChangedSinceScan);
                }
                self.fs
                    .rename(&path.under(self.pair.root(*side)), &destination)?;
                report.written.push(FileRef {
                    side: *side,
                    path: to.clone(),
                });
                self.forget(path)?;
                self.record(to, content)?;
            }
            Action::Pending { .. } | Action::NotInSource { .. } => {}
        }
        Ok(())
    }

    fn dialect(&self, side: Side) -> &dyn Dialect {
        match side {
            Side::Kb => self.kb_dialect,
            Side::Repo => self.repo_dialect,
        }
    }

    fn ensure_unchanged(&self, entry: &PlannedEntry, side: Side) -> Result<(), EntryError> {
        let absolute = entry.path.under(self.pair.root(side));
        let current = match read_canonical(self.fs, &absolute, self.dialect(side)) {
            Ok(content) => content.map(|c| ContentHash::of(&c)),
            Err(ReadError::Fs(e)) => return Err(e.into()),
            Err(ReadError::Invalid(message)) => return Err(EntryError::Unreadable(message)),
        };
        if current.as_ref() == entry.seen(side) {
            Ok(())
        } else {
            Err(EntryError::ChangedSinceScan)
        }
    }

    fn write(
        &self,
        side: Side,
        path: &EntryPath,
        canonical: &str,
        run: &str,
    ) -> Result<(), EntryError> {
        let absolute = path.under(self.pair.root(side));
        if self.fs.meta(&absolute)?.is_some() {
            let previous = self.fs.read(&absolute)?;
            self.fs
                .write_atomic(&self.keep_dir("backups", side, run, path), &previous)?;
        }
        let text = self.dialect(side).from_canonical(canonical)?;
        self.fs.write_atomic(&absolute, text.as_bytes())?;
        Ok(())
    }

    fn trash(&self, side: Side, path: &EntryPath, run: &str) -> Result<(), EntryError> {
        let absolute = path.under(self.pair.root(side));
        self.fs
            .rename(&absolute, &self.keep_dir("trash", side, run, path))?;
        Ok(())
    }

    /// `<data>/<kind>/<pair>/<run>/<side>/<path>`
    fn keep_dir(&self, kind: &str, side: Side, run: &str, path: &EntryPath) -> PathBuf {
        let root = self
            .data_dir
            .join(kind)
            .join(&self.pair.name)
            .join(run)
            .join(side.as_str());
        path.under(&root)
    }

    fn record(&self, path: &EntryPath, content: &str) -> Result<(), EntryError> {
        let base = BaseEntry {
            content: content.to_owned(),
            hash: ContentHash::of(content),
            kb_meta: self.fs.meta(&path.under(&self.pair.kb))?,
            repo_meta: self.fs.meta(&path.under(&self.pair.repo))?,
        };
        self.state.put_base(&self.pair.name, path, &base)?;
        self.state.clear_pending(&self.pair.name, path)?;
        Ok(())
    }

    fn forget(&self, path: &EntryPath) -> Result<(), EntryError> {
        self.state.forget_base(&self.pair.name, path)?;
        self.state.clear_pending(&self.pair.name, path)?;
        Ok(())
    }
}

/// Identifier of an apply run, used to group its backups and trashed files.
fn run_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis());
    millis.to_string()
}
