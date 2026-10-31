//! The local HTTP API. Handlers only translate between HTTP and `markoo-core`.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Instant;

use axum::body::Bytes;
use axum::extract::{Path, Query, Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use markoo_core::config::{Config, ConfigError, NewPair};
use markoo_core::manage;
use markoo_core::paths::Paths;
use markoo_core::service;
use markoo_mcp::{SyncFuture, Syncer};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{Stream, StreamExt};

use crate::queue::{Event, Queue, Trigger};
use crate::token;

/// Ends the open MCP sessions.
pub(crate) type McpStop = Box<dyn FnOnce() + Send>;

#[derive(Clone)]
struct App {
    paths: Paths,
    queue: Queue,
    started: Instant,
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
            E::NotPending(_) | E::UnknownIssue(_) => StatusCode::NOT_FOUND,
            E::Busy(_)
            | E::Paused(_)
            | E::DirectionBlocked { .. }
            | E::MissingRoot { .. }
            | E::AlreadyExcluded(_) => StatusCode::CONFLICT,
            E::NeedsBidirectional
            | E::NeedsOneWay
            | E::KeepBothNeedsTwoVersions(_)
            | E::InvalidRequest(_) => StatusCode::BAD_REQUEST,
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

/// Runs the syncs of MCP clients through the queue, waiting for their report.
struct QueueSyncer(Queue);

impl Syncer for QueueSyncer {
    fn sync(&self, pair: String, dry_run: bool) -> SyncFuture {
        let queue = self.0.clone();
        Box::pin(async move {
            let id = queue.submit(&pair, dry_run, Trigger::Mcp);
            queue.finished(&id).await
        })
    }
}

/// The API, and what ends its open MCP sessions when `mcp` is set.
pub(crate) fn router(paths: Paths, queue: Queue, mcp: bool) -> (Router, Option<McpStop>) {
    let endpoint =
        mcp.then(|| markoo_mcp::http_endpoint(paths.clone(), Arc::new(QueueSyncer(queue.clone()))));
    let app = App {
        paths,
        queue,
        started: Instant::now(),
    };
    let mut routes = Router::new()
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
        .route("/events", get(events));
    let mut stop = None;
    if let Some(endpoint) = endpoint {
        routes = routes.nest_service("/mcp", endpoint.service);
        stop = Some(endpoint.stop);
    }
    let router = routes
        .route_layer(middleware::from_fn_with_state(app.clone(), authenticate))
        .with_state(app);
    (router, stop)
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
        let mut views = service::list_pairs(&app.paths)?;
        let active = app.queue.active_pairs();
        for view in &mut views {
            view["active"] = json!(active.iter().any(|pair| view["name"] == pair.as_str()));
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

async fn create_pair(
    State(app): State<App>,
    Json(body): Json<CreatePair>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    service::check_folder("kb", &body.kb)?;
    service::check_folder("repo", &body.repo)?;
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
            let _edit = service::config_edit();
            manage::add_pair(&app.paths, &new)?
        };
        let config = Config::load(app.paths.config_file()).map_err(ApiError::from)?;
        let mut view = service::pair_view(config.pair(&new.name).map_err(ApiError::from)?);
        view["warnings"] = json!(service::warnings_view(&warnings));
        Ok((StatusCode::CREATED, Json(view)))
    })
    .await
}

async fn delete_pair(State(app): State<App>, Path(name): Path<String>) -> ApiResult {
    blocking(move || {
        let _edit = service::config_edit();
        manage::remove_pair(&app.paths, &name)?;
        Ok(Json(json!({"removed": name})))
    })
    .await
}

async fn status(State(app): State<App>, Path(name): Path<String>) -> ApiResult {
    blocking(move || Ok(Json(service::status(&app.paths, &name)?))).await
}

#[derive(Debug, Deserialize)]
struct DiffQuery {
    path: Option<String>,
}

async fn diff(
    State(app): State<App>,
    Path(name): Path<String>,
    Query(query): Query<DiffQuery>,
) -> ApiResult {
    blocking(move || {
        Ok(Json(service::diff(
            &app.paths,
            &name,
            query.path.as_deref(),
        )?))
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
    let pair = blocking(move || Ok(service::usable_pair(&paths, &lookup)?)).await?;
    if pair.paused {
        return Err(markoo_core::Error::Paused(name).into());
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
    blocking(move || Ok(Json(service::pair_config(&app.paths, &name)?))).await
}

async fn patch_config(
    State(app): State<App>,
    Path(name): Path<String>,
    Json(body): Json<serde_json::Map<String, Value>>,
) -> ApiResult {
    blocking(move || Ok(Json(service::update_config(&app.paths, &name, &body)?))).await
}

#[derive(Debug, Deserialize)]
struct ConflictsQuery {
    pair: Option<String>,
}

async fn conflicts(State(app): State<App>, Query(query): Query<ConflictsQuery>) -> ApiResult {
    blocking(move || Ok(Json(service::conflicts(&app.paths, query.pair.as_deref())?))).await
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolveBody {
    keep: Option<String>,
    content: Option<String>,
    #[serde(default)]
    push_to_source: bool,
}

async fn resolve(
    State(app): State<App>,
    Path(id): Path<String>,
    Json(body): Json<ResolveBody>,
) -> ApiResult {
    let resolution = service::resolution(body.keep.as_deref(), body.content, body.push_to_source)?;
    blocking(move || Ok(Json(service::resolve(&app.paths, &id, resolution)?))).await
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
