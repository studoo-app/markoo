use super::*;
use crate::fs::MemFs;

const LINE: &str = "markoo hook run";

fn repo() -> GitRepo {
    GitRepo {
        top_level: PathBuf::from("/repo"),
        git_dir: PathBuf::from("/repo/.git"),
        hooks_dir: PathBuf::from("/repo/.git/hooks"),
        hooks_path: None,
        branch: Some("main".to_owned()),
    }
}

fn target() -> HookTarget<'static> {
    HookTarget {
        pair: "clara",
        program: Path::new("/bin/markoo"),
        config_file: Path::new("/cfg/markoo.toml"),
        data_dir: Path::new("/data"),
    }
}

fn settings(hooks: &[HookName]) -> HookSettings {
    HookSettings {
        install: hooks.to_vec(),
        branches: Vec::new(),
    }
}

fn hook_file(fs: &MemFs, name: &str) -> Option<String> {
    fs.read(&repo().hooks_dir.join(name))
        .ok()
        .map(|bytes| String::from_utf8(bytes).unwrap())
}

fn status_of(changes: &[HookChange], hook: HookName) -> &HookStatus {
    &changes.iter().find(|c| c.hook == hook).unwrap().status
}

fn arguments(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| (*v).to_owned()).collect()
}

// Editing the text of a hook

#[test]
fn a_new_hook_is_a_shell_script_with_the_block() {
    let text = with_block(None, "clara", LINE).unwrap().unwrap();
    assert_eq!(
        text,
        "#!/bin/sh\n# >>> markoo clara\nmarkoo hook run\n# <<< markoo clara\n"
    );
}

#[test]
fn the_block_goes_after_what_is_there() {
    let text = with_block(Some("#!/bin/bash\necho hi\n"), "clara", LINE)
        .unwrap()
        .unwrap();
    assert!(text.starts_with("#!/bin/bash\necho hi\n# >>> markoo clara\n"));
}

#[test]
fn removing_the_block_restores_the_file() {
    let original = "#!/bin/bash\necho hi\n";
    let added = with_block(Some(original), "clara", LINE).unwrap().unwrap();
    assert_eq!(without_block(&added, "clara").unwrap().unwrap(), original);
}

#[test]
fn a_file_without_a_final_newline_stays_valid() {
    let text = with_block(Some("#!/bin/sh\necho hi"), "clara", LINE)
        .unwrap()
        .unwrap();
    assert!(text.contains("echo hi\n# >>> markoo clara\n"));
}

#[test]
fn the_same_block_changes_nothing() {
    let text = with_block(None, "clara", LINE).unwrap().unwrap();
    assert_eq!(with_block(Some(&text), "clara", LINE).unwrap(), None);
}

#[test]
fn a_changed_block_is_replaced_in_place() {
    let text = format!("#!/bin/sh\n{}echo after\n", block("clara", "old"));
    let updated = with_block(Some(&text), "clara", LINE).unwrap().unwrap();
    assert_eq!(
        updated,
        format!("#!/bin/sh\n{}echo after\n", block("clara", LINE))
    );
}

#[test]
fn two_pairs_have_their_own_blocks() {
    let first = with_block(None, "clara", "one").unwrap().unwrap();
    let both = with_block(Some(&first), "markoo", "two").unwrap().unwrap();
    assert_eq!(without_block(&both, "markoo").unwrap().unwrap(), first);
}

#[test]
fn a_block_missing_its_end_is_refused() {
    let text = "#!/bin/sh\n# >>> markoo clara\nold\n";
    assert!(with_block(Some(text), "clara", LINE).is_err());
}

#[test]
fn a_python_hook_is_refused() {
    let text = "#!/usr/bin/env python3\nprint('hi')\n";
    assert!(with_block(Some(text), "clara", LINE).is_err());
}

#[test]
fn a_shell_found_through_env_is_accepted() {
    let text = "#!/usr/bin/env bash\necho hi\n";
    assert!(with_block(Some(text), "clara", LINE).is_ok());
}

#[test]
fn a_file_without_shebang_is_accepted() {
    assert!(with_block(Some("echo hi\n"), "clara", LINE).is_ok());
}

#[test]
fn a_hook_ending_with_exit_is_refused() {
    let text = "#!/bin/sh\necho hi\nexit 0\n\n# done\n";
    assert!(with_block(Some(text), "clara", LINE).is_err());
}

#[test]
fn a_hook_ending_with_exec_is_refused() {
    assert!(with_block(Some("#!/bin/sh\nexec other\n"), "clara", LINE).is_err());
}

#[test]
fn an_exit_inside_a_condition_is_accepted() {
    let text = "#!/bin/sh\nif false; then\n  exit 1\nfi\n";
    assert!(with_block(Some(text), "clara", LINE).is_ok());
}

// Lines of the hooks

#[test]
fn post_hooks_run_in_the_background() {
    let line = hook_line(HookName::PostMerge, &target());
    assert!(
        line.ends_with("\"$@\" </dev/null >/dev/null 2>&1 &"),
        "{line}"
    );
}

#[test]
fn pre_commit_can_block_the_commit() {
    let line = hook_line(HookName::PreCommit, &target());
    assert!(
        line.contains("hook run pre-commit --pair clara || exit 1"),
        "{line}"
    );
}

#[test]
fn paths_are_quoted_for_the_shell() {
    assert_eq!(quote(Path::new("/a b/it's")), r"'/a b/it'\''s'");
}

// Installing

#[test]
fn install_creates_the_hook() {
    let fs = MemFs::default();
    let changes = install(&fs, &repo(), &target(), &settings(&[HookName::PostMerge])).unwrap();
    assert_eq!(
        status_of(&changes, HookName::PostMerge),
        &HookStatus::Created
    );
    assert!(
        hook_file(&fs, "post-merge")
            .unwrap()
            .contains("hook run post-merge")
    );
}

#[test]
fn installing_again_writes_nothing() {
    let fs = MemFs::default();
    let wanted = settings(&[HookName::PostMerge]);
    install(&fs, &repo(), &target(), &wanted).unwrap();
    let writes = fs.write_count();
    let changes = install(&fs, &repo(), &target(), &wanted).unwrap();
    assert_eq!(
        status_of(&changes, HookName::PostMerge),
        &HookStatus::Unchanged
    );
    assert_eq!(fs.write_count(), writes);
}

#[test]
fn an_edited_hook_is_backed_up() {
    let fs = MemFs::default();
    let path = repo().hooks_dir.join("post-merge");
    fs.write_atomic(&path, b"#!/bin/sh\necho mine\n").unwrap();
    let changes = install(&fs, &repo(), &target(), &settings(&[HookName::PostMerge])).unwrap();
    assert_eq!(status_of(&changes, HookName::PostMerge), &HookStatus::Added);
    let kept = fs.list(Path::new("/data/backups/clara")).unwrap();
    assert_eq!(kept.len(), 1);
    assert_eq!(
        fs.read(&Path::new("/data/backups/clara").join(&kept[0]))
            .unwrap(),
        b"#!/bin/sh\necho mine\n"
    );
}

#[test]
fn a_binary_hook_is_left_alone() {
    let fs = MemFs::default();
    let path = repo().hooks_dir.join("post-merge");
    fs.write_atomic(&path, &[0xff, 0xfe, 0x00]).unwrap();
    let changes = install(&fs, &repo(), &target(), &settings(&[HookName::PostMerge])).unwrap();
    assert!(matches!(
        status_of(&changes, HookName::PostMerge),
        HookStatus::Manual { .. }
    ));
    assert_eq!(fs.read(&path).unwrap(), [0xff, 0xfe, 0x00]);
}

#[test]
fn a_hook_no_longer_asked_for_is_removed() {
    let fs = MemFs::default();
    install(&fs, &repo(), &target(), &settings(&[HookName::PostCommit])).unwrap();
    let changes = install(&fs, &repo(), &target(), &settings(&[HookName::PostMerge])).unwrap();
    assert_eq!(
        status_of(&changes, HookName::PostCommit),
        &HookStatus::Removed
    );
    assert_eq!(hook_file(&fs, "post-commit"), None);
}

// Removing

#[test]
fn uninstall_moves_a_hook_markoo_created_to_the_trash() {
    let fs = MemFs::default();
    let wanted = settings(&[HookName::PostMerge]);
    install(&fs, &repo(), &target(), &wanted).unwrap();
    uninstall(&fs, &repo(), &target(), &wanted).unwrap();
    assert_eq!(hook_file(&fs, "post-merge"), None);
    assert_eq!(fs.list(Path::new("/data/trash/clara")).unwrap().len(), 1);
}

#[test]
fn uninstall_keeps_what_the_user_wrote() {
    let fs = MemFs::default();
    let path = repo().hooks_dir.join("post-merge");
    fs.write_atomic(&path, b"#!/bin/sh\necho mine\n").unwrap();
    let wanted = settings(&[HookName::PostMerge]);
    install(&fs, &repo(), &target(), &wanted).unwrap();
    uninstall(&fs, &repo(), &target(), &wanted).unwrap();
    assert_eq!(
        hook_file(&fs, "post-merge").unwrap(),
        "#!/bin/sh\necho mine\n"
    );
}

#[test]
fn uninstall_leaves_the_other_pair_alone() {
    let fs = MemFs::default();
    let wanted = settings(&[HookName::PostMerge]);
    let other = HookTarget {
        pair: "markoo",
        ..target()
    };
    install(&fs, &repo(), &target(), &wanted).unwrap();
    install(&fs, &repo(), &other, &wanted).unwrap();
    uninstall(&fs, &repo(), &target(), &wanted).unwrap();
    let text = hook_file(&fs, "post-merge").unwrap();
    assert!(
        text.contains("markoo markoo") && !text.contains("markoo clara"),
        "{text}"
    );
}

#[test]
fn uninstall_without_hooks_changes_nothing() {
    let fs = MemFs::default();
    let changes = uninstall(&fs, &repo(), &target(), &settings(&[HookName::PostMerge])).unwrap();
    assert!(changes.is_empty());
}

// Hook managers

#[test]
fn husky_is_detected_by_its_folder() {
    let fs = MemFs::default();
    fs.write_atomic(Path::new("/repo/.husky/pre-commit"), b"x")
        .unwrap();
    assert_eq!(
        detect_manager(&fs, &repo()).unwrap(),
        Some(HookManager::Husky)
    );
}

#[test]
fn husky_is_detected_by_the_hooks_path() {
    let fs = MemFs::default();
    let mut repo = repo();
    repo.hooks_path = Some(".husky/_".to_owned());
    assert_eq!(
        detect_manager(&fs, &repo).unwrap(),
        Some(HookManager::Husky)
    );
}

#[test]
fn lefthook_is_detected_by_its_configuration() {
    let fs = MemFs::default();
    fs.write_atomic(Path::new("/repo/lefthook.yml"), b"x")
        .unwrap();
    assert_eq!(
        detect_manager(&fs, &repo()).unwrap(),
        Some(HookManager::Lefthook)
    );
}

#[test]
fn another_hooks_path_is_not_ours_to_edit() {
    let fs = MemFs::default();
    let mut repo = repo();
    repo.hooks_path = Some("~/hooks".to_owned());
    let changes = install(&fs, &repo, &target(), &settings(&[HookName::PostMerge])).unwrap();
    assert!(matches!(
        status_of(&changes, HookName::PostMerge),
        HookStatus::Manual { .. }
    ));
    assert_eq!(fs.write_count(), 0);
}

#[test]
fn a_manager_gets_a_snippet_and_no_file_is_written() {
    let fs = MemFs::default();
    fs.write_atomic(Path::new("/repo/lefthook.yml"), b"x")
        .unwrap();
    let writes = fs.write_count();
    let changes = install(&fs, &repo(), &target(), &settings(&[HookName::PostMerge])).unwrap();
    let HookStatus::Manual { snippet, .. } = status_of(&changes, HookName::PostMerge) else {
        panic!("expected a snippet");
    };
    assert!(
        snippet.starts_with("post-merge:\n  commands:\n    markoo-clara:\n"),
        "{snippet}"
    );
    assert_eq!(fs.write_count(), writes);
}

#[test]
fn a_snippet_names_no_file_of_this_machine() {
    let line = portable_line(HookName::PostMerge, "clara", "\"$@\"");
    assert!(
        !line.contains("/bin/markoo") && !line.contains("--config"),
        "{line}"
    );
}

#[test]
fn a_snippet_does_nothing_without_markoo() {
    let line = portable_line(HookName::PreCommit, "clara", "");
    assert!(line.starts_with("! command -v markoo"), "{line}");
}

// Deciding to act

#[test]
fn a_merge_acts() {
    let fs = MemFs::default();
    let acts = should_act(&fs, &repo(), HookName::PostMerge, &[], &settings(&[])).unwrap();
    assert!(acts);
}

#[test]
fn switching_branches_acts() {
    let fs = MemFs::default();
    let args = arguments(&["aaa", "bbb", "1"]);
    assert!(should_act(&fs, &repo(), HookName::PostCheckout, &args, &settings(&[])).unwrap());
}

#[test]
fn restoring_a_file_does_not_act() {
    let fs = MemFs::default();
    let args = arguments(&["aaa", "bbb", "0"]);
    assert!(!should_act(&fs, &repo(), HookName::PostCheckout, &args, &settings(&[])).unwrap());
}

#[test]
fn a_new_branch_on_the_same_commit_does_not_act() {
    let fs = MemFs::default();
    let args = arguments(&["aaa", "aaa", "1"]);
    assert!(!should_act(&fs, &repo(), HookName::PostCheckout, &args, &settings(&[])).unwrap());
}

#[test]
fn a_rebase_under_way_does_not_act() {
    let fs = MemFs::default();
    fs.write_atomic(Path::new("/repo/.git/rebase-merge/head-name"), b"x")
        .unwrap();
    assert!(!should_act(&fs, &repo(), HookName::PostCommit, &[], &settings(&[])).unwrap());
}

#[test]
fn a_cherry_pick_under_way_does_not_act() {
    let fs = MemFs::default();
    fs.write_atomic(Path::new("/repo/.git/CHERRY_PICK_HEAD"), b"x")
        .unwrap();
    assert!(!should_act(&fs, &repo(), HookName::PostCommit, &[], &settings(&[])).unwrap());
}

#[test]
fn a_listed_branch_acts() {
    let fs = MemFs::default();
    let mut wanted = settings(&[]);
    wanted.branches = vec!["main".to_owned()];
    assert!(should_act(&fs, &repo(), HookName::PostMerge, &[], &wanted).unwrap());
}

#[test]
fn another_branch_does_not_act() {
    let fs = MemFs::default();
    let mut wanted = settings(&[]);
    wanted.branches = vec!["release".to_owned()];
    assert!(!should_act(&fs, &repo(), HookName::PostMerge, &[], &wanted).unwrap());
}

#[test]
fn a_detached_head_does_not_match_a_branch_filter() {
    let fs = MemFs::default();
    let mut repo = repo();
    repo.branch = None;
    let mut wanted = settings(&[]);
    wanted.branches = vec!["main".to_owned()];
    assert!(!should_act(&fs, &repo, HookName::PostMerge, &[], &wanted).unwrap());
}
