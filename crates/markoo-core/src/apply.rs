//! Executing a [`Plan`]: the only place where user files are written.
//!
//! Before touching an entry, the applier re-reads both sides; if either
//! changed since the scan, the entry is skipped and will be replanned at the
//! next sync. Overwritten files are backed up and deleted files go to the
//! trash, both under `<data>/`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::entry::{ContentHash, EntryPath, Side};
use crate::fs::{Fs, FsError};
use crate::kb::{ConvertCtx, Dialect, DialectError, NameIndex};
use crate::pair::Pair;
use crate::plan::{Action, AttachmentAction, Plan, PlannedAttachment, PlannedEntry};
use crate::scan::{ReadError, read_canonical};
use crate::state::{BaseEntry, IssueKind, PendingIssue, State, StateError};

/// A file on one side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRef {
    /// Side of the file.
    pub side: Side,
    /// The entry.
    pub path: EntryPath,
    /// For a file written: hash of its bytes right after, so that a revert can
    /// tell whether it changed since.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<ContentHash>,
    /// For a file written: its previous version was kept in the backups.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub backup: bool,
    /// Where the file is, when it is not `path` under its side's folder (an
    /// attachment kept elsewhere in the vault).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<PathBuf>,
}

impl FileRef {
    pub(crate) fn new(side: Side, path: EntryPath) -> Self {
        Self {
            side,
            path,
            hash: None,
            backup: false,
            file: None,
        }
    }
}

/// A file moved within one side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Renamed {
    /// Side of the file.
    pub side: Side,
    /// Where it was.
    pub from: EntryPath,
    /// Where it is now.
    pub to: EntryPath,
}

/// An entry waiting for the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueRef {
    /// The entry.
    pub path: EntryPath,
    /// Why it is waiting.
    pub kind: IssueKind,
}

/// An entry that could not be applied; the next sync will replan it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skipped {
    /// The entry.
    pub path: EntryPath,
    /// Why it was skipped.
    pub reason: String,
}

/// Outcome of applying a plan.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Report {
    /// Folder of this run's backups and trashed files, under `<data>/backups/<pair>/`
    /// and `<data>/trash/<pair>/`. Set when something was written or trashed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
    /// For a revert: the history identifier of the sync it puts back.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reverts: Option<i64>,
    /// Files written (copies, merges, renames).
    pub written: Vec<FileRef>,
    /// Files moved within a side; their new place is also in `written`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub renamed: Vec<Renamed>,
    /// Files moved to the trash.
    pub trashed: Vec<FileRef>,
    /// Entries waiting for the user.
    pub pending: Vec<IssueRef>,
    /// Entries left alone because they only exist in a one-way target.
    pub not_in_source: Vec<EntryPath>,
    /// Entries skipped (changed since the scan, I/O error…).
    pub skipped: Vec<Skipped>,
    /// Attachments changed on both sides, left alone.
    pub attachment_conflicts: Vec<String>,
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
    pub index: &'a NameIndex,
    pub data_dir: &'a Path,
    /// Groups the backups and trashed files of everything this applier does.
    pub run: String,
}

impl<F: Fs> Applier<'_, F> {
    /// Applies a whole plan, then clears the issues it no longer contains:
    /// the user solved those by hand.
    pub fn apply(&self, plan: &Plan) -> Result<Report, StateError> {
        // Attachments first, so that notes referring to them are never ahead.
        let mut attachments = Report::default();
        for attachment in &plan.attachments {
            self.apply_attachment(attachment, &mut attachments)?;
        }
        let mut report = self.apply_entries(&plan.entries)?;
        report.written.extend(attachments.written);
        report.run = self.run_if_used(&report);
        report.skipped.extend(attachments.skipped);
        report.attachment_conflicts = attachments.attachment_conflicts;
        for issue in self.state.pending(&self.pair.id)? {
            let still_pending = plan.entries.iter().any(|entry| {
                entry.path == issue.path && matches!(entry.action, Action::Pending { .. })
            });
            if !still_pending {
                self.state.clear_pending(&self.pair.id, &issue.path)?;
            }
        }
        Ok(report)
    }

    fn apply_attachment(
        &self,
        attachment: &PlannedAttachment,
        report: &mut Report,
    ) -> Result<(), StateError> {
        let name = &attachment.name;
        let asset = self.index.asset_path(name);
        let Some(entry) = EntryPath::from_relative(Path::new(&asset)) else {
            return Ok(());
        };
        match &attachment.action {
            AttachmentAction::Conflict => report.attachment_conflicts.push(name.clone()),
            AttachmentAction::Record { hash } => {
                self.state.put_attachment(&self.pair.id, name, hash)?;
            }
            AttachmentAction::Copy { to, hash } => {
                let result = self.copy_attachment(attachment, *to, hash, &entry);
                match result {
                    Ok(backup) => {
                        self.state.put_attachment(&self.pair.id, name, hash)?;
                        let target = match to {
                            Side::Repo => &attachment.repo_path,
                            Side::Kb => &attachment.kb_path,
                        };
                        let file =
                            (target != &entry.under(self.pair.root(*to))).then(|| target.clone());
                        report.written.push(FileRef {
                            hash: Some(hash.clone()),
                            backup,
                            file,
                            ..FileRef::new(*to, entry)
                        });
                    }
                    Err(e) => report.skipped.push(Skipped {
                        path: entry,
                        reason: e.to_string(),
                    }),
                }
            }
        }
        Ok(())
    }

    fn copy_attachment(
        &self,
        attachment: &PlannedAttachment,
        to: Side,
        hash: &ContentHash,
        entry: &EntryPath,
    ) -> Result<bool, EntryError> {
        let (source, target) = match to {
            Side::Repo => (&attachment.kb_path, &attachment.repo_path),
            Side::Kb => (&attachment.repo_path, &attachment.kb_path),
        };
        let bytes = self.fs.read(source)?;
        if &ContentHash::of_bytes(&bytes) != hash {
            return Err(EntryError::ChangedSinceScan);
        }
        let backup = self.fs.meta(target)?.is_some();
        if backup {
            let previous = self.fs.read(target)?;
            self.fs
                .write_atomic(&self.keep_path("backups", to, entry), &previous)?;
        }
        self.fs.write_atomic(target, &bytes)?;
        Ok(backup)
    }

    /// Applies some entries, leaving the other pending issues as they are.
    pub fn apply_entries(&self, entries: &[PlannedEntry]) -> Result<Report, StateError> {
        let mut report = Report::default();
        for entry in entries {
            match self.apply_entry(entry, &mut report) {
                Ok(()) => {}
                Err(EntryError::State(e)) => return Err(e),
                Err(e) => report.skipped.push(Skipped {
                    path: entry.path.clone(),
                    reason: e.to_string(),
                }),
            }
        }
        report.run = self.run_if_used(&report);
        Ok(report)
    }

    /// The run's folder, if this report put anything in it.
    fn run_if_used(&self, report: &Report) -> Option<String> {
        (!report.written.is_empty() || !report.trashed.is_empty()).then(|| self.run.clone())
    }

    fn apply_entry(&self, entry: &PlannedEntry, report: &mut Report) -> Result<(), EntryError> {
        let path = &entry.path;
        match &entry.action {
            Action::Pending {
                issue,
                kb_content,
                repo_content,
            } => {
                self.state.set_pending(
                    &self.pair.id,
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
                report.written.push(self.write(*to, path, content)?);
                self.record(path, content)?;
            }
            Action::Merge { content } => {
                let merged = ContentHash::of(content);
                for side in [Side::Kb, Side::Repo] {
                    if entry.seen(side) != Some(&merged) {
                        report.written.push(self.write(side, path, content)?);
                    }
                }
                self.record(path, content)?;
            }
            Action::Trash { side } => {
                self.trash(*side, path)?;
                report.trashed.push(FileRef::new(*side, path.clone()));
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
                let moved = self.fs.read(&destination)?;
                report.written.push(FileRef {
                    hash: Some(ContentHash::of_bytes(&moved)),
                    ..FileRef::new(*side, to.clone())
                });
                report.renamed.push(Renamed {
                    side: *side,
                    from: path.clone(),
                    to: to.clone(),
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
        let ctx = ConvertCtx {
            path: &entry.path,
            index: self.index,
        };
        let current = match read_canonical(self.fs, &absolute, self.dialect(side), &ctx) {
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

    fn write(&self, side: Side, path: &EntryPath, canonical: &str) -> Result<FileRef, EntryError> {
        let absolute = path.under(self.pair.root(side));
        let backup = self.fs.meta(&absolute)?.is_some();
        if backup {
            let previous = self.fs.read(&absolute)?;
            self.fs
                .write_atomic(&self.keep_path("backups", side, path), &previous)?;
        }
        let ctx = ConvertCtx {
            path,
            index: self.index,
        };
        let text = self.dialect(side).from_canonical(canonical, &ctx)?;
        self.fs.write_atomic(&absolute, text.as_bytes())?;
        Ok(FileRef {
            hash: Some(ContentHash::of_bytes(text.as_bytes())),
            backup,
            ..FileRef::new(side, path.clone())
        })
    }

    fn trash(&self, side: Side, path: &EntryPath) -> Result<(), EntryError> {
        let absolute = path.under(self.pair.root(side));
        self.fs
            .rename(&absolute, &self.keep_path("trash", side, path))?;
        Ok(())
    }

    /// `<data>/<kind>/<pair>/<run>/<side>/<path>` for this applier's run.
    fn keep_path(&self, kind: &str, side: Side, path: &EntryPath) -> PathBuf {
        keep_path(self.data_dir, kind, &self.pair.id, &self.run, side, path)
    }

    fn record(&self, path: &EntryPath, content: &str) -> Result<(), EntryError> {
        let base = BaseEntry {
            content: content.to_owned(),
            hash: ContentHash::of(content),
            kb_meta: self.fs.meta(&path.under(&self.pair.kb))?,
            repo_meta: self.fs.meta(&path.under(&self.pair.repo))?,
        };
        self.state.put_base(&self.pair.id, path, &base)?;
        self.state.clear_pending(&self.pair.id, path)?;
        Ok(())
    }

    fn forget(&self, path: &EntryPath) -> Result<(), EntryError> {
        self.state.forget_base(&self.pair.id, path)?;
        self.state.clear_pending(&self.pair.id, path)?;
        Ok(())
    }
}

/// `<data>/<kind>/<pair>/<run>/<side>/<path>`
pub(crate) fn keep_path(
    data_dir: &Path,
    kind: &str,
    pair: &str,
    run: &str,
    side: Side,
    path: &EntryPath,
) -> PathBuf {
    path.under(&data_dir.join(kind).join(pair).join(run).join(side.as_str()))
}

/// Identifier of an apply run, used to group its backups and trashed files:
/// the time in milliseconds, made unique within the process so that two runs
/// never share a folder.
pub(crate) fn run_id() -> String {
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        });
    let mut last = LAST.load(Ordering::Relaxed);
    loop {
        let next = now.max(last + 1);
        match LAST.compare_exchange_weak(last, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return next.to_string(),
            Err(seen) => last = seen,
        }
    }
}
