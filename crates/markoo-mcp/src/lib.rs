//! The MCP server: tools and resources over `markoo-core`, on stdio for a
//! client that spawns `markoo mcp`, and on HTTP inside the daemon.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use markoo_core::config::Config;
use markoo_core::paths::Paths;
use markoo_core::service::{self, SyncOutcome};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ContentBlock, Implementation, ListResourceTemplatesResult, ListResourcesResult,
    PaginatedRequestParams, ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult,
    Resource, ResourceContents, ResourceTemplate, ServerCapabilities, ServerConfig,
};
use rmcp::schemars::{self, JsonSchema};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData, ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use serde::Deserialize;
use serde_json::{Map, Value, json};

const PAIRS_PREFIX: &str = "markoo://pairs/";
const STATUS_SUFFIX: &str = "/status";
const CONFLICTS_PREFIX: &str = "markoo://conflicts/";

/// Errors raised while serving on stdio.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The async runtime could not start.
    #[error("cannot start the MCP server: {0}")]
    Runtime(#[from] std::io::Error),
    /// The connection with the client failed.
    #[error("MCP connection failed: {0}")]
    Connection(String),
}

/// A future answering a sync request.
pub type SyncFuture = Pin<Box<dyn Future<Output = Result<SyncOutcome, String>> + Send>>;

/// How a sync gets run: directly, or through the queue of the daemon.
pub trait Syncer: Send + Sync + 'static {
    /// Syncs a pair and returns what was done.
    fn sync(&self, pair: String, dry_run: bool) -> SyncFuture;
}

/// Runs syncs in this process. The lock of the pair keeps it from running at
/// the same time as a daemon: it then answers that the pair is busy.
#[derive(Debug, Clone)]
pub struct DirectSyncer {
    paths: Paths,
}

impl DirectSyncer {
    /// Creates a syncer on these locations.
    pub fn new(paths: Paths) -> Self {
        Self { paths }
    }
}

impl Syncer for DirectSyncer {
    fn sync(&self, pair: String, dry_run: bool) -> SyncFuture {
        let paths = self.paths.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || service::run_sync(&paths, &pair, dry_run))
                .await
                .map_err(|error| error.to_string())?
                .map_err(|error| error.to_string())
        })
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
struct OptionalPair {
    /// Name of a pair; all of them when omitted.
    pair: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct RequiredPair {
    /// Name of the pair.
    pair: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DiffParams {
    /// Name of the pair.
    pair: String,
    /// The file, relative to the pair's folders.
    path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct RunParams {
    /// Name of a pair; every pair that can sync when omitted.
    pair: Option<String>,
    /// Compute the plan without writing anything.
    #[serde(default)]
    dry_run: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ResolveParams {
    /// Identifier given by `list_conflicts`.
    id: String,
    /// Version to keep: `kb`, `repo` or `both`.
    keep: Option<String>,
    /// Text to use on both sides.
    content: Option<String>,
    /// One-way pairs: copy the target's version to the source, once.
    #[serde(default)]
    push_to_source: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct UpdateParams {
    /// Name of the pair.
    pair: String,
    /// Settings to change: `direction`, `target_changes`, `kb`, `repo`,
    /// `kb_type`, `include`, `exclude`, `paused`, `transform.links`,
    /// `transform.assets_dir`, `transform.frontmatter`, `kb_options.callouts`,
    /// `kb_options.frontmatter_drop`, `hooks.install`, `hooks.branches`.
    changes: Map<String, Value>,
}

/// The MCP server. Every request reads `markoo.toml` again, as the API does.
#[derive(Clone)]
pub struct MarkooServer {
    paths: Paths,
    syncer: Arc<dyn Syncer>,
}

fn answer(value: &Value) -> Result<CallToolResult, ErrorData> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
}

/// A failure the model can read and act on, such as a pair being busy.
fn failure(message: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message.into())])
}

#[tool_router]
impl MarkooServer {
    /// Creates a server on these locations.
    pub fn new(paths: Paths, syncer: Arc<dyn Syncer>) -> Self {
        Self { paths, syncer }
    }

    /// Runs engine work outside the async runtime and turns it into an answer.
    async fn blocking(
        &self,
        work: impl FnOnce(&Paths) -> Result<Value, markoo_core::Error> + Send + 'static,
    ) -> Result<CallToolResult, ErrorData> {
        let paths = self.paths.clone();
        let result = tokio::task::spawn_blocking(move || work(&paths))
            .await
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
        match result {
            Ok(value) => answer(&value),
            Err(error) => Ok(failure(error.to_string())),
        }
    }

    #[tool(
        description = "List the configured pairs with their knowledge base type, direction, pending issues and errors.",
        annotations(read_only_hint = true)
    )]
    async fn list_pairs(&self) -> Result<CallToolResult, ErrorData> {
        self.blocking(|paths| Ok(Value::Array(service::list_pairs(paths)?)))
            .await
    }

    #[tool(
        description = "Show the current plan: what a sync would do. Nothing is written.",
        annotations(read_only_hint = true)
    )]
    async fn sync_status(
        &self,
        Parameters(params): Parameters<OptionalPair>,
    ) -> Result<CallToolResult, ErrorData> {
        self.blocking(move |paths| match params.pair {
            Some(pair) => service::status(paths, &pair),
            None => Ok(Value::Array(service::status_of_all(paths)?)),
        })
        .await
    }

    #[tool(
        description = "Show the unified diff of one file between the knowledge base and the repository, in canonical form.",
        annotations(read_only_hint = true)
    )]
    async fn sync_diff(
        &self,
        Parameters(params): Parameters<DiffParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.blocking(move |paths| service::diff(paths, &params.pair, Some(&params.path)))
            .await
    }

    #[tool(
        description = "Sync a pair, or every pair that can sync, and return the report. Writes to the knowledge base and the repository, unless dry_run is set. Conflicts are never resolved by a sync.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn sync_run(
        &self,
        Parameters(params): Parameters<RunParams>,
    ) -> Result<CallToolResult, ErrorData> {
        if let Some(pair) = params.pair {
            return match self.syncer.sync(pair.clone(), params.dry_run).await {
                Ok(outcome) => answer(&json!({"pair": pair, "outcome": outcome})),
                Err(error) => Ok(failure(error)),
            };
        }
        let paths = self.paths.clone();
        let names = tokio::task::spawn_blocking(move || service::runnable_pairs(&paths))
            .await
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
        let names = match names {
            Ok(names) => names,
            Err(error) => return Ok(failure(error.to_string())),
        };
        let mut results = Vec::with_capacity(names.len());
        for pair in names {
            results.push(match self.syncer.sync(pair.clone(), params.dry_run).await {
                Ok(outcome) => json!({"pair": pair, "outcome": outcome}),
                Err(error) => json!({"pair": pair, "error": error}),
            });
        }
        answer(&Value::Array(results))
    }

    #[tool(
        description = "List pending conflicts and divergences, with the knowledge base, base and repository versions of each file.",
        annotations(read_only_hint = true)
    )]
    async fn list_conflicts(
        &self,
        Parameters(params): Parameters<OptionalPair>,
    ) -> Result<CallToolResult, ErrorData> {
        self.blocking(move |paths| service::conflicts(paths, params.pair.as_deref()))
            .await
    }

    #[tool(
        description = "Settle a conflict or a divergence with one of keep, content or push_to_source. Writes to the files concerned; what it overwrites is kept in Markoo's backups.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn resolve_conflict(
        &self,
        Parameters(params): Parameters<ResolveParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.blocking(move |paths| {
            let resolution = service::resolution(
                params.keep.as_deref(),
                params.content,
                params.push_to_source,
            )?;
            service::resolve(paths, &params.id, resolution)
        })
        .await
    }

    #[tool(
        description = "Show the configuration of a pair: folders, knowledge base type, direction and filters.",
        annotations(read_only_hint = true)
    )]
    async fn get_pair_config(
        &self,
        Parameters(params): Parameters<RequiredPair>,
    ) -> Result<CallToolResult, ErrorData> {
        self.blocking(move |paths| service::pair_config(paths, &params.pair))
            .await
    }

    #[tool(
        description = "Change settings of a pair and return a preview of the new plan. Refused unless the user set mcp.allow_config_write = true in markoo.toml.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn update_pair_config(
        &self,
        Parameters(params): Parameters<UpdateParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.blocking(move |paths| {
            // Checked at each call: the user may withdraw it at any time.
            if !Config::load(paths.config_file())?.mcp.allow_config_write {
                return Err(markoo_core::Error::InvalidRequest(
                    "changing the configuration is disabled: \
                     the user can allow it with mcp.allow_config_write = true in markoo.toml"
                        .to_owned(),
                ));
            }
            service::update_config(paths, &params.pair, &params.changes)
        })
        .await
    }
}

/// Reads a resource of the server.
fn read_resource_value(paths: &Paths, uri: &str) -> Result<Value, ErrorData> {
    let not_found = || ErrorData::resource_not_found(format!("no resource {uri}"), None);
    let result = if let Some(id) = uri.strip_prefix(CONFLICTS_PREFIX) {
        service::conflict(paths, id)
    } else {
        let pair = uri
            .strip_prefix(PAIRS_PREFIX)
            .and_then(|rest| rest.strip_suffix(STATUS_SUFFIX))
            .ok_or_else(not_found)?;
        service::status(paths, pair)
    };
    result.map_err(|error| match error {
        markoo_core::Error::UnknownIssue(_)
        | markoo_core::Error::Config(markoo_core::config::ConfigError::UnknownPair(_)) => {
            not_found()
        }
        other => ErrorData::internal_error(other.to_string(), None),
    })
}

#[tool_handler]
impl ServerHandler for MarkooServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::new("markoo", env!("CARGO_PKG_VERSION")))
        .with_instructions(
            "Markoo syncs Markdown between a knowledge base and a Git repository. Check \
             sync_status before sync_run. Conflicts wait for a decision: show them to the user \
             before calling resolve_conflict. Markoo never commits or pushes.",
        )
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let paths = self.paths.clone();
        let resources = tokio::task::spawn_blocking(move || -> Result<Vec<Resource>, String> {
            let config = Config::load(paths.config_file()).map_err(|e| e.to_string())?;
            let mut resources: Vec<Resource> = config
                .pairs
                .iter()
                .map(|entry| {
                    let name = &entry.pair.name;
                    Resource::new(
                        format!("{PAIRS_PREFIX}{name}{STATUS_SUFFIX}"),
                        format!("Status of {name}"),
                    )
                    .with_mime_type("application/json")
                })
                .collect();
            let conflicts = service::conflicts(&paths, None).map_err(|e| e.to_string())?;
            for issue in conflicts["conflicts"].as_array().into_iter().flatten() {
                let id = issue["id"].as_str().unwrap_or_default();
                let path = issue["path"].as_str().unwrap_or_default();
                resources.push(
                    Resource::new(
                        format!("{CONFLICTS_PREFIX}{id}"),
                        format!("Conflict on {path}"),
                    )
                    .with_mime_type("application/json"),
                );
            }
            Ok(resources)
        })
        .await
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?
        .map_err(|error| ErrorData::internal_error(error, None))?;
        Ok(ListResourcesResult::with_all_items(resources))
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        let template = |uri: String, name: &str| {
            ResourceTemplate::new(uri, name).with_mime_type("application/json")
        };
        Ok(ListResourceTemplatesResult::with_all_items(vec![
            template(
                format!("{PAIRS_PREFIX}{{pair}}{STATUS_SUFFIX}"),
                "Status of a pair",
            ),
            template(
                format!("{CONFLICTS_PREFIX}{{id}}"),
                "KB, base and repo versions of a conflict",
            ),
        ]))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let paths = self.paths.clone();
        let uri = request.uri;
        let lookup = uri.clone();
        let value = tokio::task::spawn_blocking(move || read_resource_value(&paths, &lookup))
            .await
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))??;
        let text = serde_json::to_string_pretty(&value)
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
        Ok(ReadResourceResult::new(vec![
            ResourceContents::text(text, uri).with_mime_type("application/json"),
        ])
        .into())
    }
}

/// Serves on stdin and stdout until the client closes them. Nothing else may
/// write to stdout, which carries the protocol.
pub fn run_stdio(paths: Paths) -> Result<(), Error> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let syncer = Arc::new(DirectSyncer::new(paths.clone()));
        let server = MarkooServer::new(paths, syncer)
            .serve(rmcp::transport::stdio())
            .await
            .map_err(|error| Error::Connection(error.to_string()))?;
        server
            .waiting()
            .await
            .map_err(|error| Error::Connection(error.to_string()))?;
        Ok(())
    })
}

/// The HTTP endpoint, to be mounted on a server that checks the client.
pub struct HttpEndpoint {
    /// The service to mount.
    pub service: StreamableHttpService<MarkooServer, LocalSessionManager>,
    /// Ends the open sessions, which would otherwise keep a server from stopping.
    pub stop: Box<dyn FnOnce() + Send>,
}

/// Creates the HTTP endpoint. It only accepts loopback hosts, which protects
/// it from DNS rebinding.
pub fn http_endpoint(paths: Paths, syncer: Arc<dyn Syncer>) -> HttpEndpoint {
    let config = StreamableHttpServerConfig::default();
    let cancel = config.cancellation_token.clone();
    let service = StreamableHttpService::new(
        move || Ok(MarkooServer::new(paths.clone(), Arc::clone(&syncer))),
        Arc::new(LocalSessionManager::default()),
        config,
    );
    HttpEndpoint {
        service,
        stop: Box::new(move || cancel.cancel()),
    }
}
