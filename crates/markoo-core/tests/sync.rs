//! End-to-end sync scenarios on an in-memory file system.
//!
//! Row numbers refer to the classification table of the sync algorithm design doc.

// Test helpers panic on failure: that is what fails the test.
#![expect(clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use markoo_core::fs::{Fs, MemFs};
use markoo_core::state::{IssueKind, State};
use markoo_core::{Direction, Engine, EntryPath, Pair, Report, Resolution, Side, TargetChanges};

struct Harness {
    engine: Engine<MemFs>,
    pair: Pair,
}

impl Harness {
    fn new() -> Self {
        Self::with(Direction::Both, TargetChanges::Protect)
    }

    fn with(direction: Direction, target_changes: TargetChanges) -> Self {
        let mut pair = Pair::new("test", "/kb", "/repo");
        pair.direction = direction;
        pair.target_changes = target_changes;
        let engine = Engine::new(MemFs::new(), State::open_in_memory().unwrap(), "/data");
        Self { engine, pair }
    }

    fn fs(&self) -> &MemFs {
        self.engine.fs()
    }

    fn root(side: Side) -> &'static str {
        match side {
            Side::Kb => "/kb",
            Side::Repo => "/repo",
        }
    }

    fn put(&self, side: Side, path: &str, text: &str) {
        let file = Path::new(Self::root(side)).join(path);
        self.fs().write_atomic(&file, text.as_bytes()).unwrap();
    }

    /// Simulates the user deleting a file (moved out of the pair).
    fn delete(&self, side: Side, path: &str) {
        let file = Path::new(Self::root(side)).join(path);
        let away = Path::new("/elsewhere").join(side.as_str()).join(path);
        self.fs().rename(&file, &away).unwrap();
    }

    fn get(&self, side: Side, path: &str) -> Option<String> {
        self.fs()
            .read_string(Path::new(Self::root(side)).join(path))
    }

    fn sync(&self) -> Report {
        self.engine.sync(&self.pair).unwrap()
    }

    /// Creates `path` with `text` on both sides and syncs, so it has a base.
    fn synced(&self, path: &str, text: &str) {
        self.put(Side::Kb, path, text);
        self.put(Side::Repo, path, text);
        self.sync();
    }

    fn pending_kinds(&self) -> Vec<IssueKind> {
        self.engine
            .issues(&self.pair)
            .unwrap()
            .into_iter()
            .map(|issue| issue.kind)
            .collect()
    }

    fn trashed(&self) -> Vec<PathBuf> {
        self.fs().files_under("/data/trash")
    }

    fn backups(&self) -> Vec<PathBuf> {
        self.fs().files_under("/data/backups")
    }
}

fn entry(path: &str) -> EntryPath {
    EntryPath::from_relative(Path::new(path)).unwrap()
}

const BASE: &str = "L1\nL2\nL3\nL4\n";

// --- Bidirectional: classification table -------------------------------

#[test]
fn row01_nothing_anywhere_plans_nothing() {
    let h = Harness::new();
    assert!(h.engine.plan(&h.pair).unwrap().plan.is_empty());
}

#[test]
fn row02_created_in_kb_is_copied_to_repo() {
    let h = Harness::new();
    h.put(Side::Kb, "a.md", "hello\n");
    h.sync();
    assert_eq!(h.get(Side::Repo, "a.md").as_deref(), Some("hello\n"));
}

#[test]
fn row03_created_in_repo_is_copied_to_kb() {
    let h = Harness::new();
    h.put(Side::Repo, "sub/a.md", "hello\n");
    h.sync();
    assert_eq!(h.get(Side::Kb, "sub/a.md").as_deref(), Some("hello\n"));
}

#[test]
fn row04_identical_creations_only_record_the_base() {
    let h = Harness::new();
    h.put(Side::Kb, "a.md", "same\n");
    h.put(Side::Repo, "a.md", "same\n");
    assert!(h.sync().written.is_empty());
}

#[test]
fn row05_different_creations_conflict() {
    let h = Harness::new();
    h.put(Side::Kb, "a.md", "kb\n");
    h.put(Side::Repo, "a.md", "repo\n");
    h.sync();
    assert_eq!(h.pending_kinds(), vec![IssueKind::Conflict]);
}

#[test]
fn row05_conflicting_files_are_left_untouched() {
    let h = Harness::new();
    h.put(Side::Kb, "a.md", "kb\n");
    h.put(Side::Repo, "a.md", "repo\n");
    h.sync();
    assert_eq!(
        (h.get(Side::Kb, "a.md"), h.get(Side::Repo, "a.md")),
        (Some("kb\n".to_owned()), Some("repo\n".to_owned()))
    );
}

#[test]
fn row06_unchanged_entry_plans_nothing() {
    let h = Harness::new();
    h.synced("a.md", BASE);
    assert!(h.engine.plan(&h.pair).unwrap().plan.is_empty());
}

#[test]
fn row07_kb_change_is_copied_to_repo() {
    let h = Harness::new();
    h.synced("a.md", BASE);
    h.put(Side::Kb, "a.md", "changed\n");
    h.sync();
    assert_eq!(h.get(Side::Repo, "a.md").as_deref(), Some("changed\n"));
}

#[test]
fn row07_overwritten_repo_file_is_backed_up() {
    let h = Harness::new();
    h.synced("a.md", BASE);
    h.put(Side::Kb, "a.md", "changed\n");
    h.sync();
    assert_eq!(h.backups().len(), 1);
}

#[test]
fn row08_repo_change_is_copied_to_kb() {
    let h = Harness::new();
    h.synced("a.md", BASE);
    h.put(Side::Repo, "a.md", "changed\n");
    h.sync();
    assert_eq!(h.get(Side::Kb, "a.md").as_deref(), Some("changed\n"));
}

#[test]
fn row09_same_change_on_both_sides_writes_nothing() {
    let h = Harness::new();
    h.synced("a.md", BASE);
    h.put(Side::Kb, "a.md", "same\n");
    h.put(Side::Repo, "a.md", "same\n");
    assert!(h.sync().written.is_empty());
}

#[test]
fn row10_changes_on_different_lines_are_merged() {
    let h = Harness::new();
    h.synced("a.md", BASE);
    h.put(Side::Kb, "a.md", "L1\nL2'\nL3\nL4\n");
    h.put(Side::Repo, "a.md", "L1\nL2\nL3\nL4'\n");
    h.sync();
    let merged = Some("L1\nL2'\nL3\nL4'\n".to_owned());
    assert_eq!(
        (h.get(Side::Kb, "a.md"), h.get(Side::Repo, "a.md")),
        (merged.clone(), merged)
    );
}

#[test]
fn row10_changes_on_the_same_line_conflict() {
    let h = Harness::new();
    h.synced("a.md", BASE);
    h.put(Side::Kb, "a.md", "L1\nKB\nL3\nL4\n");
    h.put(Side::Repo, "a.md", "L1\nREPO\nL3\nL4\n");
    h.sync();
    assert_eq!(h.pending_kinds(), vec![IssueKind::Conflict]);
}

#[test]
fn row11_deletion_in_kb_trashes_the_repo_file() {
    let h = Harness::new();
    h.synced("a.md", BASE);
    h.delete(Side::Kb, "a.md");
    h.sync();
    assert_eq!(h.get(Side::Repo, "a.md"), None);
}

#[test]
fn row11_deleted_file_is_kept_in_the_trash() {
    let h = Harness::new();
    h.synced("a.md", BASE);
    h.delete(Side::Kb, "a.md");
    h.sync();
    assert_eq!(h.trashed().len(), 1);
}

#[test]
fn row12_deletion_in_repo_trashes_the_kb_file() {
    let h = Harness::new();
    h.synced("a.md", BASE);
    h.delete(Side::Repo, "a.md");
    h.sync();
    assert_eq!(h.get(Side::Kb, "a.md"), None);
}

#[test]
fn row13_deletion_on_both_sides_forgets_the_base() {
    let h = Harness::new();
    h.synced("a.md", BASE);
    h.delete(Side::Kb, "a.md");
    h.delete(Side::Repo, "a.md");
    h.sync();
    assert!(h.engine.plan(&h.pair).unwrap().plan.is_empty());
}

#[test]
fn row14_deleted_in_kb_and_changed_in_repo_is_a_conflict() {
    let h = Harness::new();
    h.synced("a.md", BASE);
    h.delete(Side::Kb, "a.md");
    h.put(Side::Repo, "a.md", "changed\n");
    h.sync();
    assert_eq!(h.pending_kinds(), vec![IssueKind::DeleteModify]);
}

#[test]
fn row15_changed_in_kb_and_deleted_in_repo_is_a_conflict() {
    let h = Harness::new();
    h.synced("a.md", BASE);
    h.put(Side::Kb, "a.md", "changed\n");
    h.delete(Side::Repo, "a.md");
    h.sync();
    assert_eq!(h.pending_kinds(), vec![IssueKind::DeleteModify]);
}

// --- Conflicts lifecycle ------------------------------------------------

#[test]
fn conflict_resolved_by_hand_is_cleared() {
    let h = Harness::new();
    h.synced("a.md", BASE);
    h.put(Side::Kb, "a.md", "L1\nKB\nL3\nL4\n");
    h.put(Side::Repo, "a.md", "L1\nREPO\nL3\nL4\n");
    h.sync();
    h.put(Side::Repo, "a.md", "L1\nKB\nL3\nL4\n");
    h.sync();
    assert!(h.pending_kinds().is_empty());
}

// --- Renames -------------------------------------------------------------

#[test]
fn rename_in_kb_is_propagated_to_repo() {
    let h = Harness::new();
    h.synced("old.md", BASE);
    h.delete(Side::Kb, "old.md");
    h.put(Side::Kb, "new/name.md", BASE);
    h.sync();
    assert_eq!(
        (
            h.get(Side::Repo, "old.md"),
            h.get(Side::Repo, "new/name.md")
        ),
        (None, Some(BASE.to_owned()))
    );
}

#[test]
fn rename_does_not_use_the_trash() {
    let h = Harness::new();
    h.synced("old.md", BASE);
    h.delete(Side::Repo, "old.md");
    h.put(Side::Repo, "renamed.md", BASE);
    h.sync();
    assert!(h.trashed().is_empty());
}

// --- Safety --------------------------------------------------------------

#[test]
fn second_sync_writes_nothing() {
    let h = Harness::new();
    h.put(Side::Kb, "a.md", "a\n");
    h.put(Side::Repo, "b/c.md", "c\n");
    h.sync();
    let before = h.fs().write_count();
    h.sync();
    assert_eq!(h.fs().write_count(), before);
}

#[test]
fn second_plan_is_empty() {
    let h = Harness::new();
    h.put(Side::Kb, "a.md", "a\n");
    h.sync();
    assert!(h.engine.plan(&h.pair).unwrap().plan.is_empty());
}

#[test]
fn dry_run_writes_nothing() {
    let h = Harness::new();
    h.put(Side::Kb, "a.md", "a\n");
    let before = h.fs().write_count();
    h.engine.plan(&h.pair).unwrap();
    assert_eq!(h.fs().write_count(), before);
}

#[test]
fn file_changed_between_plan_and_apply_is_skipped() {
    let h = Harness::new();
    h.synced("a.md", BASE);
    h.put(Side::Kb, "a.md", "first\n");
    let planned = h.engine.plan(&h.pair).unwrap();
    h.put(Side::Kb, "a.md", "typed meanwhile\n");
    let report = h.engine.apply(&h.pair, &planned.plan).unwrap();
    assert_eq!(report.skipped.len(), 1);
}

#[test]
fn skipped_entry_leaves_the_target_untouched() {
    let h = Harness::new();
    h.synced("a.md", BASE);
    h.put(Side::Kb, "a.md", "first\n");
    let planned = h.engine.plan(&h.pair).unwrap();
    h.put(Side::Repo, "a.md", "edited meanwhile\n");
    h.engine.apply(&h.pair, &planned.plan).unwrap();
    assert_eq!(
        h.get(Side::Repo, "a.md").as_deref(),
        Some("edited meanwhile\n")
    );
}

#[test]
fn crlf_only_difference_is_not_a_change() {
    let h = Harness::new();
    h.synced("a.md", "a\nb\n");
    h.put(Side::Kb, "a.md", "a\r\nb\r\n");
    assert!(h.engine.plan(&h.pair).unwrap().plan.is_empty());
}

#[test]
fn untracked_files_are_never_copied() {
    let h = Harness::new();
    h.put(Side::Kb, "image.png", "png");
    h.sync();
    assert_eq!(h.get(Side::Repo, "image.png"), None);
}

// --- One-way: KB → repo -------------------------------------------------

fn one_way(target_changes: TargetChanges) -> Harness {
    Harness::with(Direction::KbToRepo, target_changes)
}

#[test]
fn one_way_created_in_source_is_copied() {
    let h = one_way(TargetChanges::Protect);
    h.put(Side::Kb, "a.md", "a\n");
    h.sync();
    assert_eq!(h.get(Side::Repo, "a.md").as_deref(), Some("a\n"));
}

#[test]
fn one_way_changed_in_source_is_copied() {
    let h = one_way(TargetChanges::Protect);
    h.synced("a.md", BASE);
    h.put(Side::Kb, "a.md", "new\n");
    h.sync();
    assert_eq!(h.get(Side::Repo, "a.md").as_deref(), Some("new\n"));
}

#[test]
fn one_way_never_writes_to_the_source() {
    let h = one_way(TargetChanges::Overwrite);
    h.synced("a.md", BASE);
    h.put(Side::Repo, "a.md", "target edit\n");
    h.put(Side::Repo, "extra.md", "extra\n");
    let report = h.sync();
    assert!(report.written.iter().all(|file| file.side == Side::Repo));
}

#[test]
fn protect_created_in_target_is_left_alone() {
    let h = one_way(TargetChanges::Protect);
    h.put(Side::Repo, "extra.md", "extra\n");
    assert_eq!(h.sync().not_in_source, vec![entry("extra.md")]);
}

#[test]
fn overwrite_created_in_target_is_trashed() {
    let h = one_way(TargetChanges::Overwrite);
    h.put(Side::Repo, "extra.md", "extra\n");
    h.sync();
    assert_eq!(h.get(Side::Repo, "extra.md"), None);
}

#[test]
fn protect_changed_in_target_is_a_divergence() {
    let h = one_way(TargetChanges::Protect);
    h.synced("a.md", BASE);
    h.put(Side::Repo, "a.md", "target edit\n");
    h.sync();
    assert_eq!(h.pending_kinds(), vec![IssueKind::Divergence]);
}

#[test]
fn protect_divergence_keeps_the_target_edit() {
    let h = one_way(TargetChanges::Protect);
    h.synced("a.md", BASE);
    h.put(Side::Repo, "a.md", "target edit\n");
    h.sync();
    assert_eq!(h.get(Side::Repo, "a.md").as_deref(), Some("target edit\n"));
}

#[test]
fn overwrite_changed_in_target_is_restored() {
    let h = one_way(TargetChanges::Overwrite);
    h.synced("a.md", BASE);
    h.put(Side::Repo, "a.md", "target edit\n");
    h.sync();
    assert_eq!(h.get(Side::Repo, "a.md").as_deref(), Some(BASE));
}

#[test]
fn protect_changed_on_both_sides_is_a_divergence() {
    let h = one_way(TargetChanges::Protect);
    h.synced("a.md", BASE);
    h.put(Side::Kb, "a.md", "source edit\n");
    h.put(Side::Repo, "a.md", "target edit\n");
    h.sync();
    assert_eq!(h.pending_kinds(), vec![IssueKind::Divergence]);
}

#[test]
fn overwrite_changed_on_both_sides_takes_the_source() {
    let h = one_way(TargetChanges::Overwrite);
    h.synced("a.md", BASE);
    h.put(Side::Kb, "a.md", "source edit\n");
    h.put(Side::Repo, "a.md", "target edit\n");
    h.sync();
    assert_eq!(h.get(Side::Repo, "a.md").as_deref(), Some("source edit\n"));
}

#[test]
fn one_way_deleted_in_source_is_trashed_in_target() {
    let h = one_way(TargetChanges::Protect);
    h.synced("a.md", BASE);
    h.delete(Side::Kb, "a.md");
    h.sync();
    assert_eq!(h.get(Side::Repo, "a.md"), None);
}

#[test]
fn protect_deleted_in_source_but_changed_in_target_is_a_divergence() {
    let h = one_way(TargetChanges::Protect);
    h.synced("a.md", BASE);
    h.delete(Side::Kb, "a.md");
    h.put(Side::Repo, "a.md", "target edit\n");
    h.sync();
    assert_eq!(h.pending_kinds(), vec![IssueKind::Divergence]);
}

#[test]
fn protect_deleted_in_target_is_a_divergence() {
    let h = one_way(TargetChanges::Protect);
    h.synced("a.md", BASE);
    h.delete(Side::Repo, "a.md");
    h.sync();
    assert_eq!(h.pending_kinds(), vec![IssueKind::Divergence]);
}

#[test]
fn overwrite_deleted_in_target_is_recreated() {
    let h = one_way(TargetChanges::Overwrite);
    h.synced("a.md", BASE);
    h.delete(Side::Repo, "a.md");
    h.sync();
    assert_eq!(h.get(Side::Repo, "a.md").as_deref(), Some(BASE));
}

#[test]
fn one_way_divergence_is_idempotent() {
    let h = one_way(TargetChanges::Protect);
    h.synced("a.md", BASE);
    h.put(Side::Repo, "a.md", "target edit\n");
    h.sync();
    let before = h.fs().write_count();
    h.sync();
    assert_eq!(h.fs().write_count(), before);
}

// --- One-way: repo → KB -------------------------------------------------

#[test]
fn repo_to_kb_copies_repo_changes() {
    let h = Harness::with(Direction::RepoToKb, TargetChanges::Protect);
    h.synced("a.md", BASE);
    h.put(Side::Repo, "a.md", "from repo\n");
    h.sync();
    assert_eq!(h.get(Side::Kb, "a.md").as_deref(), Some("from repo\n"));
}

#[test]
fn repo_to_kb_protects_kb_edits() {
    let h = Harness::with(Direction::RepoToKb, TargetChanges::Protect);
    h.synced("a.md", BASE);
    h.put(Side::Kb, "a.md", "kb edit\n");
    h.sync();
    assert_eq!(h.pending_kinds(), vec![IssueKind::Divergence]);
}

#[test]
fn repo_to_kb_follows_the_one_way_table() {
    let (s, t) = (Side::Repo, Side::Kb);
    let run = |changes, arrange: &dyn Fn(&Harness)| {
        let h = Harness::with(Direction::RepoToKb, changes);
        h.synced("a.md", BASE);
        arrange(&h);
        h.sync();
        (h.get(t, "a.md"), h.pending_kinds())
    };
    let divergence = vec![IssueKind::Divergence];
    let base = Some(BASE.to_owned());
    let source_edit = Some("source\n".to_owned());
    let target_edit = Some("target\n".to_owned());

    use TargetChanges::{Overwrite, Protect};
    // Changed in S
    let edit_source = |h: &Harness| h.put(s, "a.md", "source\n");
    assert_eq!(run(Protect, &edit_source), (source_edit.clone(), vec![]));
    assert_eq!(run(Overwrite, &edit_source), (source_edit.clone(), vec![]));
    // Changed in T only
    let edit_target = |h: &Harness| h.put(t, "a.md", "target\n");
    assert_eq!(
        run(Protect, &edit_target),
        (target_edit.clone(), divergence.clone())
    );
    assert_eq!(run(Overwrite, &edit_target), (base.clone(), vec![]));
    // Changed on both sides
    let edit_both = |h: &Harness| {
        h.put(s, "a.md", "source\n");
        h.put(t, "a.md", "target\n");
    };
    assert_eq!(
        run(Protect, &edit_both),
        (target_edit.clone(), divergence.clone())
    );
    assert_eq!(run(Overwrite, &edit_both), (source_edit, vec![]));
    // Deleted in S
    let delete_source = |h: &Harness| h.delete(s, "a.md");
    assert_eq!(run(Protect, &delete_source), (None, vec![]));
    assert_eq!(run(Overwrite, &delete_source), (None, vec![]));
    // Deleted in T
    let delete_target = |h: &Harness| h.delete(t, "a.md");
    assert_eq!(run(Protect, &delete_target), (None, divergence));
    assert_eq!(run(Overwrite, &delete_target), (base, vec![]));
}

// --- Unreadable files ----------------------------------------------------

/// A corrupted file must never be mistaken for a deleted one.
#[test]
fn unreadable_file_does_not_trash_the_other_side() {
    let h = Harness::new();
    h.synced("a.md", BASE);
    h.fs()
        .write_atomic(Path::new("/kb/a.md"), &[0xff, 0xfe])
        .unwrap();
    h.sync();
    assert_eq!(h.get(Side::Repo, "a.md").as_deref(), Some(BASE));
}

#[test]
fn unreadable_file_is_reported() {
    let h = Harness::new();
    h.put(Side::Kb, "a.md", "ok\n");
    h.fs()
        .write_atomic(Path::new("/repo/a.md"), &[0xff, 0xfe])
        .unwrap();
    assert_eq!(h.engine.plan(&h.pair).unwrap().scan_issues.len(), 1);
}

#[test]
fn unreadable_file_is_not_overwritten() {
    let h = Harness::new();
    h.put(Side::Kb, "a.md", "ok\n");
    h.fs()
        .write_atomic(Path::new("/repo/a.md"), &[0xff, 0xfe])
        .unwrap();
    h.sync();
    assert_eq!(h.fs().read(Path::new("/repo/a.md")).unwrap(), [0xff, 0xfe]);
}

#[test]
fn unreadable_file_is_left_out_of_the_plan() {
    let h = Harness::new();
    h.synced("a.md", BASE);
    h.fs()
        .write_atomic(Path::new("/kb/a.md"), &[0xff, 0xfe])
        .unwrap();
    assert!(h.engine.plan(&h.pair).unwrap().plan.is_empty());
}

// --- Unavailable folders ---------------------------------------------------

/// An unplugged disk must not look like every file was deleted.
#[test]
fn offline_kb_folder_refuses_to_sync() {
    let h = Harness::new();
    h.synced("a.md", BASE);
    h.fs().set_offline("/kb");
    assert!(matches!(
        h.engine.sync(&h.pair),
        Err(markoo_core::Error::MissingRoot { side: Side::Kb, .. })
    ));
}

#[test]
fn offline_kb_folder_keeps_the_repo_files() {
    let h = Harness::new();
    h.synced("a.md", BASE);
    h.fs().set_offline("/kb");
    let _ = h.engine.sync(&h.pair);
    assert_eq!(h.get(Side::Repo, "a.md").as_deref(), Some(BASE));
}

#[test]
fn offline_repo_folder_keeps_the_kb_files() {
    let h = Harness::new();
    h.synced("a.md", BASE);
    h.fs().set_offline("/repo");
    let _ = h.engine.sync(&h.pair);
    assert_eq!(h.get(Side::Kb, "a.md").as_deref(), Some(BASE));
}

// --- Resolution -------------------------------------------------------------

fn conflicted() -> Harness {
    let h = Harness::new();
    h.synced("a.md", BASE);
    h.put(Side::Kb, "a.md", "L1\nKB\nL3\nL4\n");
    h.put(Side::Repo, "a.md", "L1\nREPO\nL3\nL4\n");
    h.sync();
    h
}

const KB_TEXT: &str = "L1\nKB\nL3\nL4\n";
const REPO_TEXT: &str = "L1\nREPO\nL3\nL4\n";

#[test]
fn resolve_keep_kb_copies_it_to_the_repo() {
    let h = conflicted();
    h.engine
        .resolve(&h.pair, &entry("a.md"), Resolution::Keep(Side::Kb))
        .unwrap();
    assert_eq!(h.get(Side::Repo, "a.md").as_deref(), Some(KB_TEXT));
}

#[test]
fn resolve_keep_repo_copies_it_to_the_kb() {
    let h = conflicted();
    h.engine
        .resolve(&h.pair, &entry("a.md"), Resolution::Keep(Side::Repo))
        .unwrap();
    assert_eq!(h.get(Side::Kb, "a.md").as_deref(), Some(REPO_TEXT));
}

#[test]
fn resolve_backs_up_the_discarded_version() {
    let h = conflicted();
    h.engine
        .resolve(&h.pair, &entry("a.md"), Resolution::Keep(Side::Kb))
        .unwrap();
    let saved = h
        .backups()
        .iter()
        .filter_map(|p| h.fs().read_string(p))
        .any(|t| t == REPO_TEXT);
    assert!(saved);
}

#[test]
fn resolve_clears_the_issue() {
    let h = conflicted();
    h.engine
        .resolve(&h.pair, &entry("a.md"), Resolution::Keep(Side::Kb))
        .unwrap();
    assert!(h.engine.issues(&h.pair).unwrap().is_empty());
}

#[test]
fn resolve_leaves_nothing_to_sync() {
    let h = conflicted();
    h.engine
        .resolve(&h.pair, &entry("a.md"), Resolution::Keep(Side::Kb))
        .unwrap();
    assert!(h.engine.plan(&h.pair).unwrap().plan.is_empty());
}

#[test]
fn resolve_one_issue_keeps_the_others() {
    let h = conflicted();
    h.synced("b.md", BASE);
    h.put(Side::Kb, "b.md", KB_TEXT);
    h.put(Side::Repo, "b.md", REPO_TEXT);
    h.sync();
    h.engine
        .resolve(&h.pair, &entry("a.md"), Resolution::Keep(Side::Kb))
        .unwrap();
    assert_eq!(h.pending_kinds(), vec![IssueKind::Conflict]);
}

#[test]
fn resolve_with_content_writes_both_sides() {
    let h = conflicted();
    h.engine
        .resolve(
            &h.pair,
            &entry("a.md"),
            Resolution::Content("merged\n".into()),
        )
        .unwrap();
    let merged = Some("merged\n".to_owned());
    assert_eq!(
        (h.get(Side::Kb, "a.md"), h.get(Side::Repo, "a.md")),
        (merged.clone(), merged)
    );
}

#[test]
fn resolve_keep_both_saves_the_repo_version_aside() {
    let h = conflicted();
    h.engine
        .resolve(&h.pair, &entry("a.md"), Resolution::KeepBoth)
        .unwrap();
    let copy = "a.conflict-repo.md";
    assert_eq!(
        (
            h.get(Side::Kb, copy),
            h.get(Side::Repo, copy),
            h.get(Side::Repo, "a.md")
        ),
        (
            Some(REPO_TEXT.to_owned()),
            Some(REPO_TEXT.to_owned()),
            Some(KB_TEXT.to_owned())
        )
    );
}

#[test]
fn resolve_keep_both_leaves_nothing_to_sync() {
    let h = conflicted();
    h.engine
        .resolve(&h.pair, &entry("a.md"), Resolution::KeepBoth)
        .unwrap();
    assert!(h.engine.plan(&h.pair).unwrap().plan.is_empty());
}

#[test]
fn resolve_keep_both_never_overwrites_an_existing_copy() {
    let h = conflicted();
    h.put(Side::Kb, "a.conflict-repo.md", "precious\n");
    let report = h
        .engine
        .resolve(&h.pair, &entry("a.md"), Resolution::KeepBoth)
        .unwrap();
    assert_eq!(
        (
            report.skipped.len(),
            h.get(Side::Kb, "a.conflict-repo.md").as_deref()
        ),
        (1, Some("precious\n"))
    );
}

#[test]
fn resolve_delete_modify_by_keeping_the_deletion() {
    let h = Harness::new();
    h.synced("a.md", BASE);
    h.delete(Side::Kb, "a.md");
    h.put(Side::Repo, "a.md", "changed\n");
    h.sync();
    h.engine
        .resolve(&h.pair, &entry("a.md"), Resolution::Keep(Side::Kb))
        .unwrap();
    assert_eq!((h.get(Side::Repo, "a.md"), h.trashed().len()), (None, 1));
}

#[test]
fn resolve_without_issue_is_an_error() {
    let h = Harness::new();
    h.synced("a.md", BASE);
    let result = h
        .engine
        .resolve(&h.pair, &entry("a.md"), Resolution::Keep(Side::Kb));
    assert!(matches!(result, Err(markoo_core::Error::NotPending(_))));
}

fn diverged() -> Harness {
    let h = one_way(TargetChanges::Protect);
    h.synced("a.md", BASE);
    h.put(Side::Repo, "a.md", "target edit\n");
    h.sync();
    h
}

#[test]
fn divergence_keep_source_restores_the_target() {
    let h = diverged();
    h.engine
        .resolve(&h.pair, &entry("a.md"), Resolution::Keep(Side::Kb))
        .unwrap();
    assert_eq!(h.get(Side::Repo, "a.md").as_deref(), Some(BASE));
}

#[test]
fn divergence_keep_target_excludes_the_file() {
    let h = diverged();
    h.engine
        .resolve(&h.pair, &entry("a.md"), Resolution::Keep(Side::Repo))
        .unwrap();
    h.put(Side::Kb, "a.md", "source moved on\n");
    h.sync();
    assert_eq!(h.get(Side::Repo, "a.md").as_deref(), Some("target edit\n"));
}

#[test]
fn excluded_file_comes_back_by_keeping_the_source() {
    let h = diverged();
    h.engine
        .resolve(&h.pair, &entry("a.md"), Resolution::Keep(Side::Repo))
        .unwrap();
    h.engine
        .resolve(&h.pair, &entry("a.md"), Resolution::Keep(Side::Kb))
        .unwrap();
    assert_eq!(h.get(Side::Repo, "a.md").as_deref(), Some(BASE));
}

#[test]
fn divergence_push_to_source_copies_the_target_edit() {
    let h = diverged();
    h.engine
        .resolve(&h.pair, &entry("a.md"), Resolution::PushToSource)
        .unwrap();
    assert_eq!(h.get(Side::Kb, "a.md").as_deref(), Some("target edit\n"));
}

#[test]
fn one_way_refuses_writing_merged_content_to_the_source() {
    let h = diverged();
    let result = h
        .engine
        .resolve(&h.pair, &entry("a.md"), Resolution::Content("x\n".into()));
    assert!(matches!(
        result,
        Err(markoo_core::Error::NeedsBidirectional)
    ));
}

// --- Changing direction -----------------------------------------------------

#[test]
fn run_records_the_direction_without_forcing_a_dry_run_the_first_time() {
    let h = Harness::new();
    h.put(Side::Kb, "a.md", "a\n");
    assert!(h.engine.run(&h.pair, false).unwrap().report.is_some());
}

#[test]
fn run_dry_run_writes_nothing() {
    let h = Harness::new();
    h.put(Side::Kb, "a.md", "a\n");
    h.engine.run(&h.pair, true).unwrap();
    assert_eq!(h.get(Side::Repo, "a.md"), None);
}

#[test]
fn changing_direction_forces_one_dry_run() {
    let mut h = Harness::new();
    h.synced("a.md", BASE);
    h.engine.run(&h.pair, false).unwrap();
    h.pair.direction = Direction::KbToRepo;
    h.put(Side::Kb, "a.md", "new\n");
    let outcome = h.engine.run(&h.pair, false).unwrap();
    assert_eq!(
        (
            outcome.direction_changed,
            outcome.report.is_none(),
            h.get(Side::Repo, "a.md").as_deref()
        ),
        (true, true, Some(BASE))
    );
}

#[test]
fn run_after_the_forced_dry_run_applies() {
    let mut h = Harness::new();
    h.synced("a.md", BASE);
    h.engine.run(&h.pair, false).unwrap();
    h.pair.direction = Direction::KbToRepo;
    h.put(Side::Kb, "a.md", "new\n");
    h.engine.run(&h.pair, false).unwrap();
    h.engine.run(&h.pair, false).unwrap();
    assert_eq!(h.get(Side::Repo, "a.md").as_deref(), Some("new\n"));
}

#[test]
fn switching_to_one_way_is_refused_with_pending_conflicts() {
    let mut h = conflicted();
    h.engine.run(&h.pair, false).unwrap();
    h.pair.direction = Direction::KbToRepo;
    assert!(matches!(
        h.engine.run(&h.pair, false),
        Err(markoo_core::Error::DirectionBlocked { pending: 1, .. })
    ));
}

// --- Diff --------------------------------------------------------------------

#[test]
fn diff_shows_differing_entries() {
    let h = Harness::new();
    h.put(Side::Kb, "a.md", "kb\n");
    h.put(Side::Repo, "a.md", "repo\n");
    let diffs = h.engine.diffs(&h.pair, None).unwrap();
    assert!(diffs[0].patch.contains("-repo") && diffs[0].patch.contains("+kb"));
}

#[test]
fn diff_ignores_identical_entries() {
    let h = Harness::new();
    h.synced("a.md", BASE);
    assert!(h.engine.diffs(&h.pair, None).unwrap().is_empty());
}

#[test]
fn diff_can_target_one_entry() {
    let h = Harness::new();
    h.put(Side::Kb, "a.md", "a\n");
    h.put(Side::Kb, "b.md", "b\n");
    assert_eq!(
        h.engine.diffs(&h.pair, Some(&entry("b.md"))).unwrap().len(),
        1
    );
}

#[test]
fn resolving_is_refused_while_the_pair_is_locked() {
    let t = Harness::new();
    t.put(Side::Kb, "a.md", "kb\n");
    t.put(Side::Repo, "a.md", "repo\n");
    t.sync();
    let _lock = t.engine.lock_pair(&t.pair.name).unwrap();
    assert!(matches!(
        t.engine
            .resolve(&t.pair, &entry("a.md"), Resolution::Keep(Side::Kb)),
        Err(markoo_core::Error::Busy(_))
    ));
}
