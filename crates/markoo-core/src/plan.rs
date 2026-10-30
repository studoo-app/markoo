//! Turning two scans and the bases into a [`Plan`].
//!
//! Planning never writes anything. Every entry is classified following the
//! classification table of the sync algorithm design doc, adjusted for the
//! pair's direction, then exact renames are detected.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;

use serde::Serialize;

use crate::entry::{ContentHash, EntryPath, Side};
use crate::merge;
use crate::pair::{Direction, Pair, TargetChanges};
use crate::scan::{Observed, SideScan};
use crate::state::{BaseEntry, IssueKind};

/// Everything a sync would do for one pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Plan {
    /// Name of the pair.
    pub pair: String,
    /// Planned entries, ordered by path. Entries with nothing to do are omitted.
    pub entries: Vec<PlannedEntry>,
    /// Planned attachments, ordered by name.
    pub attachments: Vec<PlannedAttachment>,
}

/// What to do with one attachment. Attachments follow the notes that refer to
/// them; they are copied, never deleted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlannedAttachment {
    /// File name, unique in the repo's assets folder.
    pub name: String,
    /// Location in the knowledge base.
    pub kb_path: PathBuf,
    /// Location in the repo.
    pub repo_path: PathBuf,
    /// The action.
    pub action: AttachmentAction,
}

/// An action on one attachment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum AttachmentAction {
    /// Copy the file to one side.
    Copy {
        /// Side written.
        to: Side,
        /// Hash of the source, checked again just before copying.
        hash: ContentHash,
    },
    /// Both sides already agree: only record it.
    Record {
        /// Hash shared by both sides.
        hash: ContentHash,
    },
    /// Both sides changed the file: it is reported and left alone.
    Conflict,
}

impl Plan {
    /// Whether the sync has nothing to do.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.attachments.is_empty()
    }
}

/// What to do with one entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlannedEntry {
    /// The entry.
    pub path: EntryPath,
    /// Hash seen on the KB side at scan time (`None` if absent). The applier
    /// skips the entry if the file changed since.
    pub kb: Option<ContentHash>,
    /// Hash seen on the repo side at scan time (`None` if absent).
    pub repo: Option<ContentHash>,
    /// The action.
    pub action: Action,
}

impl PlannedEntry {
    /// Hash seen at scan time on a side.
    pub fn seen(&self, side: Side) -> Option<&ContentHash> {
        match side {
            Side::Kb => self.kb.as_ref(),
            Side::Repo => self.repo.as_ref(),
        }
    }
}

/// An action on one entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Action {
    /// Write `content` to one side (creation, update or restore).
    Copy {
        /// Side written.
        to: Side,
        /// Canonical content.
        content: String,
    },
    /// Write the merged `content` to both sides.
    Merge {
        /// Canonical merged content.
        content: String,
    },
    /// Move the entry to the trash on one side.
    Trash {
        /// Side affected.
        side: Side,
    },
    /// Both sides already agree: only record the new base.
    RecordBase {
        /// Canonical content shared by both sides.
        content: String,
    },
    /// The entry is gone from both sides: forget its base.
    ForgetBase,
    /// The entry was renamed on the other side: rename it on `side` too.
    Rename {
        /// Side where the file is renamed.
        side: Side,
        /// New path.
        to: EntryPath,
        /// Canonical content (unchanged by the rename).
        content: String,
    },
    /// Nothing is written: the user must decide.
    Pending {
        /// Why.
        issue: IssueKind,
        /// Canonical KB content, `None` if absent.
        kb_content: Option<String>,
        /// Canonical repo content, `None` if absent.
        repo_content: Option<String>,
    },
    /// The entry only exists in the target of a one-way pair; it is left alone.
    NotInSource {
        /// The target side.
        side: Side,
    },
}

/// State of an entry on one side, compared with its base.
#[derive(Debug, Clone, Copy)]
enum Seen<'a> {
    Absent,
    Same(&'a Observed),
    Changed(&'a Observed),
}

impl<'a> Seen<'a> {
    fn new(observed: Option<&'a Observed>, base: Option<&BaseEntry>) -> Self {
        match (observed, base) {
            (None, _) => Seen::Absent,
            (Some(observed), Some(base)) if observed.hash == base.hash => Seen::Same(observed),
            (Some(observed), _) => Seen::Changed(observed),
        }
    }

    fn observed(self) -> Option<&'a Observed> {
        match self {
            Seen::Absent => None,
            Seen::Same(observed) | Seen::Changed(observed) => Some(observed),
        }
    }
}

/// Canonical content of an observed file (the base's when it was not read).
fn content_of(observed: &Observed, base: Option<&BaseEntry>) -> String {
    match (&observed.content, base) {
        (Some(content), _) => content.clone(),
        (None, Some(base)) => base.content.clone(),
        // The scanner only skips reading when a base exists.
        (None, None) => String::new(),
    }
}

/// Builds the plan of a pair.
pub fn plan(
    pair: &Pair,
    bases: &BTreeMap<EntryPath, BaseEntry>,
    kb: &SideScan,
    repo: &SideScan,
) -> Plan {
    let paths: BTreeSet<&EntryPath> = bases
        .keys()
        .chain(kb.files.keys())
        .chain(repo.files.keys())
        .collect();

    let mut entries = BTreeMap::new();
    for path in paths {
        // Unknown content or excluded by the user: planning anything could lose data.
        if kb.left_alone.contains(path) || repo.left_alone.contains(path) {
            continue;
        }
        let base = bases.get(path);
        let kb_seen = Seen::new(kb.files.get(path), base);
        let repo_seen = Seen::new(repo.files.get(path), base);
        let action = match pair.direction.source() {
            None => classify_both(base, kb_seen, repo_seen),
            Some(source) => {
                let (s, t) = match source {
                    Side::Kb => (kb_seen, repo_seen),
                    Side::Repo => (repo_seen, kb_seen),
                };
                classify_one_way(base, source, s, t, pair.target_changes, kb_seen, repo_seen)
            }
        };
        if let Some(action) = action {
            entries.insert(
                path.clone(),
                PlannedEntry {
                    path: path.clone(),
                    kb: kb_seen.observed().map(|o| o.hash.clone()),
                    repo: repo_seen.observed().map(|o| o.hash.clone()),
                    action,
                },
            );
        }
    }

    detect_renames(pair.direction, bases, &mut entries);

    Plan {
        pair: pair.name.clone(),
        entries: entries.into_values().collect(),
        attachments: Vec::new(),
    }
}

fn pending(issue: IssueKind, base: Option<&BaseEntry>, kb: Seen<'_>, repo: Seen<'_>) -> Action {
    Action::Pending {
        issue,
        kb_content: kb.observed().map(|o| content_of(o, base)),
        repo_content: repo.observed().map(|o| content_of(o, base)),
    }
}

/// Classification table for `direction = "both"`.
fn classify_both(base: Option<&BaseEntry>, kb: Seen<'_>, repo: Seen<'_>) -> Option<Action> {
    use Seen::{Absent, Changed, Same};
    let action = match (kb, repo) {
        // 1, 13
        (Absent, Absent) => base.map(|_| Action::ForgetBase)?,
        // 6
        (Same(_), Same(_)) => return None,
        // 2, 7
        (Changed(k), Absent | Same(_)) if base.is_none() || matches!(repo, Same(_)) => {
            Action::Copy {
                to: Side::Repo,
                content: content_of(k, base),
            }
        }
        // 3, 8
        (Absent | Same(_), Changed(r)) if base.is_none() || matches!(kb, Same(_)) => Action::Copy {
            to: Side::Kb,
            content: content_of(r, base),
        },
        // 11
        (Absent, Same(_)) => Action::Trash { side: Side::Repo },
        // 12
        (Same(_), Absent) => Action::Trash { side: Side::Kb },
        // 14, 15
        (Absent, Changed(_)) | (Changed(_), Absent) => {
            pending(IssueKind::DeleteModify, base, kb, repo)
        }
        // 4, 5, 9, 10
        (Changed(k), Changed(r)) => {
            let kb_content = content_of(k, base);
            if k.hash == r.hash {
                Action::RecordBase {
                    content: kb_content,
                }
            } else {
                let repo_content = content_of(r, base);
                let ancestor = base.map_or("", |b| b.content.as_str());
                match merge::three_way(ancestor, &kb_content, &repo_content) {
                    Some(content) => Action::Merge { content },
                    None => pending(IssueKind::Conflict, base, kb, repo),
                }
            }
        }
        // Unreachable combinations: `Same` implies a base, and every case
        // with a base and one `Changed` side is handled above.
        _ => return None,
    };
    Some(action)
}

/// Classification for one-way pairs (`S → T`).
fn classify_one_way(
    base: Option<&BaseEntry>,
    source: Side,
    s: Seen<'_>,
    t: Seen<'_>,
    target_changes: TargetChanges,
    kb: Seen<'_>,
    repo: Seen<'_>,
) -> Option<Action> {
    use Seen::{Absent, Changed, Same};
    let target = source.other();
    let protect = target_changes == TargetChanges::Protect;
    let divergence = || pending(IssueKind::Divergence, base, kb, repo);
    let copy_source = |observed: &Observed| Action::Copy {
        to: target,
        content: content_of(observed, base),
    };

    let action = match (s, t) {
        (Absent, Absent) => base.map(|_| Action::ForgetBase)?,
        (Same(_), Same(_)) => return None,
        // Created or changed in the source.
        (Changed(so), Absent) if base.is_none() => copy_source(so),
        (Changed(so), Same(_)) => copy_source(so),
        // Only in the target, never synced.
        (Absent, Changed(_)) if base.is_none() => {
            if protect {
                Action::NotInSource { side: target }
            } else {
                Action::Trash { side: target }
            }
        }
        // Deleted in the source, untouched in the target.
        (Absent, Same(_)) => Action::Trash { side: target },
        // Deleted in the source, changed in the target.
        (Absent, Changed(_)) => {
            if protect {
                divergence()
            } else {
                Action::Trash { side: target }
            }
        }
        // Both present and identical: just record it.
        (Changed(so), Changed(to)) if so.hash == to.hash => Action::RecordBase {
            content: content_of(so, base),
        },
        // Target changed or deleted while the source exists.
        (Same(so) | Changed(so), Changed(_) | Absent) => {
            if protect {
                divergence()
            } else {
                copy_source(so)
            }
        }
    };
    Some(action)
}

/// Replaces "deleted at P + created at Q with the same content" on one side
/// by a rename propagated to the other side.
fn detect_renames(
    direction: Direction,
    bases: &BTreeMap<EntryPath, BaseEntry>,
    entries: &mut BTreeMap<EntryPath, PlannedEntry>,
) {
    for renamed_on in [Side::Kb, Side::Repo] {
        if direction
            .source()
            .is_some_and(|source| source != renamed_on)
        {
            continue;
        }
        let other = renamed_on.other();
        let mut gone: HashMap<ContentHash, Vec<EntryPath>> = HashMap::new();
        let mut born: HashMap<ContentHash, Vec<(EntryPath, String)>> = HashMap::new();
        for entry in entries.values() {
            match &entry.action {
                Action::Trash { side } if *side == other => {
                    // Only a file left untouched on the other side can follow the rename.
                    if let Some(base) = bases
                        .get(&entry.path)
                        .filter(|base| entry.seen(other) == Some(&base.hash))
                    {
                        gone.entry(base.hash.clone())
                            .or_default()
                            .push(entry.path.clone());
                    }
                }
                Action::Copy { to, content }
                    if *to == other
                        && !bases.contains_key(&entry.path)
                        && entry.seen(other).is_none() =>
                {
                    if let Some(hash) = entry.seen(renamed_on) {
                        born.entry(hash.clone())
                            .or_default()
                            .push((entry.path.clone(), content.clone()));
                    }
                }
                _ => {}
            }
        }
        for (hash, from) in gone {
            let Some(to) = born.get(&hash) else { continue };
            // Only unambiguous one-to-one matches count as renames.
            let ([from], [(to, content)]) = (from.as_slice(), to.as_slice()) else {
                continue;
            };
            entries.remove(to);
            if let Some(entry) = entries.get_mut(from) {
                entry.action = Action::Rename {
                    side: other,
                    to: to.clone(),
                    content: content.clone(),
                };
            }
        }
    }
}
