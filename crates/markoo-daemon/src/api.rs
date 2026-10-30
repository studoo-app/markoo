//! The local HTTP API. Handlers only translate between HTTP and `markoo-core`.

use std::convert::Infallible;
use std::path::Path as FsPath;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use axum::body::Bytes;
use axum::extract::{Path, Query, Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use markoo_core::config::{Config, ConfigError, NewPair, PairEntry};
use markoo_core::engine::issue_id;
use markoo_core::fs::DiskFs;
use markoo_core::paths::Paths;
use markoo_core::{Engine, EntryPath, Pair, Resolution, Side, manage};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{Stream, StreamExt};

use crate::queue::{Event, Queue, Trigger};
use crate::token;

#[derive(Clone)]
struct App {
    paths: Paths,
    queue: Queue,
    started: Instant,
    /// Requests changing `markoo.toml` wait for each other instead of
    /// overwriting each other's edits.
    config_edit: Arc<Mutex<()>>,
}

/// An error, as seen by a client.
#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({"error": self.message}))).into_response()
    }
}

impl From<ConfigError> for ApiError {
    fn from(error: ConfigError) -> Self {
        let status = match &error {
            ConfigError::UnknownPair(_) => StatusCode::NOT_FOUND,
            ConfigError::DuplicatePair(_) => StatusCode::CONFLICT,
            ConfigError::InvalidName(_)
            | ConfigError::UnknownKey(_)
            | ConfigError::InvalidPair { .. }
            | ConfigError::UnsupportedKbType { .. }
            | ConfigError::Invalid(_) => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self::new(status, error.to_string())
    }
}

impl From<markoo_core::Error> for ApiError {
    fn from(error: markoo_core::Error) -> Self {
        use markoo_core::Error as E;
        if let E::Config(error) = error {
            return error.into();
        }
        let status = match &error {
            E::NotPending(_) => StatusCode::NOT_FOUND,
            E::Busy(_)
            | E::DirectionBlocked { .. }
            | E::MissingRoot { .. }
            | E::AlreadyExcluded(_) => StatusCode::CONFLICT,
            E::NeedsBidirectional | E::NeedsOneWay | E::KeepBothNeedsTwoVersions(_) => {
                StatusCode::BAD_REQUEST
            }
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self::new(status, error.to_string())
    }
}

type ApiResult<T = Json<Value>> = Result<T, ApiError>;

/// Runs blocking work (disk scans, SQLite) outside the async runtime.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> ApiResult<T> + Send + 'static,
) -> ApiResult<T> {
    tokio::task::spawn_blocking(work).await.map_err(|error| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("the request stopped unexpectedly: {error}"),
        )
    })?
}

pub(crate) fn router(paths: Paths, queue: Queue) -> Router {
    let app = App {
        paths,
        queue,
        started: Instant::now(),
        config_edit: Arc::default(),
    };
    Router::new()
        .route("/health", get(health))
        .route("/pairs", get(list_pairs).post(create_pair))
        .route("/pairs/{pair}", axum::routing::delete(delete_pair))
        .route("/pairs/{pair}/status", get(status))
        .route("/pairs/{pair}/diff", get(diff))
        .route("/pairs/{pair}/sync", post(sync))
        .route("/pairs/{pair}/config", get(get_config).patch(patch_config))
        .route("/jobs/{id}", get(job))
        .route("/conflicts", get(conflicts))
        .route("/conflicts/{id}/resolve", post(resolve))
        .route("/events", get(events))
        .route_layer(middleware::from_fn_with_state(app.clone(), authenticate))
        .with_state(app)
}

/// Checks the bearer token. It is read at every request so that
/// `markoo token rotate` applies without restarting the daemon.
async fn authenticate(State(app): State<App>, request: Request, next: Next) -> Response {
    let given = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    let data_dir = app.paths.data_dir().to_path_buf();
    let expected = blocking(move || {
        token::ensure(&data_dir)
            .map_err(|error| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
    })
    .await;
    match (expected, given) {
        (Ok(expected), Some(given)) if token::matches(&expected, given) => next.run(request).await,
        (Err(error), _) => error.into_response(),
        _ => ApiError::new(StatusCode::UNAUTHORIZED, "missing or invalid token").into_response(),
    }
}

fn load_config(paths: &Paths) -> ApiResult<Config> {
    Ok(Config::load(paths.config_file())?)
}

fn open_engine(paths: &Paths) -> ApiResult<Engine<DiskFs>> {
    Ok(Engine::open(DiskFs, paths)?)
}

/// A pair that can be used: it exists and passes validation.
fn usable_pair(paths: &Paths, name: &str) -> ApiResult<Pair> {
    let config = load_config(paths)?;
    let pair = config.pair(name)?.pair.clone();
    let validation = config.validate();
    let errors: Vec<String> = validation
        .errors
        .into_iter()
        .filter(|problem| problem.pair == name)
        .map(|problem| problem.message)
        .collect();
    if errors.is_empty() {
        return Ok(pair);
    }
    Err(ConfigError::InvalidPair {
        pair: name.to_owned(),
        message: errors.join("; "),
    }
    .into())
}

fn pair_view(entry: &PairEntry) -> Value {
    let pair = &entry.pair;
    json!({
        "name": pair.name,
        "kb": pair.kb,
        "kb_type": pair.kb_type,
        "repo": pair.repo,
        "direction": pair.direction,
        "target_changes": pair.target_changes,
        "include": pair.include,
        "exclude": pair.exclude,
        "paused": pair.paused,
    })
}

fn plan_view(engine: &Engine<DiskFs>, pair: &Pair) -> ApiResult {
    let planned = engine.plan(pair)?;
    let entries: Vec<_> = planned
        .plan
        .entries
        .iter()
        .filter(|entry| entry.is_visible())
        .map(markoo_core::plan::PlannedEntry::view)
        .collect();
    Ok(Json(json!({
        "pair": pair.name,
        "direction": pair.direction,
        "paused": pair.paused,
        "entries": entries,
        "attachments": planned.plan.attachments,
        "scan_issues": planned.scan_issues,
    })))
}

fn warnings_view(warnings: &[markoo_core::config::Problem]) -> Vec<&str> {
    warnings
        .iter()
        .map(|problem| problem.message.as_str())
        .collect()
}

async fn health(State(app): State<App>) -> Json<Value> {
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_seconds": app.started.elapsed().as_secs(),
        "active_pairs": app.queue.active_pairs(),
    }))
}

async fn list_pairs(State(app): State<App>) -> ApiResult {
    blocking(move || {
        let config = load_config(&app.paths)?;
        let validation = config.validate();
        let engine = open_engine(&app.paths)?;
        let mut views = Vec::with_capacity(config.pairs.len());
        for entry in &config.pairs {
            let name = entry.pair.name.as_str();
            let mut view = pair_view(entry);
            view["pending"] = json!(engine.pending_count(&entry.pair)?);
            view["active"] = json!(app.queue.active_pairs().iter().any(|p| p == name));
            view["errors"] = json!(
                validation
                    .errors
                    .iter()
                    .filter(|problem| problem.pair == name)
                    .map(|problem| &problem.message)
                    .collect::<Vec<_>>()
            );
            views.push(view);
        }
        Ok(Json(Value::Array(views)))
    })
    .await
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreatePair {
    name: String,
    kb: String,
    repo: String,
    kb_type: Option<String>,
    direction: Option<String>,
    target_changes: Option<String>,
}

fn check_folder(field: &str, path: &str) -> ApiResult<()> {
    if manage::is_safe_folder(path) {
        return Ok(());
    }
    Err(ApiError::bad_request(format!(
        "{field} must be an absolute path without \"..\""
    )))
}

async fn create_pair(
    State(app): State<App>,
    Json(body): Json<CreatePair>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    check_folder("kb", &body.kb)?;
    check_folder("repo", &body.repo)?;
    blocking(move || {
        let new = NewPair {
            name: body.name,
            kb: body.kb,
            kb_type: body.kb_type,
            repo: body.repo,
            direction: body.direction,
            target_changes: body.target_changes,
        };
        let warnings = {
            let _edit = app
                .config_edit
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            manage::add_pair(&app.paths, &new)?
        };
        let config = load_config(&app.paths)?;
        let mut view = pair_view(config.pair(&new.name)?);
        view["warnings"] = json!(warnings_view(&warnings));
        Ok((StatusCode::CREATED, Json(view)))
    })
    .await
}

async fn delete_pair(State(app): State<App>, Path(name): Path<String>) -> ApiResult {
    blocking(move || {
        let _edit = app
            .config_edit
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        manage::remove_pair(&app.paths, &name)?;
        Ok(Json(json!({"removed": name})))
    })
    .await
}

async fn status(State(app): State<App>, Path(name): Path<String>) -> ApiResult {
    blocking(move || {
        let pair = usable_pair(&app.paths, &name)?;
        plan_view(&open_engine(&app.paths)?, &pair)
    })
    .await
}

#[derive(Debug, Deserialize)]
struct DiffQuery {
    path: Option<String>,
}

/// A path received from a client, confined to the pair's folders.
fn entry_path(path: &str) -> ApiResult<EntryPath> {
    EntryPath::from_relative(FsPath::new(path)).ok_or_else(|| {
        ApiError::bad_request(format!("{path} must be relative to the pair's folders"))
    })
}

async fn diff(
    State(app): State<App>,
    Path(name): Path<String>,
    Query(query): Query<DiffQuery>,
) -> ApiResult {
    blocking(move || {
        let only = query.path.as_deref().map(entry_path).transpose()?;
        let pair = usable_pair(&app.paths, &name)?;
        let diffs = open_engine(&app.paths)?.diffs(&pair, only.as_ref())?;
        Ok(Json(json!({"pair": name, "diffs": diffs})))
    })
    .await
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SyncBody {
    #[serde(default)]
    dry_run: bool,
}

async fn sync(
    State(app): State<App>,
    Path(name): Path<String>,
    body: Bytes,
) -> ApiResult<(StatusCode, Json<Value>)> {
    // The body is optional, whatever the Content-Type says.
    let dry_run = if body.is_empty() {
        false
    } else {
        serde_json::from_slice::<SyncBody>(&body)
            .map_err(|error| ApiError::bad_request(error.to_string()))?
            .dry_run
    };
    let paths = app.paths.clone();
    let lookup = name.clone();
    let pair = blocking(move || usable_pair(&paths, &lookup)).await?;
    if pair.paused {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            format!("pair \"{name}\" is paused"),
        ));
    }
    let job = app.queue.submit(&name, dry_run, Trigger::Api);
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({"job": job, "pair": name})),
    ))
}

async fn job(State(app): State<App>, Path(id): Path<String>) -> ApiResult {
    let job = app
        .queue
        .job(&id)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, format!("no job {id}")))?;
    Ok(Json(json!(job)))
}

async fn get_config(State(app): State<App>, Path(name): Path<String>) -> ApiResult {
    blocking(move || {
        let config = load_config(&app.paths)?;
        Ok(Json(pair_view(config.pair(&name)?)))
    })
    .await
}

/// Turns a JSON value into the text form of `ConfigFile::set`.
fn setting_text(key: &str, value: &Value) -> ApiResult<String> {
    let text = |value: &Value| value.as_str().map(str::to_owned);
    let converted = match value {
        Value::String(value) => Some(value.clone()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Array(items) => items
            .iter()
            .map(text)
            .collect::<Option<Vec<_>>>()
            .map(|items| items.join(",")),
        _ => None,
    };
    converted.ok_or_else(|| ApiError::bad_request(format!("invalid value for {key}")))
}

async fn patch_config(
    State(app): State<App>,
    Path(name): Path<String>,
    Json(body): Json<serde_json::Map<String, Value>>,
) -> ApiResult {
    let mut changes = Vec::with_capacity(body.len());
    for (key, value) in &body {
        // The state is stored under the name: renaming would lose the base.
        if key == "name" {
            return Err(ApiError::bad_request("a pair cannot be renamed"));
        }
        let text = setting_text(key, value)?;
        if key == "kb" || key == "repo" {
            check_folder(key, &text)?;
        }
        changes.push((key.clone(), text));
    }
    blocking(move || {
        let warnings = {
            let _edit = app
                .config_edit
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            manage::set_pair(&app.paths, &name, &changes)?
        };
        let config = load_config(&app.paths)?;
        let entry = config.pair(&name)?;
        let preview = plan_view(&open_engine(&app.paths)?, &entry.pair);
        Ok(Json(json!({
            "pair": pair_view(entry),
            "warnings": warnings_view(&warnings),
            "plan": preview.as_ref().ok().map(|Json(plan)| plan),
            "plan_error": preview.as_ref().err().map(|error| &error.message),
        })))
    })
    .await
}

#[derive(Debug, Deserialize)]
struct ConflictsQuery {
    pair: Option<String>,
}

async fn conflicts(State(app): State<App>, Query(query): Query<ConflictsQuery>) -> ApiResult {
    blocking(move || {
        let config = load_config(&app.paths)?;
        if let Some(name) = &query.pair {
            config.pair(name)?;
        }
        let validation = config.validate();
        let engine = open_engine(&app.paths)?;
        let (mut conflicts, mut excluded, mut errors) = (Vec::new(), Vec::new(), Vec::new());
        for entry in &config.pairs {
            let pair = &entry.pair;
            if query.pair.as_ref().is_some_and(|name| *name != pair.name)
                || !validation.is_ok(&pair.name)
            {
                continue;
            }
            match engine.issues(pair) {
                Ok(issues) => conflicts.extend(issues.iter().map(|issue| {
                    json!({
                        "id": issue_id(&pair.name, &issue.path),
                        "pair": pair.name,
                        "path": issue.path,
                        "kind": issue.kind,
                        "kb": issue.kb,
                        "repo": issue.repo,
                    })
                })),
                Err(error) => errors.push(json!({"pair": pair.name, "error": error.to_string()})),
            }
            for path in engine.ignored(pair)? {
                excluded.push(json!({
                    "id": issue_id(&pair.name, &path),
                    "pair": pair.name,
                    "path": path,
                }));
            }
        }
        Ok(Json(json!({
            "conflicts": conflicts,
            "excluded": excluded,
            "errors": errors,
        })))
    })
    .await
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolveBody {
    keep: Option<String>,
    content: Option<String>,
    #[serde(default)]
    push_to_source: bool,
}

impl ResolveBody {
    fn resolution(self) -> ApiResult<Resolution> {
        match (self.keep.as_deref(), self.content, self.push_to_source) {
            (Some("kb"), None, false) => Ok(Resolution::Keep(Side::Kb)),
            (Some("repo"), None, false) => Ok(Resolution::Keep(Side::Repo)),
            (Some("both"), None, false) => Ok(Resolution::KeepBoth),
            (None, Some(content), false) => Ok(Resolution::Content(content)),
            (None, None, true) => Ok(Resolution::PushToSource),
            _ => Err(ApiError::bad_request(
                "give one of keep (kb, repo or both), content or push_to_source",
            )),
        }
    }
}

/// Finds which pair and file an issue identifier stands for. Scanning the
/// pending issues means that a client can never name a file outside of them.
fn find_issue(paths: &Paths, id: &str) -> ApiResult<(Pair, EntryPath)> {
    let config = load_config(paths)?;
    let validation = config.validate();
    let engine = open_engine(paths)?;
    for entry in &config.pairs {
        let pair = &entry.pair;
        if !validation.is_ok(&pair.name) {
            continue;
        }
        // An unavailable pair cannot hold the issue we can act on.
        let Ok(issues) = engine.issues(pair) else {
            continue;
        };
        let known = issues
            .into_iter()
            .map(|issue| issue.path)
            .chain(engine.ignored(pair)?);
        for path in known {
            if issue_id(&pair.name, &path) == id {
                return Ok((pair.clone(), path));
            }
        }
    }
    Err(ApiError::new(
        StatusCode::NOT_FOUND,
        format!("no conflict or divergence {id}"),
    ))
}

async fn resolve(
    State(app): State<App>,
    Path(id): Path<String>,
    Json(body): Json<ResolveBody>,
) -> ApiResult {
    let resolution = body.resolution()?;
    blocking(move || {
        let (pair, path) = find_issue(&app.paths, &id)?;
        let report = open_engine(&app.paths)?.resolve(&pair, &path, resolution)?;
        Ok(Json(
            json!({"pair": pair.name, "path": path, "report": report}),
        ))
    })
    .await
}

async fn events(State(app): State<App>) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let stream = BroadcastStream::new(app.queue.subscribe())
        // A subscriber that lagged misses events, but stays connected.
        .filter_map(Result::ok)
        .take_while(|event| *event != Event::Shutdown)
        .map(|event| {
            let name = event.name();
            let data = serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_owned());
            Ok(SseEvent::default().event(name).data(data))
        });
    Sse::new(stream).keep_alive(KeepAlive::default())
}
