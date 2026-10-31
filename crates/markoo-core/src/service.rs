//! What the remote interfaces (HTTP API, MCP) can ask for, as JSON views.
//!
//! They only translate between their protocol and these functions, so that
//! both show the same data and apply the same checks.

use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};

use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::apply::Report;
use crate::config::{Config, ConfigError, PairEntry, Problem};
use crate::engine::issue_id;
use crate::fs::DiskFs;
use crate::paths::Paths;
use crate::plan::{Action, Plan};
use crate::{Engine, EntryPath, Error, Pair, Resolution, Side, manage};

/// Requests changing `markoo.toml` wait for each other instead of overwriting
/// each other's edits.
static CONFIG_EDIT: Mutex<()> = Mutex::new(());

/// Holds off the other edits of `markoo.toml` in this process.
pub fn config_edit() -> MutexGuard<'static, ()> {
    CONFIG_EDIT.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What a sync did.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SyncOutcome {
    /// Type of the pair's knowledge base.
    pub kb_type: String,
    /// The direction changed, so the sync only computed its plan.
    pub direction_changed: bool,
    /// Counts of what was done.
    pub summary: Value,
    /// Issues waiting for the user.
    pub conflicts: Vec<Value>,
    /// Details of what was applied, `null` for a dry run.
    pub report: Value,
}

impl SyncOutcome {
    /// Number of issues waiting for the user.
    pub fn conflict_count(&self) -> usize {
        self.conflicts.len()
    }
}

fn open_engine(paths: &Paths) -> Result<Engine<DiskFs>, Error> {
    Engine::open(DiskFs, paths)
}

/// A pair that can be used: it exists and passes validation.
pub fn usable_pair(paths: &Paths, name: &str) -> Result<Pair, Error> {
    let config = Config::load(paths.config_file())?;
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

/// Settings of a pair.
pub fn pair_view(entry: &PairEntry) -> Value {
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

/// Settings of every pair, with what is pending and what prevents it from syncing.
pub fn list_pairs(paths: &Paths) -> Result<Vec<Value>, Error> {
    let config = Config::load(paths.config_file())?;
    let validation = config.validate();
    let engine = open_engine(paths)?;
    let mut views = Vec::with_capacity(config.pairs.len());
    for entry in &config.pairs {
        let name = entry.pair.name.as_str();
        let mut view = pair_view(entry);
        view["pending"] = json!(engine.pending_count(&entry.pair)?);
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
    Ok(views)
}

/// Settings of a pair.
pub fn pair_config(paths: &Paths, name: &str) -> Result<Value, Error> {
    let config = Config::load(paths.config_file())?;
    Ok(pair_view(config.pair(name)?))
}

fn plan_view(engine: &Engine<DiskFs>, pair: &Pair) -> Result<Value, Error> {
    let planned = engine.plan(pair)?;
    let entries: Vec<_> = planned
        .plan
        .entries
        .iter()
        .filter(|entry| entry.is_visible())
        .map(crate::plan::PlannedEntry::view)
        .collect();
    Ok(json!({
        "pair": pair.name,
        "direction": pair.direction,
        "paused": pair.paused,
        "entries": entries,
        "attachments": planned.plan.attachments,
        "scan_issues": planned.scan_issues,
    }))
}

/// The current plan of a pair; nothing is applied.
pub fn status(paths: &Paths, name: &str) -> Result<Value, Error> {
    let pair = usable_pair(paths, name)?;
    plan_view(&open_engine(paths)?, &pair)
}

/// The current plan of every pair, or the error that prevented computing it.
pub fn status_of_all(paths: &Paths) -> Result<Vec<Value>, Error> {
    let config = Config::load(paths.config_file())?;
    Ok(config
        .pairs
        .iter()
        .map(|entry| {
            let name = entry.pair.name.as_str();
            status(paths, name)
                .unwrap_or_else(|error| json!({"pair": name, "error": error.to_string()}))
        })
        .collect())
}

/// A path received from a client, confined to the pair's folders.
pub fn entry_path(path: &str) -> Result<EntryPath, Error> {
    EntryPath::from_relative(Path::new(path)).ok_or_else(|| {
        Error::InvalidRequest(format!("{path} must be relative to the pair's folders"))
    })
}

/// Differences between both sides of a pair, optionally of one file only.
pub fn diff(paths: &Paths, name: &str, path: Option<&str>) -> Result<Value, Error> {
    let only = path.map(entry_path).transpose()?;
    let pair = usable_pair(paths, name)?;
    let diffs = open_engine(paths)?.diffs(&pair, only.as_ref())?;
    Ok(json!({"pair": name, "diffs": diffs}))
}

/// Whether a folder can be written to the configuration by a remote client.
pub fn check_folder(field: &str, path: &str) -> Result<(), Error> {
    if manage::is_safe_folder(path) {
        return Ok(());
    }
    Err(Error::InvalidRequest(format!(
        "{field} must be an absolute path without \"..\""
    )))
}

/// Turns a JSON value into the text form of `ConfigFile::set`.
fn setting_text(key: &str, value: &Value) -> Result<String, Error> {
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
    converted.ok_or_else(|| Error::InvalidRequest(format!("invalid value for {key}")))
}

/// Messages of some problems.
pub fn warnings_view(warnings: &[Problem]) -> Vec<&str> {
    warnings
        .iter()
        .map(|problem| problem.message.as_str())
        .collect()
}

/// Changes settings of a pair; the answer includes a preview of the new plan.
pub fn update_config(
    paths: &Paths,
    name: &str,
    settings: &Map<String, Value>,
) -> Result<Value, Error> {
    let mut changes = Vec::with_capacity(settings.len());
    for (key, value) in settings {
        // The state is stored under the name: renaming would lose the base.
        if key == "name" {
            return Err(Error::InvalidRequest("a pair cannot be renamed".to_owned()));
        }
        let text = setting_text(key, value)?;
        if key == "kb" || key == "repo" {
            check_folder(key, &text)?;
        }
        changes.push((key.clone(), text));
    }
    let warnings = {
        let _edit = config_edit();
        manage::set_pair(paths, name, &changes)?
    };
    let config = Config::load(paths.config_file())?;
    let entry = config.pair(name)?;
    let preview = plan_view(&open_engine(paths)?, &entry.pair);
    Ok(json!({
        "pair": pair_view(entry),
        "warnings": warnings_view(&warnings),
        "plan": preview.as_ref().ok(),
        "plan_error": preview.as_ref().err().map(ToString::to_string),
    }))
}

/// Pending conflicts and divergences, of one pair or of all of them, with the
/// KB, base and repo versions.
pub fn conflicts(paths: &Paths, only: Option<&str>) -> Result<Value, Error> {
    let config = Config::load(paths.config_file())?;
    if let Some(name) = only {
        config.pair(name)?;
    }
    let validation = config.validate();
    let engine = open_engine(paths)?;
    let (mut conflicts, mut excluded, mut errors) = (Vec::new(), Vec::new(), Vec::new());
    for entry in &config.pairs {
        let pair = &entry.pair;
        if only.is_some_and(|name| name != pair.name) || !validation.is_ok(&pair.name) {
            continue;
        }
        match engine.issues(pair) {
            Ok(issues) => {
                let bases = engine.bases(pair)?;
                conflicts.extend(issues.iter().map(|issue| {
                    json!({
                        "id": issue_id(&pair.name, &issue.path),
                        "pair": pair.name,
                        "path": issue.path,
                        "kind": issue.kind,
                        "kb": issue.kb,
                        "base": bases.get(&issue.path).map(|base| &base.content),
                        "repo": issue.repo,
                    })
                }));
            }
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
    Ok(json!({
        "conflicts": conflicts,
        "excluded": excluded,
        "errors": errors,
    }))
}

/// One pending conflict or divergence.
pub fn conflict(paths: &Paths, id: &str) -> Result<Value, Error> {
    let mut all = conflicts(paths, None)?;
    all["conflicts"]
        .as_array_mut()
        .and_then(|issues| {
            let index = issues.iter().position(|issue| issue["id"] == id)?;
            Some(issues.swap_remove(index))
        })
        .ok_or_else(|| Error::UnknownIssue(id.to_owned()))
}

/// How a client wants to settle an issue.
pub fn resolution(
    keep: Option<&str>,
    content: Option<String>,
    push_to_source: bool,
) -> Result<Resolution, Error> {
    match (keep, content, push_to_source) {
        (Some("kb"), None, false) => Ok(Resolution::Keep(Side::Kb)),
        (Some("repo"), None, false) => Ok(Resolution::Keep(Side::Repo)),
        (Some("both"), None, false) => Ok(Resolution::KeepBoth),
        (None, Some(content), false) => Ok(Resolution::Content(content)),
        (None, None, true) => Ok(Resolution::PushToSource),
        _ => Err(Error::InvalidRequest(
            "give one of keep (kb, repo or both), content or push_to_source".to_owned(),
        )),
    }
}

/// Finds which pair and file an issue identifier stands for. Scanning the
/// pending issues means that a client can never name a file outside of them.
fn find_issue(paths: &Paths, id: &str) -> Result<(Pair, EntryPath), Error> {
    let config = Config::load(paths.config_file())?;
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
    Err(Error::UnknownIssue(id.to_owned()))
}

/// Settles a conflict or a divergence.
pub fn resolve(paths: &Paths, id: &str, resolution: Resolution) -> Result<Value, Error> {
    let (pair, path) = find_issue(paths, id)?;
    let report = open_engine(paths)?.resolve(&pair, &path, resolution)?;
    Ok(json!({"pair": pair.name, "path": path, "report": report}))
}

/// The pairs a sync of everything applies to.
pub fn runnable_pairs(paths: &Paths) -> Result<Vec<String>, Error> {
    let config = Config::load(paths.config_file())?;
    let validation = config.validate();
    Ok(config
        .pairs
        .iter()
        .filter(|entry| !entry.pair.paused && validation.is_ok(&entry.pair.name))
        .map(|entry| entry.pair.name.clone())
        .collect())
}

/// Syncs a pair through the engine. The configuration is read here: a queued
/// sync may have waited behind a change.
pub fn run_sync(paths: &Paths, name: &str, dry_run: bool) -> Result<SyncOutcome, Error> {
    let pair = usable_pair(paths, name)?;
    if pair.paused {
        return Err(Error::Paused(name.to_owned()));
    }
    let outcome = open_engine(paths)?.run(&pair, dry_run)?;
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
    Ok(SyncOutcome {
        kb_type: pair.kb_type.id().to_owned(),
        direction_changed: outcome.direction_changed,
        summary: summary(plan, outcome.report.as_ref()),
        conflicts,
        report: json!(outcome.report),
    })
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
