//! What the remote interfaces (HTTP API, MCP) can ask for, as JSON views.
//!
//! They only translate between their protocol and these functions, so that
//! both show the same data and apply the same checks.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use serde::Serialize;
use serde_json::{Map, Value, json};
use unicode_normalization::UnicodeNormalization;

use crate::apply::Report;
use crate::config::{Config, ConfigError, NewPair, PairEntry, Problem, expand_home};
use crate::engine::issue_id;
use crate::fs::DiskFs;
use crate::hooks::{self, GitRepo, HookChange, HookError, HookManager, HookSettings, HookTarget};
use crate::kb::KbType;
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
        "assets_dir": pair.assets_dir,
        "links": if pair.kb_options.convert_links { "markdown" } else { "keep" },
        "frontmatter": frontmatter_mode(pair),
        "frontmatter_drop": pair.kb_options.frontmatter_drop,
        "callouts": if pair.kb_options.convert_callouts { "github" } else { "keep" },
        "hooks": {
            "install": pair.hooks.install,
            "branches": pair.hooks.branches,
        },
    })
}

fn frontmatter_mode(pair: &Pair) -> &'static str {
    if pair.kb_options.frontmatter_drop_all {
        "drop"
    } else if pair.kb_options.frontmatter_drop.is_empty() {
        "keep"
    } else {
        "filter"
    }
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

/// Checks settings received from a client and puts them in text form.
fn setting_changes(settings: &Map<String, Value>) -> Result<Vec<(String, String)>, Error> {
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
    Ok(changes)
}

/// Changes settings of a pair; the answer includes a preview of the new plan.
pub fn update_config(
    paths: &Paths,
    name: &str,
    settings: &Map<String, Value>,
) -> Result<Value, Error> {
    let changes = setting_changes(settings)?;
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

/// The plan of a new pair, which is not saved.
pub fn preview_new_pair(paths: &Paths, new: &NewPair) -> Result<Value, Error> {
    check_folder("kb", &new.kb)?;
    check_folder("repo", &new.repo)?;
    let pair = manage::preview_new_pair(paths, new)?;
    preview_view(paths, &pair)
}

/// The plan of a pair with changed settings, which are not saved.
pub fn preview_config(
    paths: &Paths,
    name: &str,
    settings: &Map<String, Value>,
) -> Result<Value, Error> {
    let changes = setting_changes(settings)?;
    let pair = manage::preview_set_pair(paths, name, &changes)?;
    preview_view(paths, &pair)
}

/// How many examples a preview gives of the files that are not tracked by Git.
const UNTRACKED_EXAMPLES: usize = 5;

/// The plan, with what a user should look at before accepting it: where files
/// go, and how many of those sent to the KB are not in Git. Such files may have
/// nothing to do with the repository, left there by an earlier sync for example.
fn preview_view(paths: &Paths, pair: &Pair) -> Result<Value, Error> {
    let mut view = plan_view(&open_engine(paths)?, pair)?;
    let copies_to = |side: Side| -> Vec<String> {
        let side = json!(side);
        view["entries"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|entry| entry["kind"] == "copy" && entry["side"] == side)
            .filter_map(|entry| entry["path"].as_str().map(str::to_owned))
            .collect()
    };
    let to_kb = copies_to(Side::Kb);
    let to_repo = copies_to(Side::Repo).len();
    // Git and the file system may not encode accents the same way.
    let comparable = |text: &str| text.nfc().collect::<String>();
    let untracked = hooks::untracked_files(&pair.repo)
        .map_err(hook_error)?
        .map(|files| {
            files
                .iter()
                .map(|file| comparable(file))
                .collect::<BTreeSet<_>>()
        });
    let untracked_to_kb: Option<Vec<&String>> = untracked.as_ref().map(|untracked| {
        to_kb
            .iter()
            .filter(|path| untracked.contains(&comparable(path)))
            .collect()
    });
    let summary = json!({
        "to_kb": to_kb.len(),
        "to_repo": to_repo,
        "git": untracked.is_some(),
        "untracked_to_kb": untracked_to_kb.as_ref().map_or(0, Vec::len),
        "untracked_examples": untracked_to_kb
            .iter()
            .flatten()
            .take(UNTRACKED_EXAMPLES)
            .collect::<Vec<_>>(),
    });
    view["summary"] = summary;
    Ok(view)
}

/// Most files listed by a filter preview, which also gives their real number.
const PREVIEW_FILES: usize = 200;

fn filter_view(paths: &Paths, pair: &Pair) -> Result<Value, Error> {
    let preview = open_engine(paths)?.filter_preview(pair)?;
    let listed = |files: &[EntryPath]| -> Vec<String> {
        files
            .iter()
            .take(PREVIEW_FILES)
            .map(ToString::to_string)
            .collect()
    };
    Ok(json!({
        "tracked_count": preview.tracked.len(),
        "excluded_count": preview.excluded.len(),
        "tracked": listed(&preview.tracked),
        "excluded": listed(&preview.excluded),
    }))
}

/// Which files a pair would track with these settings, which are not saved.
pub fn preview_filters(
    paths: &Paths,
    name: &str,
    settings: &Map<String, Value>,
) -> Result<Value, Error> {
    let changes = setting_changes(settings)?;
    let pair = manage::preview_set_pair(paths, name, &changes)?;
    filter_view(paths, &pair)
}

/// Which files a new pair would track; it is not saved.
pub fn preview_new_pair_filters(paths: &Paths, new: &NewPair) -> Result<Value, Error> {
    check_folder("kb", &new.kb)?;
    check_folder("repo", &new.repo)?;
    let pair = manage::preview_new_pair(paths, new)?;
    filter_view(paths, &pair)
}

/// A note before and after conversion with these settings, which are not saved.
pub fn preview_transform(
    paths: &Paths,
    name: &str,
    settings: &Map<String, Value>,
) -> Result<Value, Error> {
    let changes = setting_changes(settings)?;
    let pair = manage::preview_set_pair(paths, name, &changes)?;
    Ok(json!(open_engine(paths)?.transform_sample(&pair)?))
}

/// Settings that apply to every pair.
pub fn global_settings(paths: &Paths) -> Result<Value, Error> {
    let config = Config::load(paths.config_file())?;
    Ok(json!({
        "interval": config.schedule.interval.map_or_else(
            || "off".to_owned(),
            |interval| format!("{}s", interval.as_secs()),
        ),
        "on_startup": config.schedule.on_startup,
        "api_enabled": config.api.enabled,
        "api_port": config.api.port,
        "mcp_http": config.mcp.http,
        "mcp_allow_config_write": config.mcp.allow_config_write,
    }))
}

/// Changes settings that apply to every pair. Numbers and booleans are given as text.
pub fn update_global_settings(paths: &Paths, settings: &Map<String, Value>) -> Result<(), Error> {
    let changes = settings
        .iter()
        .map(|(key, value)| Ok((key.clone(), setting_text(key, value)?)))
        .collect::<Result<Vec<_>, Error>>()?;
    let _edit = config_edit();
    manage::set_global(paths, &changes)
}

/// The hooks of a pair found in its repository, and what edits them.
pub fn hooks_state(paths: &Paths, name: &str) -> Result<Value, Error> {
    let config = Config::load(paths.config_file())?;
    let pair = &config.pair(name)?.pair;
    let Ok(repo) = GitRepo::open(&pair.repo) else {
        return Ok(json!({"git": false, "installed": [], "manager": null}));
    };
    let installed: Vec<String> = hooks::files_with_block(&DiskFs, &repo, name)
        .map_err(hook_error)?
        .iter()
        .filter_map(|file| file.file_name())
        .map(|file| file.to_string_lossy().into_owned())
        .collect();
    let manager = hooks::detect_manager(&DiskFs, &repo).map_err(hook_error)?;
    Ok(json!({
        "git": true,
        "installed": installed,
        "manager": manager.map(|manager| match manager {
            HookManager::Husky => "husky".to_owned(),
            HookManager::Lefthook => "lefthook".to_owned(),
            HookManager::HooksPath(path) => format!("core.hooksPath ({path})"),
        }),
    }))
}

fn hook_error(error: HookError) -> Error {
    Error::InvalidRequest(error.to_string())
}

/// Installs the hooks of a pair; they run this `markoo` binary.
pub fn install_hooks(paths: &Paths, name: &str, program: &Path) -> Result<Value, Error> {
    edit_hooks(paths, name, program, hooks::install)
}

/// Removes the hooks of a pair.
pub fn uninstall_hooks(paths: &Paths, name: &str, program: &Path) -> Result<Value, Error> {
    edit_hooks(paths, name, program, hooks::uninstall)
}

fn edit_hooks(
    paths: &Paths,
    name: &str,
    program: &Path,
    edit: impl Fn(
        &DiskFs,
        &GitRepo,
        &HookTarget<'_>,
        &HookSettings,
    ) -> Result<Vec<HookChange>, HookError>,
) -> Result<Value, Error> {
    let config = Config::load(paths.config_file())?;
    let pair = &config.pair(name)?.pair;
    let repo = GitRepo::open(&pair.repo).map_err(hook_error)?;
    let target = HookTarget {
        pair: name,
        program,
        config_file: paths.config_file(),
        data_dir: paths.data_dir(),
    };
    let changes = edit(&DiskFs, &repo, &target, &pair.hooks).map_err(hook_error)?;
    Ok(json!(changes))
}

/// What a folder would be as a knowledge base.
pub fn inspect_kb(path: &str) -> Value {
    let folder = expand_home(path);
    json!({
        "exists": folder.is_dir(),
        "kb_type": KbType::detect(&folder).id(),
    })
}

/// Whether a folder can be the repo side, and where it stands in Git.
pub fn inspect_repo(path: &str) -> Value {
    let folder = expand_home(path);
    let repo = GitRepo::open(&folder).ok();
    json!({
        "exists": folder.is_dir(),
        "git": repo.is_some(),
        "top_level": repo.as_ref().map(|repo| &repo.top_level),
        "branch": repo.as_ref().and_then(|repo| repo.branch.as_ref()),
    })
}

/// Folders known to the knowledge base tools, suggested for a new pair.
pub fn known_kbs() -> Vec<PathBuf> {
    crate::kb::known_kbs()
}

/// Text to edit by hand to settle an issue, with conflict markers.
pub fn conflict_draft(paths: &Paths, id: &str) -> Result<String, Error> {
    let (pair, path) = find_issue(paths, id)?;
    open_engine(paths)?.conflict_draft(&pair, &path)
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

/// The most recent syncs of a pair, the latest first.
pub fn history(paths: &Paths, name: &str, limit: u32) -> Result<Value, Error> {
    Config::load(paths.config_file())?.pair(name)?;
    Ok(json!(open_engine(paths)?.history(name, limit)?))
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
pub fn run_sync(
    paths: &Paths,
    name: &str,
    dry_run: bool,
    trigger: &str,
) -> Result<SyncOutcome, Error> {
    let pair = usable_pair(paths, name)?;
    if pair.paused {
        return Err(Error::Paused(name.to_owned()));
    }
    let outcome = open_engine(paths)?.run_recorded(&pair, dry_run, trigger)?;
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
