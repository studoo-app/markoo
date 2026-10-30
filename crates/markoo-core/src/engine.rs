//! Entry point of the sync engine: scan, plan, apply, resolve.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::Error;
use crate::apply::{Applier, Report};
use crate::entry::{ContentHash, EntryPath, Side};
use crate::fs::{Fs, FsLock};
use crate::kb::obsidian::{self, ObsidianAdapter, VaultSettings};
use crate::kb::{ConvertCtx, Dialect, Gfm, KbAdapter, KbType, MarkdownAdapter, NameIndex};
use crate::merge;
use crate::pair::{Direction, Filter, Pair, TargetChanges};
use crate::paths::Paths;
use crate::plan::{self, Action, AttachmentAction, Plan, PlannedAttachment, PlannedEntry};
use crate::scan::{ScanIssue, SideScan, normalize, scan_side};
use crate::state::{BaseEntry, PendingIssue, State};

/// Name of the state database inside `<data>`.
pub const STATE_FILE: &str = "state.db";
/// Folder of the per-pair locks inside `<data>`.
pub const LOCKS_DIR: &str = "locks";

/// Stable identifier of the issue on `path`, for interfaces where a file path
/// would be awkward (URLs, MCP arguments).
pub fn issue_id(pair: &str, path: &EntryPath) -> String {
    let hash = blake3::hash(format!("{pair}\0{path}").as_bytes()).to_hex();
    format!("c_{}", &hash.as_str()[..16])
}

/// A plan together with the files the scan had to leave out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Planned {
    /// What the sync would do.
    pub plan: Plan,
    /// Files that could not be scanned (unreadable, not UTF-8…).
    pub scan_issues: Vec<ScanIssue>,
}

/// Result of [`Engine::run`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutcome {
    /// The plan that was computed.
    pub planned: Planned,
    /// What was applied, `None` for a dry run.
    pub report: Option<Report>,
    /// The direction changed since the last sync, so this run was forced to be
    /// a dry run: the user sees the plan before anything is written.
    pub direction_changed: bool,
}

/// How the user settles a conflict or a divergence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// Keep one side's version. In a one-way pair, keeping the source restores
    /// the target; keeping the target excludes the file from later syncs.
    Keep(Side),
    /// Keep the KB version and save the repo's as `<name>.conflict-repo.md`.
    KeepBoth,
    /// Use this text on both sides.
    Content(String),
    /// One-way pairs only: a one-off copy of the target's version to the source.
    PushToSource,
}

/// Difference between the two sides of one entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileDiff {
    /// The entry.
    pub path: EntryPath,
    /// Unified diff from the repo version to the KB version.
    pub patch: String,
}

/// The sync engine for one data directory.
#[derive(Debug)]
pub struct Engine<F: Fs> {
    fs: F,
    state: State,
    data_dir: PathBuf,
}

impl<F: Fs> Engine<F> {
    /// Creates an engine from its parts.
    pub fn new(fs: F, state: State, data_dir: impl Into<PathBuf>) -> Self {
        Self {
            fs,
            state,
            data_dir: data_dir.into(),
        }
    }

    /// Opens the engine on the resolved data directory (`<data>/state.db`).
    pub fn open(fs: F, paths: &Paths) -> Result<Self, Error> {
        let state = State::open(&paths.data_dir().join(STATE_FILE))?;
        Ok(Self::new(fs, state, paths.data_dir()))
    }

    /// The file system used by the engine.
    pub fn fs(&self) -> &F {
        &self.fs
    }

    /// Takes the pair's lock, which every write to its files and state needs,
    /// whichever process it comes from. Fails with [`Error::Busy`] if it is held.
    pub fn lock_pair(&self, name: &str) -> Result<FsLock, Error> {
        let path = self.data_dir.join(LOCKS_DIR).join(format!("{name}.lock"));
        self.fs
            .try_lock(&path)?
            .ok_or_else(|| Error::Busy(name.to_owned()))
    }

    /// Computes what a sync of `pair` would do, without writing anything.
    pub fn plan(&self, pair: &Pair) -> Result<Planned, Error> {
        // An unavailable folder (unplugged disk, unmounted volume) lists as
        // empty: without this check every file would look deleted, and the
        // other side would be emptied.
        for side in [Side::Kb, Side::Repo] {
            let root = pair.root(side);
            if !self.fs.is_dir(root) {
                return Err(Error::MissingRoot {
                    pair: pair.name.clone(),
                    side,
                    path: root.to_path_buf(),
                });
            }
        }
        let prepared = self.prepare(pair)?;
        let adapter = prepared.adapter.as_ref();
        let index = &prepared.index;
        let bases = self.state.bases(&pair.name)?;
        let mut kb = scan_side(
            &self.fs,
            &pair.kb,
            Side::Kb,
            &prepared.filter,
            adapter,
            index,
            &bases,
        )?;
        let repo = scan_side(
            &self.fs,
            &pair.repo,
            Side::Repo,
            &prepared.filter,
            &Gfm,
            index,
            &bases,
        )?;
        kb.left_alone.extend(self.state.ignored(&pair.name)?);
        let mut plan = plan::plan(pair, &bases, &kb, &repo);
        plan.attachments = self.plan_attachments(pair, &prepared, &bases, &kb, &repo)?;
        let mut scan_issues = kb.issues;
        scan_issues.extend(repo.issues);
        for (file, message) in index.take_warnings() {
            scan_issues.push(ScanIssue {
                path: pair.kb.join(file),
                message,
            });
        }
        Ok(Planned { plan, scan_issues })
    }

    /// Everything needed to convert the files of a pair.
    fn prepare(&self, pair: &Pair) -> Result<Prepared, Error> {
        let (adapter, vault_root, settings): (Box<dyn KbAdapter>, PathBuf, VaultSettings) =
            match pair.kb_type {
                KbType::Markdown => (
                    Box::new(MarkdownAdapter),
                    pair.kb.clone(),
                    VaultSettings::default(),
                ),
                KbType::Obsidian => {
                    let root = obsidian::find_vault_root(&self.fs, &pair.kb)
                        .unwrap_or_else(|| pair.kb.clone());
                    let settings = VaultSettings::read(&self.fs, &root);
                    let adapter = ObsidianAdapter {
                        options: pair.kb_options.clone(),
                        settings: settings.clone(),
                    };
                    (Box::new(adapter), root, settings)
                }
            };
        let filter = Filter::new(pair, adapter.excluded())?;
        let index = NameIndex::build(
            &self.fs,
            pair,
            &filter,
            &vault_root,
            adapter.excluded_folders(),
        )?;
        Ok(Prepared {
            adapter,
            filter,
            index,
            vault_root,
            settings,
        })
    }

    /// Plans the attachments the notes refer to.
    fn plan_attachments(
        &self,
        pair: &Pair,
        prepared: &Prepared,
        bases: &BTreeMap<EntryPath, BaseEntry>,
        kb: &SideScan,
        repo: &SideScan,
    ) -> Result<Vec<PlannedAttachment>, Error> {
        let mut referenced: BTreeMap<String, EntryPath> = BTreeMap::new();
        let mut collect = |path: &EntryPath, canonical: &str| {
            let ctx = ConvertCtx {
                path,
                index: &prepared.index,
            };
            for name in prepared.adapter.referenced_assets(canonical, &ctx) {
                referenced.entry(name).or_insert_with(|| path.clone());
            }
        };
        for (path, base) in bases {
            collect(path, &base.content);
        }
        for scan in [kb, repo] {
            for (path, observed) in &scan.files {
                if let Some(content) = &observed.content {
                    collect(path, content);
                }
            }
        }

        let mut planned = Vec::new();
        for (name, note) in referenced {
            let Some(repo_path) =
                EntryPath::from_relative(Path::new(&prepared.index.asset_path(&name)))
                    .map(|asset| asset.under(&pair.repo))
            else {
                continue;
            };
            let kb_path = prepared.attachment_location(pair, &name, &note);
            let kb_hash = self.hash_of(&kb_path)?;
            let repo_hash = self.hash_of(&repo_path)?;
            let base = self.state.attachment_hash(&pair.name, &name)?;
            if let Some(action) = attachment_action(pair, kb_hash, repo_hash, base) {
                planned.push(PlannedAttachment {
                    name,
                    kb_path,
                    repo_path,
                    action,
                });
            }
        }
        Ok(planned)
    }

    fn hash_of(&self, path: &Path) -> Result<Option<ContentHash>, Error> {
        if self.fs.meta(path)?.is_none() {
            return Ok(None);
        }
        Ok(Some(ContentHash::of_bytes(&self.fs.read(path)?)))
    }

    /// Applies a plan computed by [`Engine::plan`].
    pub fn apply(&self, pair: &Pair, plan: &Plan) -> Result<Report, Error> {
        let _lock = self.lock_pair(&pair.name)?;
        self.apply_locked(pair, plan)
    }

    fn apply_locked(&self, pair: &Pair, plan: &Plan) -> Result<Report, Error> {
        let prepared = self.prepare(pair)?;
        Ok(self.applier(pair, &prepared).apply(plan)?)
    }

    /// Plans and applies in one go, without the direction safeguard of [`Engine::run`].
    pub fn sync(&self, pair: &Pair) -> Result<Report, Error> {
        let _lock = self.lock_pair(&pair.name)?;
        let planned = self.plan(pair)?;
        self.apply_locked(pair, &planned.plan)
    }

    /// The sync used by every trigger: plans, then applies unless `dry_run`.
    ///
    /// After a change of direction, the run is forced to be a dry run. Moving
    /// from `both` to a one-way mode is refused while issues are pending, so
    /// Markoo never decides on the user's behalf.
    pub fn run(&self, pair: &Pair, dry_run: bool) -> Result<RunOutcome, Error> {
        let current = direction_key(pair.direction);
        let previous = self.state.last_direction(&pair.name)?;
        let direction_changed = previous.as_deref().is_some_and(|last| last != current);
        // A plain dry run writes nothing, so it can run alongside a sync.
        let _lock = if dry_run && !direction_changed {
            None
        } else {
            Some(self.lock_pair(&pair.name)?)
        };
        let planned = self.plan(pair)?;

        if direction_changed
            && previous.as_deref() == Some(direction_key(Direction::Both))
            && pair.direction != Direction::Both
        {
            let pending = self.state.pending(&pair.name)?.len();
            if pending > 0 {
                return Err(Error::DirectionBlocked {
                    pair: pair.name.clone(),
                    pending,
                });
            }
        }
        if !dry_run || direction_changed {
            self.state.set_last_direction(&pair.name, current)?;
        }

        let report = if dry_run || direction_changed {
            None
        } else {
            Some(self.apply_locked(pair, &planned.plan)?)
        };
        Ok(RunOutcome {
            planned,
            report,
            direction_changed,
        })
    }

    /// Conflicts and divergences as of now, computed from a fresh scan.
    pub fn issues(&self, pair: &Pair) -> Result<Vec<PendingIssue>, Error> {
        let planned = self.plan(pair)?;
        Ok(planned
            .plan
            .entries
            .into_iter()
            .filter_map(|entry| match entry.action {
                Action::Pending {
                    issue,
                    kb_content,
                    repo_content,
                } => Some(PendingIssue {
                    path: entry.path,
                    kind: issue,
                    kb: kb_content,
                    repo: repo_content,
                }),
                _ => None,
            })
            .collect())
    }

    /// Entries the user excluded by keeping a one-way target.
    pub fn ignored(&self, pair: &Pair) -> Result<Vec<EntryPath>, Error> {
        Ok(self.state.ignored(&pair.name)?.into_iter().collect())
    }

    /// Whether moving from `both` to a one-way mode would be refused.
    pub fn pending_count(&self, pair: &Pair) -> Result<usize, Error> {
        Ok(self.state.pending(&pair.name)?.len())
    }

    /// Text to edit by hand to settle an issue, with conflict markers.
    pub fn conflict_draft(&self, pair: &Pair, path: &EntryPath) -> Result<String, Error> {
        let (_, issue) = self.pending_entry(pair, path)?;
        let bases = self.state.bases(&pair.name)?;
        let base = bases.get(path).map_or("", |base| base.content.as_str());
        Ok(merge::with_markers(
            base,
            issue.kb.as_deref().unwrap_or_default(),
            issue.repo.as_deref().unwrap_or_default(),
        ))
    }

    /// Settles a conflict or a divergence.
    pub fn resolve(
        &self,
        pair: &Pair,
        path: &EntryPath,
        resolution: Resolution,
    ) -> Result<Report, Error> {
        let _lock = self.lock_pair(&pair.name)?;
        let source = pair.direction.source();
        if self.state.ignored(&pair.name)?.contains(path) {
            // The only way back for an excluded file is to restore the source.
            if !matches!(resolution, Resolution::Keep(side) if Some(side) == source) {
                return Err(Error::AlreadyExcluded(path.clone()));
            }
            self.state.unignore(&pair.name, path)?;
        }
        let (entry, issue) = self.pending_entry(pair, path)?;
        let side_content = |side: Side| match side {
            Side::Kb => issue.kb.clone(),
            Side::Repo => issue.repo.clone(),
        };
        let with_action = |action| PlannedEntry {
            path: path.clone(),
            kb: entry.kb.clone(),
            repo: entry.repo.clone(),
            action,
        };

        let prepared = self.prepare(pair)?;
        let applier = self.applier(pair, &prepared);
        match (resolution, source) {
            (Resolution::Keep(side), Some(source)) if side != source => {
                // Keeping the target of a one-way pair: exclude it from now on.
                self.state.ignore(&pair.name, path)?;
                self.state.clear_pending(&pair.name, path)?;
                Ok(Report::default())
            }
            (Resolution::Keep(side), _) => {
                let action = match side_content(side) {
                    Some(content) => Action::Copy {
                        to: side.other(),
                        content,
                    },
                    None => Action::Trash { side: side.other() },
                };
                Ok(applier.apply_entries(&[with_action(action)])?)
            }
            (Resolution::PushToSource, Some(source)) => {
                let action = match side_content(source.other()) {
                    Some(content) => Action::Copy {
                        to: source,
                        content,
                    },
                    None => Action::Trash { side: source },
                };
                Ok(applier.apply_entries(&[with_action(action)])?)
            }
            (Resolution::PushToSource, None) => Err(Error::NeedsOneWay),
            (Resolution::KeepBoth | Resolution::Content(_), Some(_)) => {
                Err(Error::NeedsBidirectional)
            }
            (Resolution::Content(text), None) => {
                let action = Action::Merge {
                    content: normalize(&text),
                };
                Ok(applier.apply_entries(&[with_action(action)])?)
            }
            (Resolution::KeepBoth, None) => {
                let (Some(kb), Some(repo)) = (issue.kb.clone(), issue.repo.clone()) else {
                    return Err(Error::KeepBothNeedsTwoVersions(path.clone()));
                };
                let copy = PlannedEntry {
                    path: conflict_copy_path(path)?,
                    kb: None,
                    repo: None,
                    action: Action::Merge { content: repo },
                };
                // Save the repo version first: if that fails, nothing is lost.
                let mut report = applier.apply_entries(&[copy])?;
                if report.skipped.is_empty() {
                    let main =
                        applier.apply_entries(&[with_action(Action::Merge { content: kb })])?;
                    report.written.extend(main.written);
                    report.skipped.extend(main.skipped);
                }
                Ok(report)
            }
        }
    }

    /// Unified diffs of the entries that differ between the two sides.
    /// With `only`, the output is limited to that entry.
    pub fn diffs(&self, pair: &Pair, only: Option<&EntryPath>) -> Result<Vec<FileDiff>, Error> {
        let mut diffs = Vec::new();
        let prepared = self.prepare(pair)?;
        let adapter = prepared.adapter.as_ref();
        let mut paths = self.side_entries(&pair.kb, &prepared.filter)?;
        paths.extend(self.side_entries(&pair.repo, &prepared.filter)?);
        paths.sort();
        paths.dedup();
        for path in paths {
            if only.is_some_and(|only| only != &path) {
                continue;
            }
            let ctx = ConvertCtx {
                path: &path,
                index: &prepared.index,
            };
            let kb = self.canonical(&path.under(&pair.kb), adapter, &ctx);
            let repo = self.canonical(&path.under(&pair.repo), &Gfm, &ctx);
            let (Ok(kb), Ok(repo)) = (kb, repo) else {
                continue; // unreadable: reported by the scan
            };
            let same = kb.as_deref().map(ContentHash::of) == repo.as_deref().map(ContentHash::of);
            if !same {
                let patch = diffy::create_patch(
                    repo.as_deref().unwrap_or_default(),
                    kb.as_deref().unwrap_or_default(),
                );
                diffs.push(FileDiff {
                    path,
                    patch: patch.to_string(),
                });
            }
        }
        Ok(diffs)
    }

    /// Forgets everything known about a pair. User files are not touched.
    pub fn forget_pair(&self, name: &str) -> Result<(), Error> {
        Ok(self.state.forget_pair(name)?)
    }

    fn applier<'a>(&'a self, pair: &'a Pair, prepared: &'a Prepared) -> Applier<'a, F> {
        Applier {
            fs: &self.fs,
            state: &self.state,
            pair,
            kb_dialect: prepared.adapter.as_ref(),
            repo_dialect: &Gfm,
            index: &prepared.index,
            data_dir: &self.data_dir,
        }
    }

    fn pending_entry(
        &self,
        pair: &Pair,
        path: &EntryPath,
    ) -> Result<(PlannedEntry, PendingIssue), Error> {
        let planned = self.plan(pair)?;
        let entry = planned
            .plan
            .entries
            .into_iter()
            .find(|entry| &entry.path == path)
            .ok_or_else(|| Error::NotPending(path.clone()))?;
        let Action::Pending {
            issue,
            kb_content,
            repo_content,
        } = entry.action.clone()
        else {
            return Err(Error::NotPending(path.clone()));
        };
        let pending = PendingIssue {
            path: path.clone(),
            kind: issue,
            kb: kb_content,
            repo: repo_content,
        };
        Ok((entry, pending))
    }

    fn side_entries(&self, root: &Path, filter: &Filter) -> Result<Vec<EntryPath>, Error> {
        Ok(self
            .fs
            .list(root)?
            .iter()
            .filter_map(|relative| EntryPath::from_relative(relative))
            .filter(|path| filter.accepts(path))
            .collect())
    }

    fn canonical(
        &self,
        path: &Path,
        dialect: &dyn Dialect,
        ctx: &ConvertCtx<'_>,
    ) -> Result<Option<String>, ()> {
        crate::scan::read_canonical(&self.fs, path, dialect, ctx).map_err(|_| ())
    }
}

/// The adapter, filter and index of a pair.
struct Prepared {
    adapter: Box<dyn KbAdapter>,
    filter: Filter,
    index: NameIndex,
    vault_root: PathBuf,
    settings: VaultSettings,
}

impl Prepared {
    /// Where an attachment is in the vault, or where a new one goes: the folder
    /// set in Obsidian, next to the note, or the vault root.
    fn attachment_location(&self, pair: &Pair, name: &str, note: &EntryPath) -> PathBuf {
        if let Some(found) = self.index.attachment_in_vault(name) {
            return EntryPath::from_relative(Path::new(&found)).map_or_else(
                || self.vault_root.join(&found),
                |entry| entry.under(&self.vault_root),
            );
        }
        let note_folder = || {
            let file = note.under(&pair.kb);
            file.parent()
                .map_or_else(|| pair.kb.clone(), Path::to_path_buf)
        };
        let folder = match self.settings.attachment_folder.as_deref() {
            None | Some("" | "/") => self.vault_root.clone(),
            Some("./") => note_folder(),
            Some(relative) => match relative.strip_prefix("./") {
                Some(sub) => note_folder().join(sub),
                None => self.vault_root.join(relative),
            },
        };
        folder.join(name)
    }
}

/// Three-way decision on one attachment. Absent on both sides: nothing to do.
fn attachment_action(
    pair: &Pair,
    kb: Option<ContentHash>,
    repo: Option<ContentHash>,
    base: Option<ContentHash>,
) -> Option<AttachmentAction> {
    if let Some(source) = pair.direction.source() {
        let (s, t) = match source {
            Side::Kb => (kb, repo),
            Side::Repo => (repo, kb),
        };
        let target = source.other();
        let s = s?;
        return match t {
            Some(t) if t == s => {
                (base.as_ref() != Some(&s)).then_some(AttachmentAction::Record { hash: s })
            }
            Some(t)
                if base.as_ref() != Some(&t) && pair.target_changes == TargetChanges::Protect =>
            {
                Some(AttachmentAction::Conflict)
            }
            _ => Some(AttachmentAction::Copy {
                to: target,
                hash: s,
            }),
        };
    }
    match (kb, repo) {
        (None, None) => None,
        (Some(hash), None) => Some(AttachmentAction::Copy {
            to: Side::Repo,
            hash,
        }),
        (None, Some(hash)) => Some(AttachmentAction::Copy { to: Side::Kb, hash }),
        (Some(k), Some(r)) if k == r => {
            (base.as_ref() != Some(&k)).then_some(AttachmentAction::Record { hash: k })
        }
        (Some(k), Some(r)) => {
            if base.as_ref() == Some(&k) {
                Some(AttachmentAction::Copy {
                    to: Side::Kb,
                    hash: r,
                })
            } else if base.as_ref() == Some(&r) {
                Some(AttachmentAction::Copy {
                    to: Side::Repo,
                    hash: k,
                })
            } else {
                Some(AttachmentAction::Conflict)
            }
        }
    }
}

fn direction_key(direction: Direction) -> &'static str {
    match direction {
        Direction::Both => "both",
        Direction::KbToRepo => "kb-to-repo",
        Direction::RepoToKb => "repo-to-kb",
    }
}

/// `notes/a.md` → `notes/a.conflict-repo.md`
fn conflict_copy_path(path: &EntryPath) -> Result<EntryPath, Error> {
    let original = Path::new(path.as_str());
    let stem = original.file_stem().and_then(|stem| stem.to_str());
    let extension = original.extension().and_then(|ext| ext.to_str());
    let name = match (stem, extension) {
        (Some(stem), Some(extension)) => format!("{stem}.conflict-repo.{extension}"),
        (Some(stem), None) => format!("{stem}.conflict-repo"),
        _ => return Err(Error::NotPending(path.clone())),
    };
    EntryPath::from_relative(&original.with_file_name(name))
        .ok_or_else(|| Error::NotPending(path.clone()))
}
