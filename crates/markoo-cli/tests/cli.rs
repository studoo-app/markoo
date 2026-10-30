//! End-to-end tests of the `markoo` binary.

// Test helpers panic on failure: that is what fails the test.
#![expect(clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::Value;

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        let env = Self {
            dir: tempfile::tempdir().unwrap(),
        };
        std::fs::create_dir_all(env.kb()).unwrap();
        std::fs::create_dir_all(env.repo()).unwrap();
        std::fs::create_dir_all(env.path("repo/.git")).unwrap();
        env
    }

    /// A pair named `clara` between the `kb/` and `repo/docs/` folders.
    fn with_pair() -> Self {
        let env = Self::new();
        env.ok(&["init"]);
        env.ok(&[
            "pair",
            "add",
            "clara",
            "--kb",
            env.kb().to_str().unwrap(),
            "--repo",
            env.repo().to_str().unwrap(),
        ]);
        env
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.dir.path().join(relative)
    }

    fn kb(&self) -> PathBuf {
        self.path("kb")
    }

    fn repo(&self) -> PathBuf {
        self.path("repo/docs")
    }

    fn config(&self) -> PathBuf {
        self.path("markoo.toml")
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_markoo"));
        command
            .args(["--config", self.config().to_str().unwrap()])
            .args(["--data-dir", self.path("data").to_str().unwrap()])
            .args(args)
            .current_dir(self.dir.path())
            .env("NO_COLOR", "1")
            .env_remove("MARKOO_CONFIG")
            .env_remove("MARKOO_DATA_DIR")
            .env_remove("VISUAL")
            .env_remove("EDITOR")
            .stdin(Stdio::null());
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }

    fn ok(&self, args: &[&str]) -> String {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "markoo {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }

    fn fails(&self, args: &[&str]) -> String {
        let output = self.run(args);
        assert!(
            !output.status.success(),
            "markoo {args:?} should have failed"
        );
        String::from_utf8(output.stderr).unwrap()
    }

    fn json(&self, args: &[&str]) -> Value {
        let mut all = vec!["--json"];
        all.extend_from_slice(args);
        serde_json::from_str(&self.ok(&all)).unwrap()
    }

    fn write(&self, root: &Path, file: &str, text: &str) {
        let path = root.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn read(&self, root: &Path, file: &str) -> Option<String> {
        std::fs::read_to_string(root.join(file)).ok()
    }

    /// `a.md` modified on the same line on both sides, after a first sync.
    fn with_conflict() -> Self {
        let env = Self::with_pair();
        env.write(&env.kb(), "a.md", "L1\nL2\nL3\n");
        env.ok(&["sync"]);
        env.write(&env.kb(), "a.md", "L1\nKB\nL3\n");
        env.write(&env.repo(), "a.md", "L1\nREPO\nL3\n");
        env.ok(&["sync"]);
        env
    }
}

// --- init -------------------------------------------------------------------

#[test]
fn init_shows_the_banner() {
    let env = Env::new();
    assert!(env.ok(&["init"]).contains("|_|\\__,_|"));
}

#[test]
fn init_creates_the_configuration() {
    let env = Env::new();
    env.ok(&["init"]);
    assert!(env.config().exists());
}

#[test]
fn init_prepares_the_state_folder() {
    let env = Env::new();
    env.ok(&["init"]);
    assert!(env.path("data/state.db").exists());
}

#[test]
fn init_keeps_an_existing_configuration() {
    let env = Env::new();
    std::fs::write(env.config(), "# mine\n").unwrap();
    env.ok(&["init"]);
    assert_eq!(std::fs::read_to_string(env.config()).unwrap(), "# mine\n");
}

#[test]
fn version_shows_the_banner_and_the_number() {
    let output = Env::new().ok(&["--version"]);
    assert!(output.contains("|_|\\__,_|") && output.contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn no_color_keeps_the_banner_plain() {
    assert!(!Env::new().ok(&["--version"]).contains('\x1b'));
}

// --- pair --------------------------------------------------------------------

#[test]
fn pair_add_writes_the_configuration() {
    let env = Env::with_pair();
    assert_eq!(env.json(&["pair", "list"])[0]["name"], "clara");
}

#[test]
fn pair_add_refuses_a_missing_folder() {
    let env = Env::new();
    env.ok(&["init"]);
    let error = env.fails(&[
        "pair",
        "add",
        "x",
        "--kb",
        "/does/not/exist",
        "--repo",
        "/nope",
    ]);
    assert!(error.contains("not found"));
}

#[test]
fn refused_pair_is_not_saved() {
    let env = Env::new();
    env.ok(&["init"]);
    env.fails(&[
        "pair",
        "add",
        "x",
        "--kb",
        "/does/not/exist",
        "--repo",
        "/nope",
    ]);
    assert_eq!(env.json(&["pair", "list"]), Value::Array(vec![]));
}

#[test]
fn pair_add_refuses_a_repo_inside_the_kb() {
    let env = Env::new();
    std::fs::create_dir_all(env.kb().join("docs")).unwrap();
    let kb = env.kb();
    let inner = kb.join("docs");
    let error = env.fails(&[
        "pair",
        "add",
        "x",
        "--kb",
        kb.to_str().unwrap(),
        "--repo",
        inner.to_str().unwrap(),
    ]);
    assert!(error.contains("overlap"));
}

#[test]
fn pair_add_detects_an_obsidian_vault() {
    let env = Env::new();
    std::fs::create_dir_all(env.kb().join(".obsidian")).unwrap();
    let (kb, repo) = (env.kb(), env.repo());
    env.ok(&[
        "pair",
        "add",
        "x",
        "--kb",
        kb.to_str().unwrap(),
        "--repo",
        repo.to_str().unwrap(),
    ]);
    assert_eq!(env.json(&["pair", "show", "x"])["kb_type"], "obsidian");
}

#[test]
fn detected_type_is_written_to_the_file() {
    let env = Env::new();
    std::fs::create_dir_all(env.kb().join(".obsidian")).unwrap();
    let (kb, repo) = (env.kb(), env.repo());
    env.ok(&[
        "pair",
        "add",
        "x",
        "--kb",
        kb.to_str().unwrap(),
        "--repo",
        repo.to_str().unwrap(),
    ]);
    assert!(
        std::fs::read_to_string(env.config())
            .unwrap()
            .contains("kb_type = \"obsidian\"")
    );
}

#[test]
fn pair_add_accepts_an_obsidian_vault_as_explicit_markdown() {
    let env = Env::new();
    std::fs::create_dir_all(env.kb().join(".obsidian")).unwrap();
    let (kb, repo) = (env.kb(), env.repo());
    env.ok(&[
        "pair",
        "add",
        "x",
        "--kb",
        kb.to_str().unwrap(),
        "--kb-type",
        "markdown",
        "--repo",
        repo.to_str().unwrap(),
    ]);
    assert_eq!(env.json(&["pair", "show", "x"])["kb_type"], "markdown");
}

#[test]
fn pair_add_wizard_reads_the_answers() {
    use std::io::Write;
    let env = Env::new();
    let mut child = env
        .command(&["pair", "add"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let answers = format!(
        "notes\n{}\n{}\nkb-to-repo\n\n",
        env.kb().display(),
        env.repo().display()
    );
    child
        .stdin
        .take()
        .unwrap()
        .write_all(answers.as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success());
    assert_eq!(
        env.json(&["pair", "show", "notes"])["direction"],
        "kb-to-repo"
    );
}

#[test]
fn pair_set_preserves_comments() {
    let env = Env::with_pair();
    let text = std::fs::read_to_string(env.config()).unwrap();
    std::fs::write(env.config(), format!("# my own note\n{text}")).unwrap();
    env.ok(&["pair", "set", "clara", "direction", "kb-to-repo"]);
    assert!(
        std::fs::read_to_string(env.config())
            .unwrap()
            .contains("# my own note")
    );
}

#[test]
fn pair_set_with_target_changes_sets_both() {
    let env = Env::with_pair();
    env.ok(&[
        "pair",
        "set",
        "clara",
        "direction",
        "kb-to-repo",
        "--target-changes",
        "overwrite",
    ]);
    let pair = env.json(&["pair", "show", "clara"]);
    assert_eq!(
        (&pair["direction"], &pair["target_changes"]),
        (&"kb-to-repo".into(), &"overwrite".into())
    );
}

#[test]
fn pair_set_refuses_an_invalid_value() {
    let env = Env::with_pair();
    env.fails(&["pair", "set", "clara", "direction", "sideways"]);
    assert_eq!(env.json(&["pair", "show", "clara"])["direction"], "both");
}

#[test]
fn pair_set_refuses_one_way_with_pending_conflicts() {
    let env = Env::with_conflict();
    let error = env.fails(&["pair", "set", "clara", "direction", "kb-to-repo"]);
    assert!(error.contains("conflict"));
}

#[test]
fn pair_remove_keeps_the_files() {
    let env = Env::with_pair();
    env.write(&env.kb(), "a.md", "a\n");
    env.ok(&["sync"]);
    env.ok(&["pair", "remove", "clara"]);
    assert_eq!(env.read(&env.repo(), "a.md").as_deref(), Some("a\n"));
}

#[test]
fn readding_a_removed_pair_starts_from_a_clean_state() {
    let env = Env::with_pair();
    env.write(&env.kb(), "a.md", "a\n");
    env.ok(&["sync"]);
    env.ok(&["pair", "remove", "clara"]);
    std::fs::remove_file(env.kb().join("a.md")).unwrap();
    let (kb, repo) = (env.kb(), env.repo());
    env.ok(&[
        "pair",
        "add",
        "clara",
        "--kb",
        kb.to_str().unwrap(),
        "--repo",
        repo.to_str().unwrap(),
    ]);
    env.ok(&["sync"]);
    // Without a stale base, the file is a creation in the repo, not a deletion.
    assert_eq!(env.read(&env.kb(), "a.md").as_deref(), Some("a\n"));
}

// --- status, diff, sync ----------------------------------------------------------

#[test]
fn status_without_configuration_explains_what_to_do() {
    assert!(Env::new().fails(&["status"]).contains("markoo init"));
}

#[test]
fn status_writes_nothing() {
    let env = Env::with_pair();
    env.write(&env.kb(), "a.md", "a\n");
    env.ok(&["status"]);
    assert_eq!(env.read(&env.repo(), "a.md"), None);
}

#[test]
fn status_json_lists_the_entries() {
    let env = Env::with_pair();
    env.write(&env.kb(), "a.md", "a\n");
    let entry = &env.json(&["status"])[0]["entries"][0];
    assert_eq!(
        (&entry["path"], &entry["kind"], &entry["side"]),
        (&"a.md".into(), &"copy".into(), &"repo".into())
    );
}

#[test]
fn status_up_to_date_says_so() {
    assert!(Env::with_pair().ok(&["status"]).contains("Up to date"));
}

#[test]
fn sync_copies_the_files() {
    let env = Env::with_pair();
    env.write(&env.kb(), "sub/a.md", "a\n");
    env.ok(&["sync"]);
    assert_eq!(env.read(&env.repo(), "sub/a.md").as_deref(), Some("a\n"));
}

#[test]
fn dry_run_writes_nothing() {
    let env = Env::with_pair();
    env.write(&env.kb(), "a.md", "a\n");
    env.ok(&["sync", "--dry-run"]);
    assert_eq!(env.read(&env.repo(), "a.md"), None);
}

#[test]
fn sync_json_reports_what_was_written() {
    let env = Env::with_pair();
    env.write(&env.kb(), "a.md", "a\n");
    let written = &env.json(&["sync"])[0]["report"]["written"][0];
    assert_eq!(
        (&written["path"], &written["side"]),
        (&"a.md".into(), &"repo".into())
    );
}

#[test]
fn sync_can_target_one_pair() {
    let env = Env::with_pair();
    let other = env.path("other-kb");
    let other_repo = env.path("other-repo");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::create_dir_all(&other_repo).unwrap();
    env.ok(&[
        "pair",
        "add",
        "other",
        "--kb",
        other.to_str().unwrap(),
        "--repo",
        other_repo.to_str().unwrap(),
    ]);
    env.write(&env.kb(), "a.md", "a\n");
    env.write(&other, "b.md", "b\n");
    env.ok(&["sync", "--pair", "other"]);
    assert_eq!(
        (
            env.read(&other_repo, "b.md").is_some(),
            env.read(&env.repo(), "a.md").is_some()
        ),
        (true, false)
    );
}

#[test]
fn sync_of_an_unknown_pair_fails() {
    assert!(
        Env::with_pair()
            .fails(&["sync", "--pair", "nope"])
            .contains("nope")
    );
}

#[test]
fn paused_pair_is_skipped() {
    let env = Env::with_pair();
    env.ok(&["pair", "set", "clara", "paused", "true"]);
    env.write(&env.kb(), "a.md", "a\n");
    env.ok(&["sync"]);
    assert_eq!(env.read(&env.repo(), "a.md"), None);
}

/// An unavailable folder must not empty the other side.
#[test]
fn missing_folder_fails_and_keeps_the_other_side() {
    let env = Env::with_pair();
    env.write(&env.kb(), "a.md", "a\n");
    env.ok(&["sync"]);
    std::fs::rename(env.kb(), env.path("unplugged")).unwrap();
    assert!(env.fails(&["sync"]).contains("not found"));
    assert_eq!(env.read(&env.repo(), "a.md").as_deref(), Some("a\n"));
}

#[test]
fn a_broken_pair_does_not_stop_the_others() {
    let env = Env::with_pair();
    let other = env.path("other-kb");
    let other_repo = env.path("other-repo");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::create_dir_all(&other_repo).unwrap();
    env.ok(&[
        "pair",
        "add",
        "other",
        "--kb",
        other.to_str().unwrap(),
        "--repo",
        other_repo.to_str().unwrap(),
    ]);
    env.write(&other, "b.md", "b\n");
    std::fs::rename(env.kb(), env.path("unplugged")).unwrap();
    env.fails(&["sync"]);
    assert!(env.read(&other_repo, "b.md").is_some());
}

#[test]
fn changing_direction_only_shows_the_plan_first() {
    let env = Env::with_pair();
    env.write(&env.kb(), "a.md", "a\n");
    env.ok(&["sync"]);
    env.ok(&["pair", "set", "clara", "direction", "kb-to-repo"]);
    env.write(&env.kb(), "a.md", "changed\n");
    assert!(env.ok(&["sync"]).contains("direction changed"));
    assert_eq!(env.read(&env.repo(), "a.md").as_deref(), Some("a\n"));
    env.ok(&["sync"]);
    assert_eq!(env.read(&env.repo(), "a.md").as_deref(), Some("changed\n"));
}

#[test]
fn hand_edited_direction_is_also_guarded() {
    let env = Env::with_conflict();
    let text = std::fs::read_to_string(env.config()).unwrap();
    std::fs::write(env.config(), format!("{text}direction = \"kb-to-repo\"\n")).unwrap();
    assert!(env.fails(&["sync"]).contains("conflict"));
}

#[test]
fn diff_shows_both_versions() {
    let env = Env::with_pair();
    env.write(&env.kb(), "a.md", "kb\n");
    env.write(&env.repo(), "a.md", "repo\n");
    let diff = env.ok(&["diff"]);
    assert!(diff.contains("-repo") && diff.contains("+kb"));
}

#[test]
fn diff_names_the_sides() {
    let env = Env::with_pair();
    env.write(&env.kb(), "a.md", "kb\n");
    assert!(env.ok(&["diff", "a.md"]).contains("+++ kb/a.md"));
}

#[test]
fn diff_refuses_a_path_outside_the_pair() {
    assert!(
        Env::with_pair()
            .fails(&["diff", "../secret.md"])
            .contains("relative")
    );
}

// --- conflicts ---------------------------------------------------------------------

#[test]
fn conflicts_lists_the_file() {
    assert!(Env::with_conflict().ok(&["conflicts"]).contains("a.md"));
}

#[test]
fn conflicts_json_gives_both_versions() {
    let issue = &Env::with_conflict().json(&["conflicts"])[0];
    assert_eq!(
        (&issue["kb"], &issue["repo"]),
        (&"L1\nKB\nL3\n".into(), &"L1\nREPO\nL3\n".into())
    );
}

#[test]
fn conflicts_empty_says_so() {
    assert!(Env::with_pair().ok(&["conflicts"]).contains("No conflicts"));
}

#[test]
fn resolve_keep_kb_writes_the_repo() {
    let env = Env::with_conflict();
    env.ok(&["resolve", "a.md", "--keep", "kb"]);
    assert_eq!(
        env.read(&env.repo(), "a.md").as_deref(),
        Some("L1\nKB\nL3\n")
    );
}

#[test]
fn resolve_keep_both_creates_the_conflict_copy() {
    let env = Env::with_conflict();
    env.ok(&["resolve", "a.md", "--keep", "both"]);
    assert_eq!(
        env.read(&env.kb(), "a.conflict-repo.md").as_deref(),
        Some("L1\nREPO\nL3\n")
    );
}

#[test]
fn resolve_with_a_content_file() {
    let env = Env::with_conflict();
    let file = env.path("merged.txt");
    std::fs::write(&file, "merged\n").unwrap();
    env.ok(&["resolve", "a.md", "--content-file", file.to_str().unwrap()]);
    assert_eq!(env.read(&env.kb(), "a.md").as_deref(), Some("merged\n"));
}

#[test]
fn resolve_needs_a_decision() {
    let env = Env::with_conflict();
    env.fails(&["resolve", "a.md"]);
}

#[test]
fn resolve_without_an_issue_fails() {
    let env = Env::with_pair();
    assert!(
        env.fails(&["resolve", "a.md", "--keep", "kb"])
            .contains("no conflict")
    );
}

#[test]
fn resolve_failure_exits_non_zero_when_the_copy_exists() {
    let env = Env::with_conflict();
    env.write(&env.kb(), "a.conflict-repo.md", "precious\n");
    env.fails(&["resolve", "a.md", "--keep", "both"]);
    assert_eq!(
        env.read(&env.kb(), "a.conflict-repo.md").as_deref(),
        Some("precious\n")
    );
}

#[cfg(unix)]
mod editor {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn script(env: &Env, body: &str) -> String {
        let path = env.path("fake-editor.sh");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.display().to_string()
    }

    #[test]
    fn edit_uses_the_saved_text() {
        let env = Env::with_conflict();
        let editor = script(&env, "printf 'edited\\n' > \"$1\"");
        let output = env
            .command(&["resolve", "a.md", "--edit"])
            .env("EDITOR", editor)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(env.read(&env.repo(), "a.md").as_deref(), Some("edited\n"));
    }

    #[test]
    fn edit_refuses_remaining_markers() {
        let env = Env::with_conflict();
        let output = env
            .command(&["resolve", "a.md", "--edit"])
            .env("EDITOR", script(&env, "true"))
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&output.stderr).contains("markers"));
        assert_eq!(
            env.read(&env.repo(), "a.md").as_deref(),
            Some("L1\nREPO\nL3\n")
        );
    }

    #[test]
    fn edit_accepts_a_markdown_heading_underline() {
        let env = Env::with_conflict();
        let editor = script(&env, "printf 'Title\\n=======\\n' > \"$1\"");
        let output = env
            .command(&["resolve", "a.md", "--edit"])
            .env("EDITOR", editor)
            .output()
            .unwrap();
        assert!(output.status.success());
    }

    #[test]
    fn failing_editor_changes_nothing() {
        let env = Env::with_conflict();
        let output = env
            .command(&["resolve", "a.md", "--edit"])
            .env("EDITOR", script(&env, "exit 3"))
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert_eq!(env.read(&env.kb(), "a.md").as_deref(), Some("L1\nKB\nL3\n"));
    }
}

// --- paths -------------------------------------------------------------------------

#[test]
fn paths_reports_the_override() {
    let env = Env::new();
    assert_eq!(env.json(&["paths"])["data"]["source"], "argument");
}

// --- Obsidian -------------------------------------------------------------------------

fn vault() -> Env {
    let env = Env::new();
    std::fs::create_dir_all(env.kb().join(".obsidian")).unwrap();
    env.ok(&["init"]);
    let (kb, repo) = (env.kb(), env.repo());
    env.ok(&[
        "pair",
        "add",
        "clara",
        "--kb",
        kb.to_str().unwrap(),
        "--repo",
        repo.to_str().unwrap(),
    ]);
    env
}

#[test]
fn wikilinks_reach_the_repo_as_markdown_links() {
    let env = vault();
    env.write(&env.kb(), "a.md", "See [[b]].\n");
    env.write(&env.kb(), "b.md", "B\n");
    env.ok(&["sync"]);
    assert_eq!(
        env.read(&env.repo(), "a.md").as_deref(),
        Some("See [b](./b.md).\n")
    );
}

#[test]
fn repo_edit_reaches_the_vault_as_wikilinks() {
    let env = vault();
    env.write(&env.kb(), "a.md", "See [[b]].\n");
    env.write(&env.kb(), "b.md", "B\n");
    env.ok(&["sync"]);
    env.write(&env.repo(), "a.md", "Read [b](./b.md).\n");
    env.ok(&["sync"]);
    assert_eq!(
        env.read(&env.kb(), "a.md").as_deref(),
        Some("Read [[b]].\n")
    );
}

#[test]
fn obsidian_pair_settles_after_a_sync() {
    let env = vault();
    env.write(
        &env.kb(),
        "a.md",
        "---\naliases: [x]\n---\n[[b]] ![[pic.png]]\n\n> [!info]\n> Hi\n",
    );
    env.write(&env.kb(), "b.md", "B\n");
    env.write(&env.kb(), "pic.png", "png");
    env.ok(&["sync"]);
    assert!(env.ok(&["status"]).contains("Up to date"));
}

#[test]
fn attachments_are_copied_to_the_assets_folder() {
    let env = vault();
    env.write(&env.kb(), "a.md", "![[pic.png]]\n");
    env.write(&env.kb(), "pic.png", "png");
    env.ok(&["sync"]);
    assert_eq!(
        env.read(&env.repo(), "assets/pic.png").as_deref(),
        Some("png")
    );
}

#[test]
fn obsidian_settings_folder_is_never_synced() {
    let env = vault();
    env.write(&env.kb(), ".obsidian/notes.md", "secret\n");
    env.ok(&["sync"]);
    assert_eq!(env.read(&env.repo(), ".obsidian/notes.md"), None);
}

#[test]
fn untouched_vault_file_is_not_rewritten() {
    let env = vault();
    let text = "[[adr/b]] [x](b.md)\n%% note %%\n";
    env.write(&env.kb(), "a.md", text);
    env.write(&env.kb(), "adr/b.md", "B\n");
    env.ok(&["sync"]);
    env.ok(&["sync"]);
    assert_eq!(env.read(&env.kb(), "a.md").as_deref(), Some(text));
}
