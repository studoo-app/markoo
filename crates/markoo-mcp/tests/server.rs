//! Tests of the MCP server through a real client, over an in-memory connection.

// Test helpers panic on failure: that is what fails the test.
#![expect(clippy::unwrap_used)]

use std::path::PathBuf;
use std::sync::Arc;

use markoo_core::paths::{Overrides, Paths};
use markoo_mcp::{DirectSyncer, MarkooServer};
use rmcp::model::{CallToolRequestParams, ReadResourceRequestParams, ResourceContents};
use rmcp::service::{RoleClient, RunningService};
use rmcp::{ClientHandler, ServiceExt};
use serde_json::{Value, json};

#[derive(Clone, Default)]
struct TestClient;

impl ClientHandler for TestClient {}

struct Env {
    dir: tempfile::TempDir,
    paths: Paths,
}

impl Env {
    fn new(settings: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("kb")).unwrap();
        std::fs::create_dir_all(dir.path().join("repo/.git")).unwrap();
        let paths = Paths::resolve(&Overrides {
            config_file: Some(dir.path().join("markoo.toml")),
            data_dir: Some(dir.path().join("data")),
        })
        .unwrap();
        std::fs::write(
            paths.config_file(),
            format!(
                "{settings}\n[[pair]]\nname = \"clara\"\nkb = '{}'\nrepo = '{}'\n",
                dir.path().join("kb").display(),
                dir.path().join("repo").display(),
            ),
        )
        .unwrap();
        Self { dir, paths }
    }

    /// Puts `clara` in a project, as written in the file.
    fn in_project(self, project: &str) -> Self {
        let text = std::fs::read_to_string(self.paths.config_file()).unwrap();
        let text = text.replace(
            "name = \"clara\"\n",
            &format!("name = \"clara\"\nproject = \"{project}\"\n"),
        );
        std::fs::write(self.paths.config_file(), text).unwrap();
        self
    }

    /// Adds `docs` to the projects `Clara` and `Markoo`, with their own folders.
    fn with_two_docs(self) -> Self {
        let mut text = std::fs::read_to_string(self.paths.config_file()).unwrap();
        for (id, project) in [("clara-docs", "Clara"), ("markoo-docs", "Markoo")] {
            let kb = self.dir.path().join(format!("kb-{id}"));
            let repo = self.dir.path().join(format!("repo-{id}"));
            std::fs::create_dir_all(&kb).unwrap();
            std::fs::create_dir_all(repo.join(".git")).unwrap();
            text.push_str(&format!(
                "[[pair]]\nid = \"{id}\"\nname = \"docs\"\nproject = \"{project}\"\n\
                 kb = '{}'\nrepo = '{}'\n",
                kb.display(),
                repo.display(),
            ));
        }
        std::fs::write(self.paths.config_file(), text).unwrap();
        self
    }

    fn kb(&self) -> PathBuf {
        self.dir.path().join("kb")
    }

    fn repo(&self) -> PathBuf {
        self.dir.path().join("repo")
    }

    async fn connect(&self) -> RunningService<RoleClient, TestClient> {
        let (server_io, client_io) = tokio::io::duplex(1 << 20);
        let server = MarkooServer::new(
            self.paths.clone(),
            Arc::new(DirectSyncer::new(self.paths.clone())),
        );
        tokio::spawn(async move {
            let running = server.serve(server_io).await.unwrap();
            let _ = running.waiting().await;
        });
        TestClient.serve(client_io).await.unwrap()
    }

    /// `note.md` was changed on both sides since the last sync.
    async fn pending_conflict(&self, client: &Client) {
        std::fs::write(self.kb().join("note.md"), "base\n").unwrap();
        call(client, "sync_run", json!({"pair": "clara"})).await;
        std::fs::write(self.kb().join("note.md"), "from the kb\n").unwrap();
        std::fs::write(self.repo().join("note.md"), "from the repo\n").unwrap();
        call(client, "sync_run", json!({"pair": "clara"})).await;
    }
}

type Client = RunningService<RoleClient, TestClient>;

fn request(tool: &'static str, arguments: Value) -> CallToolRequestParams {
    let params = CallToolRequestParams::new(tool);
    match arguments {
        Value::Object(arguments) => params.with_arguments(arguments),
        _ => params,
    }
}

fn text_of(result: &rmcp::model::CallToolResult) -> String {
    result.content[0].as_text().unwrap().text.clone()
}

/// Calls a tool that should succeed and returns its JSON answer.
async fn call(client: &Client, tool: &'static str, arguments: Value) -> Value {
    let result = client.call_tool(request(tool, arguments)).await.unwrap();
    assert_ne!(result.is_error, Some(true), "{tool}: {}", text_of(&result));
    serde_json::from_str(&text_of(&result)).unwrap()
}

/// Calls a tool that should fail and returns its message.
async fn call_fails(client: &Client, tool: &'static str, arguments: Value) -> String {
    let result = client.call_tool(request(tool, arguments)).await.unwrap();
    assert_eq!(result.is_error, Some(true), "{tool}: {}", text_of(&result));
    text_of(&result)
}

#[tokio::test]
async fn every_documented_tool_is_listed() {
    let env = Env::new("");
    let client = env.connect().await;
    let tools = client.list_all_tools().await.unwrap();
    let mut names: Vec<_> = tools.iter().map(|tool| tool.name.as_ref()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "get_pair_config",
            "list_conflicts",
            "list_pairs",
            "resolve_conflict",
            "sync_diff",
            "sync_run",
            "sync_status",
            "update_pair_config",
        ]
    );
}

#[tokio::test]
async fn tools_that_write_are_marked_destructive() {
    let env = Env::new("");
    let client = env.connect().await;
    for tool in client.list_all_tools().await.unwrap() {
        let writes = matches!(
            tool.name.as_ref(),
            "sync_run" | "resolve_conflict" | "update_pair_config"
        );
        let annotations = tool.annotations.as_ref().unwrap();
        assert_eq!(
            annotations.destructive_hint,
            writes.then_some(true),
            "{}",
            tool.name
        );
        assert_eq!(annotations.read_only_hint, Some(!writes), "{}", tool.name);
    }
}

#[tokio::test]
async fn list_pairs_shows_the_pair() {
    let env = Env::new("");
    let client = env.connect().await;
    let pairs = call(&client, "list_pairs", json!({})).await;
    assert_eq!(pairs[0]["id"], "clara");
    assert_eq!(pairs[0]["name"], "clara");
    assert_eq!(pairs[0]["label"], "clara");
    assert_eq!(pairs[0]["pending"], 0);
}

#[tokio::test]
async fn pairs_with_the_same_name_are_told_apart_by_id_and_label() {
    let env = Env::new("").with_two_docs();
    let client = env.connect().await;
    let pairs = call(&client, "list_pairs", json!({})).await;
    assert_eq!(pairs[1]["id"], "clara-docs");
    assert_eq!(pairs[2]["label"], "Markoo/docs");
    let status = call(&client, "sync_status", json!({"pair": "Markoo/docs"})).await;
    assert_eq!(status["pair"], "markoo-docs");
}

#[tokio::test]
async fn an_ambiguous_pair_name_is_a_readable_failure() {
    let env = Env::new("").with_two_docs();
    let client = env.connect().await;
    for tool in ["sync_status", "sync_run"] {
        let message = call_fails(&client, tool, json!({"pair": "docs"})).await;
        assert!(message.contains("Clara/docs"), "{tool}: {message}");
    }
}

#[tokio::test]
async fn sync_run_reports_the_id_of_the_pair() {
    let env = Env::new("").with_two_docs();
    let client = env.connect().await;
    let result = call(&client, "sync_run", json!({"pair": "clara/DOCS"})).await;
    assert_eq!(result["pair"], "clara-docs");
}

#[tokio::test]
async fn sync_status_writes_nothing() {
    let env = Env::new("");
    std::fs::write(env.kb().join("note.md"), "hello\n").unwrap();
    let client = env.connect().await;
    let status = call(&client, "sync_status", json!({"pair": "clara"})).await;
    assert_eq!(status["entries"].as_array().unwrap().len(), 1);
    assert!(!env.repo().join("note.md").exists());
}

#[tokio::test]
async fn sync_status_covers_every_pair_by_default() {
    let env = Env::new("");
    let client = env.connect().await;
    let status = call(&client, "sync_status", json!({})).await;
    assert_eq!(status[0]["pair"], "clara");
}

#[tokio::test]
async fn sync_run_writes_and_reports() {
    let env = Env::new("");
    std::fs::write(env.kb().join("note.md"), "hello\n").unwrap();
    let client = env.connect().await;
    let result = call(&client, "sync_run", json!({"pair": "clara"})).await;
    assert_eq!(result["outcome"]["summary"]["kb_to_repo"], 1);
    assert!(env.repo().join("note.md").exists());
}

#[tokio::test]
async fn sync_run_dry_run_writes_nothing() {
    let env = Env::new("");
    std::fs::write(env.kb().join("note.md"), "hello\n").unwrap();
    let client = env.connect().await;
    call(
        &client,
        "sync_run",
        json!({"pair": "clara", "dry_run": true}),
    )
    .await;
    assert!(!env.repo().join("note.md").exists());
}

#[tokio::test]
async fn sync_run_covers_every_pair_by_default() {
    let env = Env::new("");
    std::fs::write(env.kb().join("note.md"), "hello\n").unwrap();
    let client = env.connect().await;
    let results = call(&client, "sync_run", json!({})).await;
    assert_eq!(results[0]["pair"], "clara");
    assert!(env.repo().join("note.md").exists());
}

#[tokio::test]
async fn sync_run_on_an_unknown_pair_is_a_readable_failure() {
    let env = Env::new("");
    let client = env.connect().await;
    let message = call_fails(&client, "sync_run", json!({"pair": "nobody"})).await;
    assert!(message.contains("no pair named"), "{message}");
}

#[tokio::test]
async fn list_pairs_shows_the_project_of_each_pair() {
    let env = Env::new("").in_project("Markoo");
    let client = env.connect().await;
    let pairs = call(&client, "list_pairs", json!({})).await;
    assert_eq!(pairs[0]["project"], "Markoo");
}

#[tokio::test]
async fn sync_status_covers_the_pairs_of_a_project() {
    let env = Env::new("").in_project("Markoo");
    let client = env.connect().await;
    let status = call(&client, "sync_status", json!({"project": "markoo"})).await;
    assert_eq!(status[0]["pair"], "clara");
    assert_eq!(status.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn sync_run_covers_the_pairs_of_a_project() {
    let env = Env::new("").in_project("Markoo");
    std::fs::write(env.kb().join("note.md"), "hello\n").unwrap();
    let client = env.connect().await;
    let results = call(&client, "sync_run", json!({"project": "Markoo"})).await;
    assert_eq!(results[0]["pair"], "clara");
    assert!(env.repo().join("note.md").exists());
}

#[tokio::test]
async fn an_unknown_project_is_a_readable_failure() {
    let env = Env::new("").in_project("Markoo");
    let client = env.connect().await;
    for tool in ["sync_run", "sync_status"] {
        let message = call_fails(&client, tool, json!({"project": "Other"})).await;
        assert!(message.contains("no project named"), "{message}");
    }
}

#[tokio::test]
async fn a_pair_and_a_project_cannot_be_given_together() {
    let env = Env::new("").in_project("Markoo");
    let client = env.connect().await;
    let arguments = json!({"pair": "clara", "project": "Markoo"});
    for tool in ["sync_run", "sync_status"] {
        let message = call_fails(&client, tool, arguments.clone()).await;
        assert!(message.contains("not both"), "{message}");
    }
    assert!(!env.repo().join("note.md").exists());
}

#[tokio::test]
async fn sync_diff_shows_the_change() {
    let env = Env::new("");
    std::fs::write(env.kb().join("note.md"), "hello\n").unwrap();
    let client = env.connect().await;
    let diff = call(
        &client,
        "sync_diff",
        json!({"pair": "clara", "path": "note.md"}),
    )
    .await;
    assert_eq!(diff["diffs"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn a_path_outside_the_pair_is_refused() {
    let env = Env::new("");
    let client = env.connect().await;
    let message = call_fails(
        &client,
        "sync_diff",
        json!({"pair": "clara", "path": "../secret.md"}),
    )
    .await;
    assert!(message.contains("must be relative"), "{message}");
}

#[tokio::test]
async fn list_conflicts_gives_all_three_versions() {
    let env = Env::new("");
    let client = env.connect().await;
    env.pending_conflict(&client).await;
    let all = call(&client, "list_conflicts", json!({"pair": "clara"})).await;
    let conflict = &all["conflicts"][0];
    assert_eq!(conflict["kb"], "from the kb\n");
    assert_eq!(conflict["base"], "base\n");
    assert_eq!(conflict["repo"], "from the repo\n");
}

#[tokio::test]
async fn a_sync_leaves_a_conflict_unresolved() {
    let env = Env::new("");
    let client = env.connect().await;
    env.pending_conflict(&client).await;
    assert_eq!(
        std::fs::read_to_string(env.kb().join("note.md")).unwrap(),
        "from the kb\n"
    );
    assert_eq!(
        std::fs::read_to_string(env.repo().join("note.md")).unwrap(),
        "from the repo\n"
    );
}

#[tokio::test]
async fn resolve_conflict_applies_the_content() {
    let env = Env::new("");
    let client = env.connect().await;
    env.pending_conflict(&client).await;
    let all = call(&client, "list_conflicts", json!({})).await;
    let id = all["conflicts"][0]["id"].clone();
    call(
        &client,
        "resolve_conflict",
        json!({"id": id, "content": "merged\n"}),
    )
    .await;
    for side in [env.kb(), env.repo()] {
        assert_eq!(
            std::fs::read_to_string(side.join("note.md")).unwrap(),
            "merged\n"
        );
    }
}

#[tokio::test]
async fn resolve_conflict_needs_a_single_way_of_settling() {
    let env = Env::new("");
    let client = env.connect().await;
    env.pending_conflict(&client).await;
    let all = call(&client, "list_conflicts", json!({})).await;
    let id = all["conflicts"][0]["id"].clone();
    let message = call_fails(
        &client,
        "resolve_conflict",
        json!({"id": id, "keep": "kb", "content": "x"}),
    )
    .await;
    assert!(message.contains("give one of"), "{message}");
}

#[tokio::test]
async fn an_unknown_conflict_is_a_readable_failure() {
    let env = Env::new("");
    let client = env.connect().await;
    let message = call_fails(
        &client,
        "resolve_conflict",
        json!({"id": "c_0", "keep": "kb"}),
    )
    .await;
    assert!(message.contains("no conflict or divergence"), "{message}");
}

#[tokio::test]
async fn get_pair_config_shows_the_settings() {
    let env = Env::new("");
    let client = env.connect().await;
    let config = call(&client, "get_pair_config", json!({"pair": "clara"})).await;
    assert_eq!(config["direction"], "both");
}

#[tokio::test]
async fn update_pair_config_is_disabled_by_default() {
    let env = Env::new("");
    let before = std::fs::read_to_string(env.paths.config_file()).unwrap();
    let client = env.connect().await;
    let message = call_fails(
        &client,
        "update_pair_config",
        json!({"pair": "clara", "changes": {"paused": true}}),
    )
    .await;
    assert!(message.contains("allow_config_write"), "{message}");
    assert_eq!(
        std::fs::read_to_string(env.paths.config_file()).unwrap(),
        before
    );
}

#[tokio::test]
async fn update_pair_config_works_once_allowed() {
    let env = Env::new("[mcp]\nallow_config_write = true");
    let client = env.connect().await;
    let result = call(
        &client,
        "update_pair_config",
        json!({"pair": "clara", "changes": {"paused": true}}),
    )
    .await;
    assert_eq!(result["pair"]["paused"], true);
}

#[tokio::test]
async fn update_pair_config_can_change_the_project() {
    let env = Env::new("[mcp]\nallow_config_write = true");
    let client = env.connect().await;
    let result = call(
        &client,
        "update_pair_config",
        json!({"pair": "clara", "changes": {"project": "Markoo"}}),
    )
    .await;
    assert_eq!(result["pair"]["project"], "Markoo");
}

#[tokio::test]
async fn update_pair_config_renames_a_pair_and_keeps_its_id() {
    let env = Env::new("[mcp]\nallow_config_write = true");
    let client = env.connect().await;
    let result = call(
        &client,
        "update_pair_config",
        json!({"pair": "clara", "changes": {"name": "Skill Claude (prod)"}}),
    )
    .await;
    assert_eq!(result["pair"]["id"], "clara");
    assert_eq!(result["pair"]["name"], "Skill Claude (prod)");
    let status = call(&client, "sync_status", json!({"pair": "clara"})).await;
    assert_eq!(status["pair"], "clara");
}

#[tokio::test]
async fn the_id_of_a_pair_cannot_be_changed() {
    let env = Env::new("[mcp]\nallow_config_write = true");
    let client = env.connect().await;
    let message = call_fails(
        &client,
        "update_pair_config",
        json!({"pair": "clara", "changes": {"id": "other"}}),
    )
    .await;
    assert!(message.contains("cannot be changed"), "{message}");
}

#[tokio::test]
async fn a_folder_with_dots_is_refused() {
    let env = Env::new("[mcp]\nallow_config_write = true");
    let client = env.connect().await;
    let message = call_fails(
        &client,
        "update_pair_config",
        json!({"pair": "clara", "changes": {"repo": "/tmp/../etc"}}),
    )
    .await;
    assert!(message.contains("without \"..\""), "{message}");
}

async fn read(client: &Client, uri: String) -> Result<Value, rmcp::ServiceError> {
    let result = client
        .read_resource(ReadResourceRequestParams::new(uri))
        .await?;
    let ResourceContents::TextResourceContents { text, .. } = &result.contents[0] else {
        panic!("expected text");
    };
    Ok(serde_json::from_str(text).unwrap())
}

#[tokio::test]
async fn the_status_resource_gives_the_plan() {
    let env = Env::new("");
    std::fs::write(env.kb().join("note.md"), "hello\n").unwrap();
    let client = env.connect().await;
    let status = read(&client, "markoo://pairs/clara/status".to_owned())
        .await
        .unwrap();
    assert_eq!(status["entries"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn the_conflict_resource_gives_all_three_versions() {
    let env = Env::new("");
    let client = env.connect().await;
    env.pending_conflict(&client).await;
    let all = call(&client, "list_conflicts", json!({})).await;
    let id = all["conflicts"][0]["id"].as_str().unwrap();
    let conflict = read(&client, format!("markoo://conflicts/{id}"))
        .await
        .unwrap();
    assert_eq!(conflict["base"], "base\n");
}

#[tokio::test]
async fn an_unknown_resource_is_not_found() {
    let env = Env::new("");
    let client = env.connect().await;
    assert!(
        read(&client, "markoo://pairs/nobody/status".to_owned())
            .await
            .is_err()
    );
    assert!(read(&client, "markoo://other".to_owned()).await.is_err());
}

#[tokio::test]
async fn resources_are_listed() {
    let env = Env::new("");
    let client = env.connect().await;
    env.pending_conflict(&client).await;
    let resources = client.list_all_resources().await.unwrap();
    let uris: Vec<_> = resources.iter().map(|r| r.uri.as_str()).collect();
    assert!(uris.contains(&"markoo://pairs/clara/status"), "{uris:?}");
    assert!(
        uris.iter()
            .any(|uri| uri.starts_with("markoo://conflicts/")),
        "{uris:?}"
    );
}

#[tokio::test]
async fn resource_templates_are_listed() {
    let env = Env::new("");
    let client = env.connect().await;
    assert_eq!(client.list_all_resource_templates().await.unwrap().len(), 2);
}
