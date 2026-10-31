//! Markoo daemon: scheduler, job queue and local HTTP API.
//!
//! This crate only decides *when* to sync and exposes what `markoo-core`
//! returns; all the sync logic stays in the core.

mod api;
pub mod client;
mod queue;
mod runner;
mod scheduler;
pub mod service;
pub mod token;

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use markoo_core::config::Config;
use markoo_core::fs::{DiskFs, Fs, FsError, FsLock};
use markoo_core::paths::Paths;
use tokio::sync::watch;
use tokio::task::JoinHandle;

pub use queue::{Event, Job, JobOutcome, JobStatus, Queue, Runner, Trigger};
pub use runner::EngineRunner;

/// Name of the file that keeps a data directory to a single daemon.
const DAEMON_LOCK: &str = "daemon.lock";

/// Errors raised while starting or running the daemon.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The configuration cannot be used.
    #[error(transparent)]
    Config(#[from] markoo_core::config::ConfigError),
    /// File access failed.
    #[error(transparent)]
    Fs(#[from] FsError),
    /// Token storage failed.
    #[error(transparent)]
    Token(#[from] token::TokenError),
    /// Another daemon uses this data directory.
    #[error("another daemon is already running on this data directory")]
    AlreadyRunning,
    /// The API port cannot be used.
    #[error("cannot listen on port {port}: {source}")]
    Bind {
        /// The port.
        port: u16,
        /// Underlying error.
        source: std::io::Error,
    },
    /// The async runtime could not start.
    #[error("cannot start the runtime: {0}")]
    Runtime(std::io::Error),
}

/// Tuning of a daemon. Tests shorten the delays.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// How often the configuration file is checked for changes.
    pub config_poll: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            config_poll: Duration::from_secs(2),
        }
    }
}

/// A started daemon.
#[derive(Debug)]
pub struct Daemon {
    addr: Option<SocketAddr>,
    queue: Queue,
    stop: watch::Sender<bool>,
    tasks: Vec<JoinHandle<()>>,
    _lock: FsLock,
}

impl Daemon {
    /// Starts the scheduler and, if enabled, the API on `127.0.0.1`.
    pub async fn start(paths: Paths, options: Options) -> Result<Self, Error> {
        let lock = DiskFs
            .try_lock(&paths.data_dir().join(DAEMON_LOCK))?
            .ok_or(Error::AlreadyRunning)?;
        let config = Config::load(paths.config_file())?;
        token::ensure(paths.data_dir())?;

        let queue = Queue::new(Arc::new(EngineRunner::new(paths.clone())));
        let (stop, stopped) = watch::channel(false);
        let mut tasks = Vec::new();
        let mut addr = None;
        if config.api.enabled {
            // The address is not configurable on purpose: the API stays local.
            let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, config.api.port))
                .await
                .map_err(|source| Error::Bind {
                    port: config.api.port,
                    source,
                })?;
            addr = Some(listener.local_addr().map_err(|source| Error::Bind {
                port: config.api.port,
                source,
            })?);
            let app = api::router(paths.clone(), queue.clone());
            let mut stopped = stopped.clone();
            tasks.push(tokio::spawn(async move {
                let shutdown = async move {
                    let _ = stopped.changed().await;
                };
                if let Err(error) = axum::serve(listener, app)
                    .with_graceful_shutdown(shutdown)
                    .await
                {
                    eprintln!("error: the API stopped: {error}");
                }
            }));
        }
        tasks.push(tokio::spawn(scheduler::run(
            paths,
            config,
            queue.clone(),
            options,
            stopped,
        )));
        Ok(Self {
            addr,
            queue,
            stop,
            tasks,
            _lock: lock,
        })
    }

    /// Address of the API, `None` if it is disabled.
    pub fn addr(&self) -> Option<SocketAddr> {
        self.addr
    }

    /// The job queue.
    pub fn queue(&self) -> &Queue {
        &self.queue
    }

    /// Stops the scheduler and the API, closing the event streams.
    pub async fn stop(self) {
        let _ = self.stop.send(true);
        self.queue.publish(Event::Shutdown);
        for task in self.tasks {
            let _ = task.await;
        }
    }
}

/// Runs a daemon until the process is asked to stop.
pub fn run(paths: Paths) -> Result<(), Error> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(Error::Runtime)?;
    // Dropping the runtime waits for the sync in progress, if any.
    runtime.block_on(async {
        let daemon = Daemon::start(paths, Options::default()).await?;
        match daemon.addr() {
            Some(addr) => println!("Markoo daemon listening on http://{addr}"),
            None => println!("Markoo daemon started (API disabled)"),
        }
        wait_for_stop().await;
        println!("Stopping…");
        daemon.stop().await;
        Ok(())
    })
}

#[cfg(unix)]
async fn wait_for_stop() {
    use tokio::signal::unix::{SignalKind, signal};
    // launchd stops its services with SIGTERM.
    match signal(SignalKind::terminate()) {
        Ok(mut terminate) => {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = terminate.recv() => {}
            }
        }
        Err(_) => {
            let _ = tokio::signal::ctrl_c().await;
        }
    }
}

#[cfg(not(unix))]
async fn wait_for_stop() {
    let _ = tokio::signal::ctrl_c().await;
}
