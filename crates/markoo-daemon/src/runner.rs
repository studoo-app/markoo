//! Runs a queued job through the engine.

use markoo_core::Engine;
use markoo_core::Side;
use markoo_core::apply::Report;
use markoo_core::config::Config;
use markoo_core::engine::issue_id;
use markoo_core::fs::DiskFs;
use markoo_core::paths::Paths;
use markoo_core::plan::{Action, Plan};
use serde_json::{Value, json};

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
        // The file is read again here: the job may have waited behind a change.
        let config = Config::load(self.paths.config_file()).map_err(|e| e.to_string())?;
        let entry = config.pair(name).map_err(|e| e.to_string())?;
        let pair = &entry.pair;
        if pair.paused {
            return Err(format!("pair \"{name}\" is paused"));
        }
        let validation = config.validate();
        if !validation.is_ok(name) {
            let messages: Vec<&str> = validation
                .errors
                .iter()
                .filter(|problem| problem.pair == name)
                .map(|problem| problem.message.as_str())
                .collect();
            return Err(format!("pair \"{name}\": {}", messages.join("; ")));
        }
        let engine = Engine::open(DiskFs, &self.paths).map_err(|e| e.to_string())?;
        let outcome = engine.run(pair, dry_run).map_err(|e| e.to_string())?;
        let plan = &outcome.planned.plan;
        let conflicts = plan
            .entries
            .iter()
            .filter_map(|entry| match &entry.action {
                Action::Pending { issue, .. } => Some(json!({
                    "id": issue_id(name, &entry.path),
                    "path": entry.path,
                    "kind": issue,
                })),
                _ => None,
            })
            .collect();
        Ok(JobOutcome {
            kb_type: pair.kb_type.id().to_owned(),
            direction_changed: outcome.direction_changed,
            summary: summary(plan, outcome.report.as_ref()),
            conflicts,
            report: json!(outcome.report),
        })
    }
}

/// Counts what the plan does, leaving out what the report says was skipped.
fn summary(plan: &Plan, report: Option<&Report>) -> Value {
    let (mut kb_to_repo, mut repo_to_kb, mut merged, mut deleted, mut conflicts) = (0, 0, 0, 0, 0);
    for entry in &plan.entries {
        let skipped = report.is_some_and(|report| {
            report
                .skipped
                .iter()
                .any(|skipped| skipped.path == entry.path)
        });
        if skipped {
            continue;
        }
        match &entry.action {
            Action::Copy { to: Side::Repo, .. }
            | Action::Rename {
                side: Side::Repo, ..
            } => {
                kb_to_repo += 1;
            }
            Action::Copy { to: Side::Kb, .. } | Action::Rename { side: Side::Kb, .. } => {
                repo_to_kb += 1;
            }
            Action::Merge { .. } => merged += 1,
            Action::Trash { .. } => deleted += 1,
            Action::Pending { .. } => conflicts += 1,
            Action::RecordBase { .. } | Action::ForgetBase | Action::NotInSource { .. } => {}
        }
    }
    json!({
        "kb_to_repo": kb_to_repo,
        "repo_to_kb": repo_to_kb,
        "merged": merged,
        "conflicts": conflicts,
        "deleted": deleted,
    })
}
