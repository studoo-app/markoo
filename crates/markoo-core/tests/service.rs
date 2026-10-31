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
            settings: Vec::new(),
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

#[test]
fn settings_of_a_new_pair_are_written_with_it() {
    let env = Env::new();
    let mut new = env.new_pair();
    new.settings = vec![("exclude".to_owned(), "drafts/**".to_owned())];
    markoo_core::manage::add_pair(&env.paths, &new).unwrap();
    let config = service::pair_config(&env.paths, "clara").unwrap();
    assert_eq!(config["exclude"][0], "drafts/**");
}

#[test]
fn a_filter_preview_separates_tracked_and_excluded_files() {
    let env = Env::new().with_saved_pair();
    std::fs::create_dir_all(env.kb().join("drafts")).unwrap();
    std::fs::write(env.kb().join("note.md"), "a\n").unwrap();
    std::fs::write(env.kb().join("drafts/idea.md"), "b\n").unwrap();
    std::fs::write(env.kb().join("image.png"), "c").unwrap();
    let view = service::preview_filters(
        &env.paths,
        "clara",
        &settings(&json!({"exclude": ["drafts/**"]})),
    )
    .unwrap();
    assert_eq!(view["tracked"], json!(["note.md"]));
    assert_eq!(view["excluded"], json!(["drafts/idea.md"]));
}

#[test]
fn a_filter_preview_leaves_out_what_is_always_ignored() {
    let env = Env::new().with_saved_pair();
    std::fs::write(env.repo().join(".git/config.md"), "x\n").unwrap();
    let view = service::preview_filters(&env.paths, "clara", &Map::new()).unwrap();
    assert_eq!(view["excluded_count"], 0);
    assert_eq!(view["tracked_count"], 0);
}

#[test]
fn a_filter_preview_saves_nothing() {
    let env = Env::new().with_saved_pair();
    let before = env.config_text();
    service::preview_filters(
        &env.paths,
        "clara",
        &settings(&json!({"exclude": ["x/**"]})),
    )
    .unwrap();
    assert_eq!(env.config_text(), before);
}

#[test]
fn an_invalid_pattern_is_an_error() {
    let env = Env::new().with_saved_pair();
    let result =
        service::preview_filters(&env.paths, "clara", &settings(&json!({"include": ["a{b"]})));
    assert!(result.is_err());
}

/// A vault with a note whose wikilink the conversion changes.
fn vault() -> Env {
    let env = Env::new();
    std::fs::create_dir_all(env.kb().join(".obsidian")).unwrap();
    std::fs::write(env.kb().join("a-plain.md"), "plain\n").unwrap();
    std::fs::write(env.kb().join("b-link.md"), "see [[a-plain]]\n").unwrap();
    env.with_saved_pair()
}

#[test]
fn a_sample_prefers_a_note_that_changes() {
    let env = vault();
    let sample = service::preview_transform(&env.paths, "clara", &Map::new()).unwrap();
    assert_eq!(sample["path"], "b-link.md");
    assert_eq!(sample["before"], "see [[a-plain]]\n");
    assert!(
        sample["after"].as_str().unwrap().contains("(./a-plain.md)"),
        "{sample}"
    );
}

#[test]
fn a_sample_follows_the_settings_being_edited() {
    let env = vault();
    let sample = service::preview_transform(
        &env.paths,
        "clara",
        &settings(&json!({"transform.links": "keep"})),
    )
    .unwrap();
    // Nothing changes any more, so the first note is the sample.
    assert_eq!(sample["path"], "a-plain.md");
    assert_eq!(sample["before"], sample["after"]);
}

#[test]
fn a_pair_without_notes_has_no_sample() {
    let env = Env::new().with_saved_pair();
    let sample = service::preview_transform(&env.paths, "clara", &Map::new()).unwrap();
    assert!(sample.is_null());
}

#[test]
fn the_view_of_a_pair_gives_its_options() {
    let env = Env::new().with_saved_pair();
    let view = service::pair_config(&env.paths, "clara").unwrap();
    assert_eq!(view["links"], "markdown");
    assert_eq!(view["frontmatter"], "filter");
    assert_eq!(view["hooks"]["install"][0], "post-merge");
}

#[test]
fn global_settings_can_be_read_and_changed() {
    let env = Env::new().with_saved_pair();
    service::update_global_settings(
        &env.paths,
        &settings(&json!({"schedule.interval": "off", "api.port": "5000"})),
    )
    .unwrap();
    let view = service::global_settings(&env.paths).unwrap();
    assert_eq!(view["interval"], "off");
    assert_eq!(view["api_port"], 5000);
}

#[test]
fn an_invalid_interval_saves_nothing() {
    let env = Env::new().with_saved_pair();
    let before = env.config_text();
    let result = service::update_global_settings(
        &env.paths,
        &settings(&json!({"schedule.interval": "soon"})),
    );
    assert!(result.is_err());
    assert_eq!(env.config_text(), before);
}

#[test]
fn hooks_are_reported_once_installed() {
    let env = Env::new();
    std::fs::remove_dir_all(env.repo().join(".git")).unwrap();
    let git = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(env.repo())
        .status()
        .unwrap();
    assert!(git.success());
    let env = env.with_saved_pair();
    let program = std::path::Path::new("/bin/markoo");
    service::install_hooks(&env.paths, "clara", program).unwrap();
    let state = service::hooks_state(&env.paths, "clara").unwrap();
    assert_eq!(state["installed"], json!(["post-merge"]));
    service::uninstall_hooks(&env.paths, "clara", program).unwrap();
    let state = service::hooks_state(&env.paths, "clara").unwrap();
    assert_eq!(state["installed"], json!([]));
}
