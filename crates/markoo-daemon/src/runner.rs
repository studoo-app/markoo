//! Runs a queued job through the engine.

use markoo_core::paths::Paths;
use markoo_core::service;

use crate::queue::{JobOutcome, Runner};

/// The runner used in production: every job opens its own engine, because
/// the state database cannot be shared across threads.
#[derive(Debug)]
pub struct EngineRunner {
    paths: Paths,
}

impl EngineRunner {
    /// Creates a runner on these locations.
    pub fn new(paths: Paths) -> Self {
        Self { paths }
    }
}

impl Runner for EngineRunner {
    fn run(&self, name: &str, dry_run: bool) -> Result<JobOutcome, String> {
        service::run_sync(&self.paths, name, dry_run).map_err(|error| error.to_string())
    }
}
