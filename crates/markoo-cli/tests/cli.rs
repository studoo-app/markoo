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

#[test]
fn token_rotate_creates_a_token_file() {
    let env = Env::with_pair();
    env.ok(&["token", "rotate"]);
    assert_eq!(
        std::fs::read_to_string(env.path("data/token"))
            .unwrap()
            .len(),
        64
    );
}

#[test]
fn token_rotate_replaces_the_token() {
    let env = Env::with_pair();
    env.ok(&["token", "rotate"]);
    let first = std::fs::read_to_string(env.path("data/token")).unwrap();
    env.ok(&["token", "rotate"]);
    assert_ne!(
        first,
        std::fs::read_to_string(env.path("data/token")).unwrap()
    );
}

/// Stops the daemon even when an assertion fails.
struct Running(std::process::Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn daemon_serves_the_api_and_syncs_on_startup() {
    use std::io::{BufRead, Read, Write};

    let env = Env::with_pair();
    std::fs::write(env.kb().join("note.md"), "hello\n").unwrap();
    let config = std::fs::read_to_string(env.config()).unwrap();
    std::fs::write(
        env.config(),
        format!("[schedule]\ninterval = \"off\"\n[api]\nport = 0\n\n{config}"),
    )
    .unwrap();
    let mut child = env
        .command(&["daemon"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let running = Running(child);
    let mut line = String::new();
    std::io::BufReader::new(stdout)
        .read_line(&mut line)
        .unwrap();
    let addr = line.trim().rsplit("http://").next().unwrap().to_owned();

    let mut stream = std::net::TcpStream::connect(&addr).unwrap();
    let token = std::fs::read_to_string(env.path("data/token")).unwrap();
    write!(
        stream,
        "GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\
         Authorization: Bearer {token}\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");

    let target = env.repo().join("note.md");
    for _ in 0..500 {
        if target.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(target.exists());
    drop(running);
}

#[test]
fn sync_reports_a_busy_pair() {
    let env = Env::with_pair();
    env.ok(&["sync"]);
    std::fs::create_dir_all(env.path("data/locks")).unwrap();
    let lock = std::fs::File::create(env.path("data/locks/clara.lock")).unwrap();
    lock.lock().unwrap();
    assert!(env.fails(&["sync"]).contains("busy"));
}

// Git hooks, in real repositories

/// Runs `git` in the pair's repository, away from the user's own settings.
fn git(env: &Env, args: &[&str]) -> Output {
    Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .current_dir(env.path("repo"))
        .env("GIT_CONFIG_GLOBAL", env.path("no-gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn git_ok(env: &Env, args: &[&str]) {
    let output = git(env, args);
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A pair whose repository is a real one, with a first commit.
fn env_with_git() -> Env {
    let env = Env::with_pair();
    std::fs::remove_dir_all(env.path("repo/.git")).unwrap();
    git_ok(&env, &["init", "-q"]);
    env.write(&env.path("repo"), "README", "hello\n");
    // Otherwise the pair's folder disappears on a branch where it is empty.
    env.write(&env.repo(), "keep.txt", "keep\n");
    git_ok(&env, &["add", "."]);
    git_ok(&env, &["commit", "-q", "-m", "first"]);
    env
}

/// Adds settings to the only pair of the file, which is its last table.
fn with_hooks(env: &Env, settings: &str) {
    let config = std::fs::read_to_string(env.config()).unwrap();
    std::fs::write(
        env.config(),
        format!("{config}\n[pair.hooks]\n{settings}\n"),
    )
    .unwrap();
}

fn hook_path(env: &Env, name: &str) -> PathBuf {
    env.path("repo/.git/hooks").join(name)
}

fn hook_text(env: &Env, name: &str) -> Option<String> {
    std::fs::read_to_string(hook_path(env, name)).ok()
}

#[test]
fn hook_install_creates_post_merge_by_default() {
    let env = env_with_git();
    let output = env.ok(&["hook", "install", "--pair", "clara"]);
    assert!(output.contains("post-merge: created"), "{output}");
    assert!(
        hook_text(&env, "post-merge")
            .unwrap()
            .contains("# >>> markoo clara")
    );
    assert_eq!(hook_text(&env, "post-commit"), None);
}

#[cfg(unix)]
#[test]
fn a_hook_is_executable() {
    use std::os::unix::fs::PermissionsExt;
    let env = env_with_git();
    env.ok(&["hook", "install", "--pair", "clara"]);
    let mode = std::fs::metadata(hook_path(&env, "post-merge"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o111, 0o111);
}

#[test]
fn hook_install_says_that_nothing_was_synced() {
    let env = env_with_git();
    env.write(&env.repo(), "api.md", "not yet\n");
    let output = env.ok(&["hook", "install", "--pair", "clara"]);
    assert!(output.contains("markoo sync --pair clara"), "{output}");
    assert_eq!(env.read(&env.kb(), "api.md"), None);
}

#[test]
fn a_snippet_comes_without_the_sync_reminder() {
    let env = env_with_git();
    env.write(&env.path("repo"), "lefthook.yml", "pre-commit:\n");
    let output = env.ok(&["hook", "install", "--pair", "clara"]);
    assert!(!output.contains("Nothing was synced"), "{output}");
}

#[test]
fn hook_install_twice_changes_nothing() {
    let env = env_with_git();
    env.ok(&["hook", "install", "--pair", "clara"]);
    let first = hook_text(&env, "post-merge");
    let output = env.ok(&["hook", "install", "--pair", "clara"]);
    assert!(output.contains("already in place"), "{output}");
    assert_eq!(hook_text(&env, "post-merge"), first);
}

#[test]
fn hook_uninstall_restores_a_hook_of_the_user() {
    let env = env_with_git();
    let mine = "#!/bin/sh\necho mine\n";
    std::fs::write(hook_path(&env, "post-merge"), mine).unwrap();
    make_executable(&hook_path(&env, "post-merge"));
    env.ok(&["hook", "install", "--pair", "clara"]);
    assert!(hook_text(&env, "post-merge").unwrap().starts_with(mine));
    env.ok(&["hook", "uninstall", "--pair", "clara"]);
    assert_eq!(hook_text(&env, "post-merge").unwrap(), mine);
}

fn make_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    #[cfg(not(unix))]
    let _ = path;
}

#[test]
fn hook_uninstall_removes_a_hook_markoo_created() {
    let env = env_with_git();
    env.ok(&["hook", "install", "--pair", "clara"]);
    env.ok(&["hook", "uninstall", "--pair", "clara"]);
    assert_eq!(hook_text(&env, "post-merge"), None);
}

#[test]
fn pair_remove_reports_the_hooks_left_behind() {
    let env = env_with_git();
    env.ok(&["hook", "install", "--pair", "clara"]);
    let output = env.run(&["pair", "remove", "clara"]);
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("post-merge still calls this pair"),
        "{stderr}"
    );
    assert!(hook_text(&env, "post-merge").is_some());
}

#[test]
fn pair_remove_is_silent_once_the_hooks_are_gone() {
    let env = env_with_git();
    env.ok(&["hook", "install", "--pair", "clara"]);
    env.ok(&["hook", "uninstall", "--pair", "clara"]);
    let output = env.run(&["pair", "remove", "clara"]);
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());
}

#[test]
fn pair_remove_works_without_a_repository() {
    let env = Env::with_pair();
    std::fs::remove_dir_all(env.path("repo")).unwrap();
    env.ok(&["pair", "remove", "clara"]);
}

#[test]
fn hook_install_gives_lefthook_a_snippet() {
    let env = env_with_git();
    env.write(&env.path("repo"), "lefthook.yml", "pre-commit:\n");
    let output = env.ok(&["hook", "install", "--pair", "clara"]);
    assert!(output.contains("markoo-clara:"), "{output}");
    assert_eq!(hook_text(&env, "post-merge"), None);
}

#[test]
fn hook_install_outside_a_repository_fails() {
    let env = Env::with_pair();
    std::fs::remove_dir_all(env.path("repo/.git")).unwrap();
    let stderr = env.fails(&["hook", "install", "--pair", "clara"]);
    assert!(stderr.contains("not a Git working tree"), "{stderr}");
}

#[test]
fn hook_install_reports_as_json() {
    let env = env_with_git();
    let changes = env.json(&["hook", "install", "--pair", "clara"]);
    assert_eq!(changes[0]["hook"], "post-merge");
    assert_eq!(changes[0]["status"], "created");
}

#[test]
fn hook_run_syncs_the_pair() {
    let env = env_with_git();
    env.write(&env.repo(), "api.md", "from the team\n");
    env.ok(&["hook", "run", "post-merge", "--pair", "clara", "0"]);
    assert_eq!(env.read(&env.kb(), "api.md").unwrap(), "from the team\n");
}

#[test]
fn hook_run_skips_another_branch() {
    let env = env_with_git();
    with_hooks(&env, "branches = ['release']");
    env.write(&env.repo(), "api.md", "from the team\n");
    env.ok(&["hook", "run", "post-merge", "--pair", "clara"]);
    assert_eq!(env.read(&env.kb(), "api.md"), None);
}

#[test]
fn hook_run_skips_a_rebase_under_way() {
    let env = env_with_git();
    std::fs::create_dir(env.path("repo/.git/rebase-merge")).unwrap();
    env.write(&env.repo(), "api.md", "half done\n");
    env.ok(&["hook", "run", "post-commit", "--pair", "clara"]);
    assert_eq!(env.read(&env.kb(), "api.md"), None);
}

#[test]
fn hook_run_logs_a_failure_and_succeeds() {
    let env = env_with_git();
    env.ok(&["hook", "run", "post-merge", "--pair", "nobody"]);
    let log = std::fs::read_to_string(env.path("data/hooks.log")).unwrap();
    assert!(log.contains("post-merge nobody: no pair named"), "{log}");
}

/// Makes both sides of `note.md` disagree, leaving a conflict pending.
fn pending_conflict(env: &Env) {
    env.write(&env.kb(), "note.md", "base\n");
    env.ok(&["sync"]);
    env.write(&env.kb(), "note.md", "from the kb\n");
    env.write(&env.repo(), "note.md", "from the repo\n");
    env.ok(&["sync"]);
}

#[test]
fn pre_commit_refuses_while_a_conflict_is_pending() {
    let env = env_with_git();
    pending_conflict(&env);
    let stderr = env.fails(&["hook", "run", "pre-commit", "--pair", "clara"]);
    assert!(stderr.contains("1 Markoo conflict(s) pending"), "{stderr}");
}

#[test]
fn pre_commit_accepts_without_conflicts() {
    let env = env_with_git();
    env.ok(&["hook", "run", "pre-commit", "--pair", "clara"]);
}

#[test]
fn pre_commit_lets_the_commit_through_when_markoo_fails() {
    let env = env_with_git();
    env.ok(&["hook", "run", "pre-commit", "--pair", "nobody"]);
}

// Git for Windows runs hooks too, but through its own shell and path rules.
#[cfg(unix)]
mod real_hooks {
    use super::*;

    fn wait_for(env: &Env, file: &str) -> Option<String> {
        for _ in 0..500 {
            if let Some(text) = env.read(&env.kb(), file) {
                return Some(text);
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        None
    }

    #[test]
    fn a_commit_reaches_the_kb_through_the_hook() {
        let env = env_with_git();
        with_hooks(&env, "install = ['post-commit']");
        env.ok(&["hook", "install", "--pair", "clara"]);
        env.write(&env.repo(), "api.md", "from a commit\n");
        git_ok(&env, &["add", "."]);
        git_ok(&env, &["commit", "-q", "-m", "docs"]);
        assert_eq!(wait_for(&env, "api.md").unwrap(), "from a commit\n");
    }

    #[test]
    fn a_merge_reaches_the_kb_through_the_hook() {
        let env = env_with_git();
        env.ok(&["hook", "install", "--pair", "clara"]);
        git_ok(&env, &["checkout", "-q", "-b", "team"]);
        env.write(&env.repo(), "api.md", "from the team\n");
        git_ok(&env, &["add", "."]);
        git_ok(&env, &["commit", "-q", "-m", "docs"]);
        git_ok(&env, &["checkout", "-q", "-"]);
        git_ok(&env, &["merge", "-q", "team"]);
        assert_eq!(wait_for(&env, "api.md").unwrap(), "from the team\n");
    }

    #[test]
    fn a_branch_switch_reaches_the_kb_through_the_hook() {
        let env = env_with_git();
        with_hooks(&env, "install = ['post-checkout']");
        git_ok(&env, &["checkout", "-q", "-b", "team"]);
        env.write(&env.repo(), "api.md", "from the team\n");
        git_ok(&env, &["add", "."]);
        git_ok(&env, &["commit", "-q", "-m", "docs"]);
        git_ok(&env, &["checkout", "-q", "-"]);
        env.ok(&["hook", "install", "--pair", "clara"]);
        git_ok(&env, &["checkout", "-q", "team"]);
        assert_eq!(wait_for(&env, "api.md").unwrap(), "from the team\n");
    }

    #[test]
    fn a_failing_sync_does_not_fail_the_commit() {
        let env = env_with_git();
        with_hooks(&env, "install = ['post-commit']");
        env.ok(&["hook", "install", "--pair", "clara"]);
        std::fs::write(env.config(), "not toml =").unwrap();
        env.write(&env.path("repo"), "other", "x\n");
        git_ok(&env, &["add", "."]);
        git_ok(&env, &["commit", "-q", "-m", "still works"]);
    }

    #[test]
    fn pre_commit_blocks_a_real_commit() {
        let env = env_with_git();
        with_hooks(&env, "install = ['pre-commit']");
        env.ok(&["hook", "install", "--pair", "clara"]);
        pending_conflict(&env);
        env.write(&env.path("repo"), "other", "x\n");
        git_ok(&env, &["add", "other"]);
        let output = git(&env, &["commit", "-q", "-m", "refused"]);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("conflict(s) pending"));
    }

    #[test]
    fn a_removed_binary_does_not_block_commits() {
        let env = env_with_git();
        with_hooks(&env, "install = ['pre-commit']");
        env.ok(&["hook", "install", "--pair", "clara"]);
        let text = hook_text(&env, "pre-commit").unwrap();
        let program = env!("CARGO_BIN_EXE_markoo");
        std::fs::write(
            hook_path(&env, "pre-commit"),
            text.replace(program, "/nowhere/markoo"),
        )
        .unwrap();
        env.write(&env.path("repo"), "other", "x\n");
        git_ok(&env, &["add", "other"]);
        git_ok(&env, &["commit", "-q", "-m", "fine"]);
    }

    #[test]
    fn a_disabled_hook_is_not_turned_on() {
        let env = env_with_git();
        std::fs::write(hook_path(&env, "post-merge"), "#!/bin/sh\necho mine\n").unwrap();
        let output = env.run(&["hook", "install", "--pair", "clara"]);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("not executable"), "{stderr}");
        assert_eq!(
            hook_text(&env, "post-merge").unwrap(),
            "#!/bin/sh\necho mine\n"
        );
    }

    #[test]
    fn a_linked_hook_stays_a_link() {
        let env = env_with_git();
        env.write(&env.path("repo"), "my-hook", "#!/bin/sh\necho mine\n");
        make_executable(&env.path("repo/my-hook"));
        std::os::unix::fs::symlink(env.path("repo/my-hook"), hook_path(&env, "post-merge"))
            .unwrap();
        env.ok(&["hook", "install", "--pair", "clara"]);
        assert!(hook_path(&env, "post-merge").is_symlink());
        assert_eq!(
            env.read(&env.path("repo"), "my-hook").unwrap(),
            "#!/bin/sh\necho mine\n"
        );
    }
}

#[test]
fn mcp_answers_on_stdio_and_writes_only_the_protocol() {
    use std::io::{BufRead, Write};

    let env = Env::with_pair();
    env.write(&env.kb(), "note.md", "hello\n");
    let mut child = env
        .command(&["mcp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let running = Running(child);
    let mut lines = std::io::BufReader::new(stdout).lines();
    let mut ask = |message: Value, answers: bool| -> Option<Value> {
        writeln!(stdin, "{message}").unwrap();
        answers.then(|| serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap())
    };
    let init = ask(
        serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "test", "version": "1"},
            },
        }),
        true,
    )
    .unwrap();
    assert_eq!(init["result"]["serverInfo"]["name"], "markoo");
    ask(
        serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        false,
    );
    let answer = ask(
        serde_json::json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": {"name": "sync_run", "arguments": {"pair": "clara"}},
        }),
        true,
    )
    .unwrap();
    assert_eq!(answer["result"]["isError"], false);
    assert_eq!(env.read(&env.repo(), "note.md").unwrap(), "hello\n");
    drop(running);
}

#[test]
fn a_sync_from_the_command_line_goes_to_the_history() {
    let env = Env::with_pair();
    env.write(&env.kb(), "note.md", "hello\n");
    env.ok(&["sync"]);
    let engine_paths = markoo_core::paths::Paths::resolve(&markoo_core::paths::Overrides {
        config_file: Some(env.config()),
        data_dir: Some(env.path("data")),
    })
    .unwrap();
    let history = markoo_core::service::history(&engine_paths, "clara", 1).unwrap();
    assert_eq!(history[0]["trigger"], "cli");
}
