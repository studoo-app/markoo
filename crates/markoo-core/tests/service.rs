//! Tests of what the remote interfaces and the app ask for.

// Test helpers panic on failure: that is what fails the test.
#![expect(clippy::unwrap_used)]

use std::path::PathBuf;

use markoo_core::config::NewPair;
use markoo_core::paths::{Overrides, Paths};
use markoo_core::service;
use serde_json::{Map, Value, json};

struct Env {
    dir: tempfile::TempDir,
    paths: Paths,
}

impl Env {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("kb")).unwrap();
        std::fs::create_dir_all(dir.path().join("repo/.git")).unwrap();
        let paths = Paths::resolve(&Overrides {
            config_file: Some(dir.path().join("markoo.toml")),
            data_dir: Some(dir.path().join("data")),
        })
        .unwrap();
        Self { dir, paths }
    }

    fn kb(&self) -> PathBuf {
        self.dir.path().join("kb")
    }

    fn repo(&self) -> PathBuf {
        self.dir.path().join("repo")
    }

    fn new_pair(&self) -> NewPair {
        NewPair {
            name: "clara".to_owned(),
            kb: self.kb().display().to_string(),
            kb_type: None,
            repo: self.repo().display().to_string(),
            direction: None,
            target_changes: None,
        }
    }

    fn config_text(&self) -> String {
        std::fs::read_to_string(self.paths.config_file()).unwrap_or_default()
    }

    fn with_saved_pair(self) -> Self {
        markoo_core::manage::add_pair(&self.paths, &self.new_pair()).unwrap();
        self
    }
}

fn settings(value: &Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

#[test]
fn previewing_a_new_pair_shows_the_plan() {
    let env = Env::new();
    std::fs::write(env.kb().join("note.md"), "hello\n").unwrap();
    let plan = service::preview_new_pair(&env.paths, &env.new_pair()).unwrap();
    assert_eq!(plan["entries"].as_array().unwrap().len(), 1);
}

#[test]
fn previewing_a_new_pair_saves_nothing() {
    let env = Env::new();
    service::preview_new_pair(&env.paths, &env.new_pair()).unwrap();
    assert!(!env.config_text().contains("clara"));
}

#[test]
fn previewing_a_new_pair_writes_no_file() {
    let env = Env::new();
    std::fs::write(env.kb().join("note.md"), "hello\n").unwrap();
    service::preview_new_pair(&env.paths, &env.new_pair()).unwrap();
    assert!(!env.repo().join("note.md").exists());
}

#[test]
fn previewing_a_new_pair_reports_a_missing_folder() {
    let env = Env::new();
    let mut new = env.new_pair();
    new.kb = env.dir.path().join("nowhere").display().to_string();
    assert!(service::preview_new_pair(&env.paths, &new).is_err());
}

#[test]
fn previewing_a_new_pair_refuses_dots_in_a_folder() {
    let env = Env::new();
    let mut new = env.new_pair();
    new.repo = "/tmp/../etc".to_owned();
    assert!(service::preview_new_pair(&env.paths, &new).is_err());
}

#[test]
fn previewing_a_change_saves_nothing() {
    let env = Env::new().with_saved_pair();
    let before = env.config_text();
    let plan = service::preview_config(
        &env.paths,
        "clara",
        &settings(&json!({"direction": "kb-to-repo"})),
    )
    .unwrap();
    assert_eq!(plan["direction"], "kb-to-repo");
    assert_eq!(env.config_text(), before);
}

#[test]
fn previewing_a_rename_is_refused() {
    let env = Env::new().with_saved_pair();
    let result = service::preview_config(&env.paths, "clara", &settings(&json!({"name": "x"})));
    assert!(result.is_err());
}

#[test]
fn a_git_folder_gives_its_branch() {
    let env = Env::new();
    std::fs::remove_dir_all(env.repo().join(".git")).unwrap();
    let git = std::process::Command::new("git")
        .args(["init", "-q", "-b", "trunk"])
        .current_dir(env.repo())
        .status()
        .unwrap();
    assert!(git.success());
    let info = service::inspect_repo(env.repo().to_str().unwrap());
    assert_eq!(info["git"], true);
    assert_eq!(info["branch"], "trunk");
}

#[test]
fn a_plain_folder_is_not_a_repository() {
    let env = Env::new();
    let info = service::inspect_repo(env.kb().to_str().unwrap());
    assert_eq!(info["exists"], true);
    assert_eq!(info["git"], false);
}

#[test]
fn a_vault_is_recognized() {
    let env = Env::new();
    std::fs::create_dir_all(env.kb().join(".obsidian")).unwrap();
    let info = service::inspect_kb(env.kb().to_str().unwrap());
    assert_eq!(info["kb_type"], "obsidian");
}

#[test]
fn a_missing_folder_is_reported() {
    let env = Env::new();
    let info = service::inspect_kb(env.dir.path().join("nowhere").to_str().unwrap());
    assert_eq!(info["exists"], false);
}

#[test]
fn a_conflict_draft_has_the_markers() {
    let env = Env::new().with_saved_pair();
    std::fs::write(env.kb().join("note.md"), "base\n").unwrap();
    service::run_sync(&env.paths, "clara", false).unwrap();
    std::fs::write(env.kb().join("note.md"), "from the kb\n").unwrap();
    std::fs::write(env.repo().join("note.md"), "from the repo\n").unwrap();
    service::run_sync(&env.paths, "clara", false).unwrap();
    let all = service::conflicts(&env.paths, None).unwrap();
    let id = all["conflicts"][0]["id"].as_str().unwrap();
    let draft = service::conflict_draft(&env.paths, id).unwrap();
    assert!(
        draft.contains("from the kb") && draft.contains("from the repo"),
        "{draft}"
    );
}
