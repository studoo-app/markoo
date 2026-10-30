//! End-to-end sync scenarios on an in-memory file system.
//!
//! Row numbers refer to the classification table of the sync algorithm design doc.

// Test helpers panic on failure: that is what fails the test.
#![expect(clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use markoo_core::fs::{Fs, MemFs};
use markoo_core::state::{IssueKind, State};
use markoo_core::{Direction, Engine, EntryPath, Pair, Report, Side, TargetChanges};

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
            .pending(&self.pair)
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
    assert!(report.written.iter().all(|(side, _)| *side == Side::Repo));
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
