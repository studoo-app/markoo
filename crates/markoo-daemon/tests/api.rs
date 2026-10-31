//! Tests of the daemon through a real local socket.

// Test helpers panic on failure: that is what fails the test.
#![expect(clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use markoo_core::Engine;
use markoo_core::fs::DiskFs;
use markoo_core::paths::{Overrides, Paths};
use markoo_daemon::{Daemon, Error, Options, token};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const QUIET: &str = "[schedule]\ninterval = \"off\"\non_startup = false\n[api]\nport = 0\n";

struct Env {
    dir: tempfile::TempDir,
    paths: Paths,
}

impl Env {
    fn with_settings(settings: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("kb")).unwrap();
        std::fs::create_dir_all(dir.path().join("repo/.git")).unwrap();
        let paths = Paths::resolve(&Overrides {
            config_file: Some(dir.path().join("markoo.toml")),
            data_dir: Some(dir.path().join("data")),
        })
        .unwrap();
        let env = Self { dir, paths };
        std::fs::write(
            env.paths.config_file(),
            format!("{settings}\n{}", env.pair_table("clara")),
        )
        .unwrap();
        env
    }

    fn new() -> Self {
        Self::with_settings(QUIET)
    }

    fn pair_table(&self, name: &str) -> String {
        format!(
            "[[pair]]\nname = \"{name}\"\nkb = '{}'\nrepo = '{}'\n",
            self.kb().display(),
            self.repo().display()
        )
    }

    fn kb(&self) -> PathBuf {
        self.dir.path().join("kb")
    }

    fn repo(&self) -> PathBuf {
        self.dir.path().join("repo")
    }

    async fn start(&self) -> Client {
        let daemon = Daemon::start(self.paths.clone(), fast()).await.unwrap();
        Client {
            addr: daemon.addr().unwrap(),
            token: token::ensure(self.paths.data_dir()).unwrap(),
            daemon: Some(daemon),
        }
    }

    /// `note.md` in sync, then changed differently on each side.
    async fn conflict(&self, client: &Client) {
        write(&self.kb().join("note.md"), "base\n");
        client.sync_and_wait(json!({})).await;
        write(&self.kb().join("note.md"), "from kb\n");
        write(&self.repo().join("note.md"), "from repo\n");
    }
}

fn fast() -> Options {
    Options {
        config_poll: Duration::from_millis(20),
    }
}

fn write(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
}

async fn eventually(what: &str, mut check: impl FnMut() -> bool) {
    for _ in 0..500 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{what} never happened");
}

struct Client {
    addr: std::net::SocketAddr,
    token: String,
    daemon: Option<Daemon>,
}

impl Client {
    async fn raw(
        &self,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: Option<&Value>,
    ) -> (u16, Value) {
        let mut stream = TcpStream::connect(self.addr).await.unwrap();
        let auth = token.map_or(String::new(), |t| format!("Authorization: Bearer {t}\r\n"));
        let body = body.map_or(String::new(), Value::to_string);
        let request = format!(
            "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n{auth}\
             Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).await.unwrap();
        let (head, body) = response.split_once("\r\n\r\n").unwrap();
        let status = head.split(' ').nth(1).unwrap().parse().unwrap();
        (status, serde_json::from_str(body).unwrap_or(Value::Null))
    }

    async fn get(&self, path: &str) -> (u16, Value) {
        self.raw("GET", path, Some(&self.token), None).await
    }

    async fn send(&self, method: &str, path: &str, body: Value) -> (u16, Value) {
        self.raw(method, path, Some(&self.token), Some(&body)).await
    }

    /// Starts a sync of `clara` and returns its finished job.
    async fn sync_and_wait(&self, body: Value) -> Value {
        let (status, accepted) = self.send("POST", "/pairs/clara/sync", body).await;
        assert_eq!(status, 202, "{accepted}");
        self.wait_job(accepted["job"].as_str().unwrap()).await
    }

    async fn wait_job(&self, id: &str) -> Value {
        for _ in 0..500 {
            let (_, job) = self.get(&format!("/jobs/{id}")).await;
            if job["status"] == "done" || job["status"] == "failed" {
                return job;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("job {id} never finished");
    }

    async fn stop(mut self) {
        self.daemon.take().unwrap().stop().await;
    }
}

#[tokio::test]
async fn api_listens_on_loopback_only() {
    let env = Env::new();
    let client = env.start().await;
    assert!(client.addr.ip().is_loopback());
    client.stop().await;
}

#[tokio::test]
async fn requests_without_a_token_are_refused() {
    let env = Env::new();
    let client = env.start().await;
    assert_eq!(client.raw("GET", "/health", None, None).await.0, 401);
    client.stop().await;
}

#[tokio::test]
async fn requests_with_a_wrong_token_are_refused() {
    let env = Env::new();
    let client = env.start().await;
    assert_eq!(client.raw("GET", "/pairs", Some("nope"), None).await.0, 401);
    client.stop().await;
}

#[tokio::test]
async fn rotating_the_token_applies_without_restart() {
    let env = Env::new();
    let client = env.start().await;
    let new = token::rotate(env.paths.data_dir()).unwrap();
    assert_eq!(client.get("/health").await.0, 401);
    assert_eq!(client.raw("GET", "/health", Some(&new), None).await.0, 200);
    client.stop().await;
}

#[tokio::test]
async fn health_reports_ok() {
    let env = Env::new();
    let client = env.start().await;
    assert_eq!(client.get("/health").await.1["status"], "ok");
    client.stop().await;
}

#[tokio::test]
async fn pairs_are_listed() {
    let env = Env::new();
    let client = env.start().await;
    let (_, pairs) = client.get("/pairs").await;
    assert_eq!(pairs[0]["name"], "clara");
    client.stop().await;
}

#[tokio::test]
async fn sync_copies_the_file_and_reports_it() {
    let env = Env::new();
    let client = env.start().await;
    write(&env.kb().join("note.md"), "hello\n");
    let job = client.sync_and_wait(json!({})).await;
    assert_eq!(job["outcome"]["summary"]["kb_to_repo"], 1);
    assert_eq!(
        std::fs::read_to_string(env.repo().join("note.md")).unwrap(),
        "hello\n"
    );
    client.stop().await;
}

#[tokio::test]
async fn sync_accepts_an_empty_body() {
    let env = Env::new();
    let client = env.start().await;
    let (status, _) = client
        .raw("POST", "/pairs/clara/sync", Some(&client.token), None)
        .await;
    assert_eq!(status, 202);
    client.stop().await;
}

#[tokio::test]
async fn second_sync_writes_nothing() {
    let env = Env::new();
    let client = env.start().await;
    write(&env.kb().join("note.md"), "hello\n");
    client.sync_and_wait(json!({})).await;
    let job = client.sync_and_wait(json!({})).await;
    assert_eq!(job["outcome"]["report"]["written"], json!([]));
    client.stop().await;
}

#[tokio::test]
async fn dry_run_writes_nothing() {
    let env = Env::new();
    let client = env.start().await;
    write(&env.kb().join("note.md"), "hello\n");
    let job = client.sync_and_wait(json!({"dry_run": true})).await;
    assert_eq!(job["outcome"]["summary"]["kb_to_repo"], 1);
    assert!(!env.repo().join("note.md").exists());
    client.stop().await;
}

#[tokio::test]
async fn sync_fails_cleanly_while_another_process_holds_the_lock() {
    let env = Env::new();
    let client = env.start().await;
    write(&env.kb().join("note.md"), "hello\n");
    let engine = Engine::open(DiskFs, &env.paths).unwrap();
    let lock = engine.lock_pair("clara").unwrap();
    let job = client.sync_and_wait(json!({})).await;
    assert!(job["error"].as_str().unwrap().contains("busy"), "{job}");
    assert!(!env.repo().join("note.md").exists());
    drop(lock);
    client.stop().await;
}

#[tokio::test]
async fn unknown_pair_is_not_found() {
    let env = Env::new();
    let client = env.start().await;
    assert_eq!(
        client.send("POST", "/pairs/nope/sync", json!({})).await.0,
        404
    );
    client.stop().await;
}

#[tokio::test]
async fn unknown_job_is_not_found() {
    let env = Env::new();
    let client = env.start().await;
    assert_eq!(client.get("/jobs/j_999").await.0, 404);
    client.stop().await;
}

#[tokio::test]
async fn paused_pair_refuses_to_sync() {
    let env = Env::new();
    let client = env.start().await;
    client
        .send("PATCH", "/pairs/clara/config", json!({"paused": true}))
        .await;
    assert_eq!(
        client.send("POST", "/pairs/clara/sync", json!({})).await.0,
        409
    );
    client.stop().await;
}

#[tokio::test]
async fn status_shows_the_plan_without_file_contents() {
    let env = Env::new();
    let client = env.start().await;
    write(&env.kb().join("note.md"), "secret text\n");
    let (_, plan) = client.get("/pairs/clara/status").await;
    assert_eq!(plan["entries"][0]["kind"], "copy");
    assert!(!plan.to_string().contains("secret text"));
    client.stop().await;
}

#[tokio::test]
async fn diff_returns_the_patch_of_one_file() {
    let env = Env::new();
    let client = env.start().await;
    write(&env.kb().join("note.md"), "kb\n");
    write(&env.repo().join("note.md"), "repo\n");
    let (_, diff) = client.get("/pairs/clara/diff?path=note.md").await;
    assert!(diff["diffs"][0]["patch"].as_str().unwrap().contains("+kb"));
    client.stop().await;
}

#[tokio::test]
async fn diff_refuses_paths_leaving_the_pair() {
    let env = Env::new();
    let client = env.start().await;
    for path in [
        "../secret.md",
        "..%2Fsecret.md",
        "/etc/passwd",
        "%2Fetc%2Fpasswd",
    ] {
        let (status, _) = client.get(&format!("/pairs/clara/diff?path={path}")).await;
        assert_eq!(status, 400, "{path}");
    }
    client.stop().await;
}

#[tokio::test]
async fn conflicts_are_listed_with_both_versions() {
    let env = Env::new();
    let client = env.start().await;
    env.conflict(&client).await;
    let (_, list) = client.get("/conflicts").await;
    assert_eq!(list["conflicts"][0]["kb"], "from kb\n");
    assert_eq!(list["conflicts"][0]["repo"], "from repo\n");
    client.stop().await;
}

#[tokio::test]
async fn a_conflict_is_reported_by_the_job_and_writes_nothing() {
    let env = Env::new();
    let client = env.start().await;
    env.conflict(&client).await;
    let job = client.sync_and_wait(json!({})).await;
    assert_eq!(job["outcome"]["conflicts"][0]["path"], "note.md");
    assert_eq!(
        std::fs::read_to_string(env.kb().join("note.md")).unwrap(),
        "from kb\n"
    );
    client.stop().await;
}

#[tokio::test]
async fn resolving_a_conflict_by_id_applies_the_choice() {
    let env = Env::new();
    let client = env.start().await;
    env.conflict(&client).await;
    let (_, list) = client.get("/conflicts").await;
    let id = list["conflicts"][0]["id"].as_str().unwrap();
    let path = format!("/conflicts/{id}/resolve");
    let (status, _) = client.send("POST", &path, json!({"keep": "repo"})).await;
    assert_eq!(status, 200);
    assert_eq!(
        std::fs::read_to_string(env.kb().join("note.md")).unwrap(),
        "from repo\n"
    );
    client.stop().await;
}

#[tokio::test]
async fn resolving_an_unknown_conflict_is_not_found() {
    let env = Env::new();
    let client = env.start().await;
    let (status, _) = client
        .send("POST", "/conflicts/c_0/resolve", json!({"keep": "kb"}))
        .await;
    assert_eq!(status, 404);
    client.stop().await;
}

#[tokio::test]
async fn resolving_needs_exactly_one_choice() {
    let env = Env::new();
    let client = env.start().await;
    let body = json!({"keep": "kb", "content": "x"});
    assert_eq!(
        client.send("POST", "/conflicts/c_0/resolve", body).await.0,
        400
    );
    client.stop().await;
}

#[tokio::test]
async fn creating_a_pair_saves_it() {
    let env = Env::new();
    let client = env.start().await;
    let kb = env.dir.path().join("kb2");
    let repo = env.dir.path().join("repo2/docs");
    std::fs::create_dir_all(&kb).unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(env.dir.path().join("repo2/.git")).unwrap();
    let body = json!({"name": "second", "kb": kb, "repo": repo});
    let (status, created) = client.send("POST", "/pairs", body).await;
    assert_eq!(status, 201, "{created}");
    assert_eq!(client.get("/pairs/second/config").await.1["name"], "second");
    client.stop().await;
}

#[tokio::test]
async fn creating_a_pair_refuses_unsafe_folders() {
    let env = Env::new();
    let client = env.start().await;
    for (kb, repo) in [("relative", "/repo"), ("/kb", "/a/../repo")] {
        let body = json!({"name": "second", "kb": kb, "repo": repo});
        assert_eq!(
            client.send("POST", "/pairs", body).await.0,
            400,
            "{kb} {repo}"
        );
    }
    client.stop().await;
}

#[tokio::test]
async fn creating_a_pair_twice_is_a_conflict() {
    let env = Env::new();
    let client = env.start().await;
    let body = json!({"name": "clara", "kb": env.kb(), "repo": env.repo()});
    assert_eq!(client.send("POST", "/pairs", body).await.0, 409);
    client.stop().await;
}

#[tokio::test]
async fn updating_a_pair_returns_the_plan_preview() {
    let env = Env::new();
    let client = env.start().await;
    write(&env.kb().join("note.md"), "x\n");
    let (status, updated) = client
        .send(
            "PATCH",
            "/pairs/clara/config",
            json!({"direction": "kb-to-repo"}),
        )
        .await;
    assert_eq!(status, 200, "{updated}");
    assert_eq!(updated["pair"]["direction"], "kb-to-repo");
    assert_eq!(updated["plan"]["entries"][0]["kind"], "copy");
    client.stop().await;
}

#[tokio::test]
async fn updating_a_pair_keeps_the_comments_of_the_file() {
    let env = Env::new();
    let text = std::fs::read_to_string(env.paths.config_file()).unwrap();
    write(env.paths.config_file(), &format!("# mine\n{text}"));
    let client = env.start().await;
    client
        .send("PATCH", "/pairs/clara/config", json!({"paused": true}))
        .await;
    let text = std::fs::read_to_string(env.paths.config_file()).unwrap();
    assert!(text.starts_with("# mine\n"));
    client.stop().await;
}

#[tokio::test]
async fn updating_a_pair_refuses_a_rename() {
    let env = Env::new();
    let client = env.start().await;
    let (status, _) = client
        .send("PATCH", "/pairs/clara/config", json!({"name": "x"}))
        .await;
    assert_eq!(status, 400);
    client.stop().await;
}

#[tokio::test]
async fn updating_a_pair_refuses_unknown_settings() {
    let env = Env::new();
    let client = env.start().await;
    let (status, _) = client
        .send("PATCH", "/pairs/clara/config", json!({"nope": "x"}))
        .await;
    assert_eq!(status, 400);
    client.stop().await;
}

#[tokio::test]
async fn leaving_both_is_refused_while_conflicts_are_pending() {
    let env = Env::new();
    let client = env.start().await;
    env.conflict(&client).await;
    client.sync_and_wait(json!({})).await;
    let (status, _) = client
        .send(
            "PATCH",
            "/pairs/clara/config",
            json!({"direction": "kb-to-repo"}),
        )
        .await;
    assert_eq!(status, 409);
    client.stop().await;
}

#[tokio::test]
async fn deleting_a_pair_leaves_its_files() {
    let env = Env::new();
    let client = env.start().await;
    write(&env.kb().join("note.md"), "x\n");
    client.sync_and_wait(json!({})).await;
    assert_eq!(
        client.send("DELETE", "/pairs/clara", json!({})).await.0,
        200
    );
    assert_eq!(client.get("/pairs/clara/config").await.0, 404);
    assert!(env.repo().join("note.md").exists());
    client.stop().await;
}

#[tokio::test]
async fn events_stream_reports_finished_jobs_and_ends_on_stop() {
    let env = Env::new();
    let client = env.start().await;
    let mut stream = TcpStream::connect(client.addr).await.unwrap();
    let request = format!(
        "GET /events HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {}\r\n\r\n",
        client.token
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    // The subscription exists once the response headers are out.
    let mut seen = String::new();
    let mut buffer = [0; 4096];
    while !seen.contains("\r\n\r\n") {
        let read = stream.read(&mut buffer).await.unwrap();
        seen.push_str(&String::from_utf8_lossy(&buffer[..read]));
    }
    client.sync_and_wait(json!({})).await;
    while !seen.contains("event: job-finished") {
        let read = stream.read(&mut buffer).await.unwrap();
        assert_ne!(read, 0, "the stream ended early: {seen}");
        seen.push_str(&String::from_utf8_lossy(&buffer[..read]));
    }
    tokio::time::timeout(Duration::from_secs(10), client.stop())
        .await
        .expect("an open event stream must not block the shutdown");
}

#[tokio::test]
async fn a_second_daemon_on_the_same_data_is_refused() {
    let env = Env::new();
    let client = env.start().await;
    let second = Daemon::start(env.paths.clone(), fast()).await;
    assert!(matches!(second, Err(Error::AlreadyRunning)));
    client.stop().await;
}

#[tokio::test]
async fn a_missing_configuration_stops_the_start() {
    let env = Env::new();
    std::fs::remove_file(env.paths.config_file()).unwrap();
    let started = Daemon::start(env.paths.clone(), fast()).await;
    assert!(matches!(started, Err(Error::Config(_))));
}

#[tokio::test]
async fn disabled_api_does_not_listen() {
    let env = Env::with_settings(
        "[schedule]\ninterval = \"off\"\non_startup = false\n[api]\nenabled = false\n",
    );
    let daemon = Daemon::start(env.paths.clone(), fast()).await.unwrap();
    assert!(daemon.addr().is_none());
    daemon.stop().await;
}

#[tokio::test]
async fn startup_syncs_when_asked() {
    let env =
        Env::with_settings("[schedule]\ninterval = \"off\"\non_startup = true\n[api]\nport = 0\n");
    write(&env.kb().join("note.md"), "x\n");
    let client = env.start().await;
    let target = env.repo().join("note.md");
    eventually("the startup sync", || target.exists()).await;
    client.stop().await;
}

#[tokio::test]
async fn startup_skips_paused_pairs() {
    let env =
        Env::with_settings("[schedule]\ninterval = \"1s\"\non_startup = true\n[api]\nport = 0\n");
    let text = std::fs::read_to_string(env.paths.config_file()).unwrap();
    write(env.paths.config_file(), &format!("{text}paused = true\n"));
    write(&env.kb().join("note.md"), "x\n");
    let client = env.start().await;
    tokio::time::sleep(Duration::from_millis(1300)).await;
    assert!(!env.repo().join("note.md").exists());
    client.stop().await;
}

#[tokio::test]
async fn the_interval_syncs_again() {
    let env =
        Env::with_settings("[schedule]\ninterval = \"1s\"\non_startup = false\n[api]\nport = 0\n");
    let client = env.start().await;
    write(&env.kb().join("note.md"), "x\n");
    let target = env.repo().join("note.md");
    eventually("the interval sync", || target.exists()).await;
    client.stop().await;
}

#[tokio::test]
async fn a_changed_pair_is_synced_without_restart() {
    let env = Env::new();
    let client = env.start().await;
    let kb = env.dir.path().join("kb2");
    let repo = env.dir.path().join("repo2/docs");
    std::fs::create_dir_all(&kb).unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(env.dir.path().join("repo2/.git")).unwrap();
    write(&kb.join("note.md"), "x\n");
    let text = std::fs::read_to_string(env.paths.config_file()).unwrap();
    let added = format!(
        "{text}\n[[pair]]\nname = \"second\"\nkb = '{}'\nrepo = '{}'\n",
        kb.display(),
        repo.display()
    );
    write(env.paths.config_file(), &added);
    let target = repo.join("note.md");
    eventually("the sync of the new pair", || target.exists()).await;
    client.stop().await;
}

#[tokio::test]
async fn an_invalid_edit_suspends_syncs_until_it_is_fixed() {
    let env =
        Env::with_settings("[schedule]\ninterval = \"1s\"\non_startup = false\n[api]\nport = 0\n");
    let client = env.start().await;
    let valid = std::fs::read_to_string(env.paths.config_file()).unwrap();
    write(env.paths.config_file(), "not [valid toml");
    write(&env.kb().join("note.md"), "x\n");
    let target = env.repo().join("note.md");
    tokio::time::sleep(Duration::from_millis(1300)).await;
    assert!(!target.exists());
    write(env.paths.config_file(), &valid);
    eventually("the sync after the fix", || target.exists()).await;
    client.stop().await;
}

#[tokio::test]
async fn sync_refuses_a_malformed_body() {
    let env = Env::new();
    let client = env.start().await;
    let body = json!({"dry_run": "maybe"});
    assert_eq!(client.send("POST", "/pairs/clara/sync", body).await.0, 400);
    client.stop().await;
}

fn port_of(client: &Client) -> u16 {
    client.addr.port()
}

#[tokio::test]
async fn a_hook_can_ask_the_daemon_for_a_sync() {
    let env = Env::new();
    std::fs::write(env.kb().join("note.md"), "hello\n").unwrap();
    let client = env.start().await;
    let port = port_of(&client);
    let data_dir = env.paths.data_dir().to_path_buf();
    let answer = tokio::task::spawn_blocking(move || {
        markoo_daemon::client::request_sync(&data_dir, port, "clara")
    })
    .await
    .unwrap();
    assert_eq!(answer, markoo_daemon::client::SyncRequest::Queued);
    for _ in 0..500 {
        if env.repo().join("note.md").exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the daemon did not sync");
}

#[tokio::test]
async fn a_hook_is_told_when_the_daemon_refuses() {
    let env = Env::new();
    let client = env.start().await;
    let port = port_of(&client);
    let data_dir = env.paths.data_dir().to_path_buf();
    let answer = tokio::task::spawn_blocking(move || {
        markoo_daemon::client::request_sync(&data_dir, port, "nobody")
    })
    .await
    .unwrap();
    assert!(matches!(
        answer,
        markoo_daemon::client::SyncRequest::Refused(_)
    ));
}

#[tokio::test]
async fn the_daemon_of_another_data_folder_counts_as_unavailable() {
    let env = Env::new();
    let client = env.start().await;
    let port = port_of(&client);
    let other = tempfile::tempdir().unwrap();
    std::fs::write(other.path().join(token::TOKEN_FILE), "another-token").unwrap();
    let data_dir = other.path().to_path_buf();
    let answer = tokio::task::spawn_blocking(move || {
        markoo_daemon::client::request_sync(&data_dir, port, "clara")
    })
    .await
    .unwrap();
    assert_eq!(answer, markoo_daemon::client::SyncRequest::Unavailable);
}

#[test]
fn no_daemon_counts_as_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(token::TOKEN_FILE), "token").unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    assert_eq!(
        markoo_daemon::client::request_sync(dir.path(), port, "clara"),
        markoo_daemon::client::SyncRequest::Unavailable
    );
}

const MCP_INITIALIZE: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}"#;

/// Sends one MCP request; returns the status and the raw answer.
async fn mcp_post(addr: std::net::SocketAddr, token: Option<&str>, host: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let auth = token.map_or(String::new(), |token| {
        format!("Authorization: Bearer {token}\r\n")
    });
    let request = format!(
        "POST /mcp HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n{auth}\
         Accept: application/json, text/event-stream\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\n\r\n{MCP_INITIALIZE}",
        MCP_INITIALIZE.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    // A session answers as a stream that stays open: the first bytes are enough.
    let mut buffer = vec![0; 8192];
    let mut response = String::new();
    while !response.contains("serverInfo") {
        let read = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buffer))
            .await
            .unwrap()
            .unwrap();
        if read == 0 {
            break;
        }
        response.push_str(&String::from_utf8_lossy(&buffer[..read]));
    }
    let status = response.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, response)
}

#[tokio::test]
async fn mcp_answers_over_http_with_the_token() {
    let env = Env::new();
    let client = env.start().await;
    let (status, response) = mcp_post(client.addr, Some(&client.token), "localhost").await;
    assert_eq!(status, 200, "{response}");
    assert!(response.contains("\"name\":\"markoo\""), "{response}");
}

#[tokio::test]
async fn mcp_over_http_needs_the_token() {
    let env = Env::new();
    let client = env.start().await;
    assert_eq!(mcp_post(client.addr, None, "localhost").await.0, 401);
}

#[tokio::test]
async fn mcp_over_http_refuses_another_host() {
    let env = Env::new();
    let client = env.start().await;
    let status = mcp_post(client.addr, Some(&client.token), "evil.example")
        .await
        .0;
    assert_eq!(status, 403);
}

#[tokio::test]
async fn mcp_over_http_can_be_turned_off() {
    let env = Env::with_settings(&format!("{QUIET}[mcp]\nhttp = false\n"));
    let client = env.start().await;
    assert_eq!(
        mcp_post(client.addr, Some(&client.token), "localhost")
            .await
            .0,
        404
    );
}

#[tokio::test]
async fn the_daemon_stops_with_an_mcp_session_open() {
    let env = Env::new();
    let mut client = env.start().await;
    let addr = client.addr;
    let token = client.token.clone();
    mcp_post(addr, Some(&token), "localhost").await;
    let daemon = client.daemon.take().unwrap();
    tokio::time::timeout(Duration::from_secs(10), daemon.stop())
        .await
        .unwrap();
}
