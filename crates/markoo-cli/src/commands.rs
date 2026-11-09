//! Implementation of the commands. Everything goes through `markoo-core`;
//! this module only parses intentions and prints results.

use std::cell::Cell;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::Command as Process;

use anyhow::{Context, Result, bail};
use markoo_core::config::{Config, ConfigError, ConfigFile, NewPair, Problem, expand_home};
use markoo_core::fs::{DiskFs, Fs};
use markoo_core::hooks::{self, GitRepo, HookChange, HookName, HookStatus, HookTarget};
use markoo_core::paths::{Overrides, Paths};
use markoo_core::plan::{Action, AttachmentAction, PlannedAttachment, PlannedEntry};
use markoo_core::state::IssueKind;
use markoo_core::{Direction, Engine, EntryPath, Pair, Report, Resolution, Side, TargetChanges};
use markoo_core::{manage, service};
use markoo_daemon::client::{self, SyncRequest};
use serde_json::{Value, json};

use crate::output::Style;
use crate::{DaemonCommand, HookCommand, Keep, PairCommand, wizard};

/// How the user wants to settle an issue.
#[derive(Debug)]
pub enum How {
    Keep(Keep),
    Edit,
    ContentFile(PathBuf),
    PushToSource,
}

impl How {
    /// clap guarantees that exactly one flag is set.
    pub fn from_flags(
        keep: Option<Keep>,
        edit: bool,
        content_file: Option<PathBuf>,
        push_to_source: bool,
    ) -> Self {
        match (keep, content_file) {
            (Some(keep), _) => How::Keep(keep),
            (None, Some(file)) => How::ContentFile(file),
            (None, None) if edit => How::Edit,
            (None, None) if push_to_source => How::PushToSource,
            (None, None) => How::Edit,
        }
    }
}

/// What a command needs to run.
pub struct Ctx {
    paths: Paths,
    json: bool,
    style: Style,
    failed: Cell<bool>,
}

impl Ctx {
    pub fn new(
        config: Option<PathBuf>,
        data_dir: Option<PathBuf>,
        json: bool,
        style: Style,
    ) -> Result<Self> {
        let paths = Paths::resolve(&Overrides {
            config_file: config,
            data_dir,
        })?;
        Ok(Self {
            paths,
            json,
            style,
            failed: Cell::new(false),
        })
    }

    pub fn succeeded(&self) -> bool {
        !self.failed.get()
    }

    fn engine(&self) -> Result<Engine<DiskFs>> {
        Ok(Engine::open(DiskFs, &self.paths)?)
    }

    fn config(&self) -> Result<Config> {
        Ok(Config::load(self.paths.config_file())?)
    }

    fn print_json(&self, value: &Value) -> Result<()> {
        println!("{}", serde_json::to_string_pretty(value)?);
        Ok(())
    }

    fn report_error(&self, message: &str) {
        self.failed.set(true);
        eprintln!("{} {message}", self.style.red("error:"));
    }

    fn warn(&self, message: &str) {
        eprintln!("{} {message}", self.style.yellow("warning:"));
    }

    /// The pairs a command applies to, skipping invalid ones after saying
    /// why. `only` is what the user typed: an id, `project/name` or a name.
    fn selected(&self, config: &Config, only: Option<&str>) -> Result<Vec<Pair>> {
        let only = only
            .map(|reference| config.pair(reference).map(|entry| entry.pair.id.as_str()))
            .transpose()?;
        Ok(self.valid_pairs(config, |id| only.is_none_or(|only| only == id)))
    }

    /// Like [`Ctx::selected`], or the pairs of `project` when it is given.
    fn selected_in(
        &self,
        config: &Config,
        only: Option<&str>,
        project: Option<&str>,
    ) -> Result<Vec<Pair>> {
        let Some(project) = project else {
            return self.selected(config, only);
        };
        let members = service::project_pairs(&self.paths, project)?;
        Ok(self.valid_pairs(config, |name| members.iter().any(|member| member == name)))
    }

    /// The pairs kept by `keep` that can be used, reporting the others.
    fn valid_pairs(&self, config: &Config, keep: impl Fn(&str) -> bool) -> Vec<Pair> {
        let validation = config.validate();
        let mut pairs = Vec::new();
        for entry in &config.pairs {
            let id = entry.pair.id.as_str();
            if !keep(id) {
                continue;
            }
            let label = entry.pair.label();
            for problem in validation.warnings.iter().filter(|p| p.pair == id) {
                self.warn(&format!("pair \"{label}\": {}", problem.message));
            }
            if validation.is_ok(id) {
                pairs.push(entry.pair.clone());
            } else {
                for problem in validation.errors.iter().filter(|p| p.pair == id) {
                    self.report_error(&format!("pair \"{label}\": {}", problem.message));
                }
            }
        }
        pairs
    }

    /// Asks the questions of a new pair on the terminal.
    fn ask_new_pair(&self) -> Result<NewPair> {
        let config = if self.paths.config_file().exists() {
            Some(self.config()?)
        } else {
            None
        };
        let projects = config.as_ref().map(Config::projects).unwrap_or_default();
        let taken: Vec<(&str, &str)> = config
            .iter()
            .flat_map(|config| &config.pairs)
            .filter_map(|entry| Some((entry.pair.project.as_deref()?, entry.pair.name.as_str())))
            .collect();
        let mut input = std::io::stdin().lock();
        let mut output = std::io::stdout();
        wizard::ask_new_pair(&mut input, &mut output, &projects, &taken)
    }

    pub fn init(&self) -> Result<()> {
        print!("{}", self.style.banner());
        let config = self.paths.config_file();
        match ConfigFile::create(config) {
            Ok(()) => println!("Created {}", config.display()),
            Err(ConfigError::AlreadyExists(_)) => {
                println!("Keeping the existing {}", config.display());
            }
            Err(error) => return Err(error.into()),
        }
        self.engine()?;
        println!("State folder: {}", self.paths.data_dir().display());
        if std::io::stdin().is_terminal() {
            println!("\nLet's add your first pair.");
            let new = self.ask_new_pair()?;
            self.add_pair(new)?;
        } else {
            println!(
                "Next: markoo pair add \"<name>\" --project <project> --kb <folder> --repo <folder>"
            );
        }
        Ok(())
    }

    pub fn status(&self, only: Option<&str>, project: Option<&str>) -> Result<()> {
        let config = self.config()?;
        let engine = self.engine()?;
        let mut views = Vec::new();
        for pair in self.selected_in(&config, only, project)? {
            match engine.plan(&pair) {
                Ok(planned) => {
                    let visible: Vec<&PlannedEntry> = visible_entries(&planned.plan.entries);
                    if self.json {
                        views.push(json!({
                            "pair": pair.id,
                            "direction": pair.direction,
                            "paused": pair.paused,
                            "entries": visible.iter().map(|e| entry_json(e)).collect::<Vec<_>>(),
                            "attachments": planned.plan.attachments,
                            "scan_issues": planned.scan_issues,
                        }));
                    } else {
                        self.pair_header(&pair);
                        self.print_entries(&visible);
                        self.print_attachments(&planned.plan.attachments);
                        for issue in &planned.scan_issues {
                            self.warn(&format!("{}: {}", issue.path.display(), issue.message));
                        }
                    }
                }
                Err(error) => self.report_error(&error.to_string()),
            }
        }
        if self.json {
            self.print_json(&Value::Array(views))?;
        }
        Ok(())
    }

    pub fn diff(&self, file: Option<&str>, only: Option<&str>) -> Result<()> {
        let config = self.config()?;
        let engine = self.engine()?;
        let target = file.map(entry_path).transpose()?;
        let mut views = Vec::new();
        for pair in self.selected(&config, only)? {
            let diffs = match engine.diffs(&pair, target.as_ref()) {
                Ok(diffs) => diffs,
                Err(error) => {
                    self.report_error(&error.to_string());
                    continue;
                }
            };
            for diff in diffs {
                if self.json {
                    views.push(json!({"pair": pair.id, "path": diff.path, "patch": diff.patch}));
                } else {
                    self.print_patch(&diff.path, &diff.patch);
                }
            }
        }
        if self.json {
            self.print_json(&Value::Array(views))?;
        }
        Ok(())
    }

    pub fn sync(&self, dry_run: bool, only: Option<&str>, project: Option<&str>) -> Result<()> {
        let config = self.config()?;
        let engine = self.engine()?;
        let mut views = Vec::new();
        for pair in self.selected_in(&config, only, project)? {
            if pair.paused {
                if self.json {
                    views.push(json!({"pair": pair.id, "paused": true}));
                } else {
                    self.pair_header(&pair);
                    println!("  {}", self.style.dim("paused"));
                }
                continue;
            }
            let outcome = match engine.run_recorded(&pair, dry_run, "cli") {
                Ok(outcome) => outcome,
                Err(error) => {
                    self.report_error(&error.to_string());
                    continue;
                }
            };
            let visible = visible_entries(&outcome.planned.plan.entries);
            if self.json {
                views.push(json!({
                    "pair": pair.id,
                    "dry_run": outcome.report.is_none(),
                    "direction_changed": outcome.direction_changed,
                    "entries": visible.iter().map(|e| entry_json(e)).collect::<Vec<_>>(),
                    "attachments": outcome.planned.plan.attachments,
                    "report": outcome.report,
                    "scan_issues": outcome.planned.scan_issues,
                }));
                continue;
            }
            self.pair_header(&pair);
            self.print_entries(&visible);
            self.print_attachments(&outcome.planned.plan.attachments);
            for issue in &outcome.planned.scan_issues {
                self.warn(&format!("{}: {}", issue.path.display(), issue.message));
            }
            if outcome.direction_changed {
                println!(
                    "  {}",
                    self.style.yellow(
                        "The direction changed: nothing was written. Sync again to apply this plan."
                    )
                );
            } else if let Some(report) = &outcome.report {
                self.print_report(report);
            } else if !visible.is_empty() {
                println!("  {}", self.style.dim("Dry run: nothing was written."));
            }
        }
        if self.json {
            self.print_json(&Value::Array(views))?;
        }
        Ok(())
    }

    pub fn conflicts(&self, only: Option<&str>) -> Result<()> {
        let config = self.config()?;
        let engine = self.engine()?;
        let mut views = Vec::new();
        let mut total = 0;
        for pair in self.selected(&config, only)? {
            let issues = match engine.issues(&pair) {
                Ok(issues) => issues,
                Err(error) => {
                    self.report_error(&error.to_string());
                    continue;
                }
            };
            let excluded = engine.ignored(&pair)?;
            total += issues.len();
            for issue in &issues {
                if self.json {
                    views.push(json!({
                        "pair": pair.id,
                        "path": issue.path,
                        "kind": issue.kind,
                        "kb": issue.kb,
                        "repo": issue.repo,
                    }));
                } else {
                    println!(
                        "{} {}  {}",
                        self.style.red("!"),
                        issue.path,
                        self.style
                            .dim(&format!("{} in {}", issue_label(issue.kind), pair.label()))
                    );
                }
            }
            for path in &excluded {
                if self.json {
                    views.push(json!({"pair": pair.id, "path": path, "kind": "excluded"}));
                } else {
                    println!(
                        "{} {path}  {}",
                        self.style.dim("-"),
                        self.style
                            .dim(&format!("excluded from sync in {}", pair.label()))
                    );
                }
            }
        }
        if self.json {
            self.print_json(&Value::Array(views))?;
        } else if total == 0 {
            println!("No conflicts.");
        } else {
            println!("\nSettle one with: markoo resolve <file> --keep kb|repo|both, or --edit");
        }
        Ok(())
    }

    pub fn resolve(&self, file: &str, only: Option<&str>, how: How) -> Result<()> {
        let config = self.config()?;
        let engine = self.engine()?;
        let path = entry_path(file)?;
        let pair = self.pair_with_issue(&config, &engine, &path, only)?;
        let resolution = match how {
            How::Keep(Keep::Kb) => Resolution::Keep(Side::Kb),
            How::Keep(Keep::Repo) => Resolution::Keep(Side::Repo),
            How::Keep(Keep::Both) => Resolution::KeepBoth,
            How::PushToSource => Resolution::PushToSource,
            How::ContentFile(file) => {
                let text = std::fs::read_to_string(&file)
                    .with_context(|| format!("cannot read {}", file.display()))?;
                Resolution::Content(text)
            }
            How::Edit => {
                if pair.direction != Direction::Both {
                    bail!("--edit needs a bidirectional pair; use --keep, or --push-to-source");
                }
                Resolution::Content(self.edit_draft(&engine, &pair, &path)?)
            }
        };
        let report = engine.resolve(&pair, &path, resolution)?;
        if self.json {
            return self.print_json(&json!({"pair": pair.id, "path": path, "report": report}));
        }
        if report.skipped.is_empty() {
            println!("{} {path} resolved.", self.style.green("✓"));
        }
        self.print_report(&report);
        Ok(())
    }

    pub fn history(&self, reference: &str, limit: u32) -> Result<()> {
        let config = self.config()?;
        let pair = &config.pair(reference)?.pair;
        let runs = self.engine()?.history(&pair.id, limit)?;
        if self.json {
            return self.print_json(&json!(runs));
        }
        if runs.is_empty() {
            println!("No sync of {} has changed anything yet.", pair.label());
            return Ok(());
        }
        let reports: Vec<Option<Report>> = runs
            .iter()
            .map(|run| {
                run.report
                    .clone()
                    .and_then(|report| serde_json::from_value(report).ok())
            })
            .collect();
        let reverted: Vec<i64> = reports
            .iter()
            .flatten()
            .filter(|report| report.run.is_some())
            .filter_map(|report| report.reverts)
            .collect();
        let now = now_millis();
        for (run, report) in runs.iter().zip(&reports) {
            let what = match (&run.error, report) {
                (Some(error), _) => self.style.red(&format!("failed: {error}")),
                (None, Some(report)) => report_summary(report),
                (None, None) => String::new(),
            };
            let mut note = String::new();
            if reverted.contains(&run.id) {
                note = self.style.dim("  (reverted)");
            } else if let Some(reverts) = report.as_ref().and_then(|report| report.reverts) {
                note = self.style.dim(&format!("  (reverts #{reverts})"));
            }
            println!(
                "{}  {}  {}  {what}{note}",
                self.style.bold(&format!("#{}", run.id)),
                self.style.dim(&age(now.saturating_sub(run.started_at))),
                self.style.dim(&run.trigger),
            );
        }
        println!("\nPut a sync back with: markoo revert {} <number>", pair.id);
        Ok(())
    }

    pub fn revert(&self, reference: &str, run: i64) -> Result<()> {
        let pair = service::usable_pair(&self.paths, reference)?;
        let report = self.engine()?.revert(&pair, run)?;
        if self.json {
            return self.print_json(&json!({"pair": pair.id, "run": run, "report": report}));
        }
        let done = report.written.len() + report.trashed.len();
        println!(
            "{} {done} file(s) put back from sync #{run}.",
            self.style.green("✓")
        );
        self.print_report(&report);
        if done > 0 {
            println!(
                "  {}",
                self.style
                    .dim("The next sync carries the old versions to the other side.")
            );
        }
        Ok(())
    }

    /// Finds the pair of the file: `--pair`, or the only one with an issue on it.
    fn pair_with_issue(
        &self,
        config: &Config,
        engine: &Engine<DiskFs>,
        path: &EntryPath,
        only: Option<&str>,
    ) -> Result<Pair> {
        let mut pairs = self.selected(config, only)?;
        if only.is_some() {
            return pairs
                .pop()
                .context("this pair cannot be used until the errors above are fixed");
        }
        let mut matching = Vec::new();
        for pair in pairs {
            let is_pending = engine
                .issues(&pair)?
                .iter()
                .any(|issue| &issue.path == path);
            if is_pending || engine.ignored(&pair)?.contains(path) {
                matching.push(pair);
            }
        }
        match matching.len() {
            0 => bail!("{path} has no conflict or divergence to resolve"),
            1 => Ok(matching.remove(0)),
            _ => bail!("several pairs have an issue on {path}; choose one with --pair"),
        }
    }

    fn edit_draft(&self, engine: &Engine<DiskFs>, pair: &Pair, path: &EntryPath) -> Result<String> {
        let draft = engine.conflict_draft(pair, path)?;
        let name = path.as_str().replace('/', "_");
        let file = self
            .paths
            .data_dir()
            .join("tmp")
            .join(format!("{}-{name}", pair.id));
        DiskFs.write_atomic(&file, draft.as_bytes())?;
        let edited = run_editor(&file).and_then(|()| {
            std::fs::read_to_string(&file)
                .with_context(|| format!("cannot read {}", file.display()))
        });
        // The draft is our own temporary file, not a user file.
        let _ = std::fs::remove_file(&file);
        let edited = edited?;
        if markoo_core::merge::has_markers(&edited) {
            bail!("conflict markers are still there; nothing was changed");
        }
        if edited.trim().is_empty() {
            bail!("the result is empty; nothing was changed");
        }
        Ok(edited)
    }

    pub fn pair(&self, command: PairCommand) -> Result<()> {
        match command {
            PairCommand::Add {
                name: None,
                project: None,
                kb: None,
                kb_type: None,
                repo: None,
                direction: None,
                target_changes: None,
            } => {
                let new = self.ask_new_pair()?;
                self.add_pair(new)
            }
            PairCommand::Add {
                name,
                project,
                kb,
                kb_type,
                repo,
                direction,
                target_changes,
            } => {
                let (Some(name), Some(kb), Some(repo)) = (name, kb, repo) else {
                    bail!(
                        "usage: markoo pair add \"<name>\" --project <project> --kb <folder> --repo <folder>"
                    );
                };
                self.add_pair(NewPair {
                    id: None,
                    name,
                    project,
                    kb,
                    kb_type,
                    repo,
                    direction,
                    target_changes,
                    settings: Vec::new(),
                })
            }
            PairCommand::List => self.pair_list(),
            PairCommand::Show { name } => self.pair_show(&name),
            PairCommand::Set {
                name,
                key,
                value,
                target_changes,
            } => self.pair_set(&name, &key, &value, target_changes.as_deref()),
            PairCommand::Remove { name } => self.pair_remove(&name),
        }
    }

    fn add_pair(&self, mut new: NewPair) -> Result<()> {
        new.kb = absolute(&new.kb)?;
        new.repo = absolute(&new.repo)?;
        let added = match manage::add_pair(&self.paths, &new) {
            Err(error @ markoo_core::Error::Config(ConfigError::MissingProject(_))) => {
                bail!("{error}: add --project <project>")
            }
            other => other?,
        };
        let id = added.id;
        let label = self.config()?.pair(&id)?.pair.label();
        self.warn_all(&label, &added.warnings);
        if self.json {
            return self.print_json(&json!({"added": id, "label": label}));
        }
        println!(
            "{} Pair \"{label}\" added {}",
            self.style.green("✓"),
            self.style.dim(&format!("(id: {id})"))
        );
        println!("Preview the first sync with: markoo status --pair {id}");
        Ok(())
    }

    fn warn_all(&self, label: &str, warnings: &[Problem]) {
        for problem in warnings {
            self.warn(&format!("pair \"{label}\": {}", problem.message));
        }
    }

    fn pair_list(&self) -> Result<()> {
        let config = self.config()?;
        if self.json {
            let pairs: Vec<Value> = config.pairs.iter().map(service::pair_view).collect();
            return self.print_json(&Value::Array(pairs));
        }
        if config.pairs.is_empty() {
            println!("No pairs yet. Add one with: markoo pair add");
        }
        // Pairs created before projects existed come last.
        let groups = config.projects().into_iter().map(Some).chain([None]);
        for project in groups {
            let members: Vec<&Pair> = config
                .pairs
                .iter()
                .map(|entry| &entry.pair)
                .filter(|pair| pair.project.as_deref() == project)
                .collect();
            if members.is_empty() {
                continue;
            }
            println!("{}", self.style.bold(project.unwrap_or("(no project)")));
            for pair in members {
                let title = format!(
                    "{}  {}",
                    self.style.bold(&pair.name),
                    self.style.dim(&format!("[{}]", pair.id))
                );
                print!("  ");
                self.pair_line(&title, pair);
            }
        }
        Ok(())
    }

    fn pair_show(&self, reference: &str) -> Result<()> {
        let config = self.config()?;
        let entry = config.pair(reference)?;
        if self.json {
            return self.print_json(&service::pair_view(entry));
        }
        let pair = &entry.pair;
        println!("{}", self.style.bold(&pair.label()));
        println!("  id              {}", pair.id);
        println!("  name            {}", pair.name);
        println!(
            "  project         {}",
            pair.project.as_deref().unwrap_or("(none)")
        );
        println!("  kb              {}", pair.kb.display());
        println!("  kb_type         {}", pair.kb_type.id());
        println!("  repo            {}", pair.repo.display());
        println!("  direction       {}", direction_value(pair.direction));
        println!(
            "  target_changes  {}",
            target_changes_value(pair.target_changes)
        );
        println!("  include         {}", pair.include.join(", "));
        println!("  exclude         {}", pair.exclude.join(", "));
        println!("  paused          {}", pair.paused);
        let validation = config.validate();
        for problem in validation.errors.iter().filter(|p| p.pair == pair.id) {
            self.report_error(&problem.message);
        }
        for problem in validation.warnings.iter().filter(|p| p.pair == pair.id) {
            self.warn(&problem.message);
        }
        Ok(())
    }

    fn pair_set(
        &self,
        reference: &str,
        key: &str,
        value: &str,
        target_changes: Option<&str>,
    ) -> Result<()> {
        let key_name = key.replace('-', "_");
        let mut changes = vec![(key_name.clone(), value.to_owned())];
        if let Some(target_changes) = target_changes {
            changes.push(("target_changes".to_owned(), target_changes.to_owned()));
        }
        // Resolved first: once renamed, the pair may no longer match `reference`.
        let id = self.config()?.pair(reference)?.pair.id.clone();
        let warnings = match manage::set_pair(&self.paths, &id, &changes) {
            Err(markoo_core::Error::DirectionBlocked { pending, .. }) => bail!(
                "{pending} conflict(s) must be resolved before switching to a one-way \
                 direction; see markoo conflicts"
            ),
            other => other?,
        };
        let config = self.config()?;
        let entry = config.pair(&id)?;
        let label = entry.pair.label();
        self.warn_all(&label, &warnings);
        if self.json {
            return self.print_json(&service::pair_view(entry));
        }
        println!("{} Pair \"{label}\" updated.", self.style.green("✓"));
        if key_name == "direction" {
            println!("The next sync only shows its plan; nothing is written until the one after.");
        }
        Ok(())
    }

    fn pair_remove(&self, reference: &str) -> Result<()> {
        let pair = self.config()?.pair(reference)?.pair.clone();
        let hooks_left = hooks_left(&pair);
        manage::remove_pair(&self.paths, &pair.id)?;
        if self.json {
            return self.print_json(&json!({"removed": pair.id, "hooks_left": hooks_left}));
        }
        println!(
            "{} Pair \"{}\" removed. Its files were not touched.",
            self.style.green("✓"),
            pair.label()
        );
        for file in &hooks_left {
            self.warn(&format!(
                "{} still calls this pair: delete its lines, between the two \"markoo {}\" markers",
                file.display(),
                pair.id
            ));
        }
        Ok(())
    }

    pub fn daemon(&self, action: Option<DaemonCommand>) -> Result<()> {
        match action {
            None => markoo_daemon::run(self.paths.clone())?,
            Some(DaemonCommand::Install) => {
                let file = markoo_daemon::service::install(&self.paths)?;
                println!(
                    "{} Service installed: {}",
                    self.style.green("✓"),
                    file.display()
                );
            }
            Some(DaemonCommand::Uninstall) => {
                markoo_daemon::service::uninstall()?;
                println!("{} Service removed.", self.style.green("✓"));
            }
        }
        Ok(())
    }

    pub fn hook(&self, command: HookCommand) -> Result<()> {
        match command {
            HookCommand::Install { pair } => {
                let config = self.config()?;
                let pair = &config.pair(&pair)?.pair;
                let changes = self.hook_edit(pair, hooks::install)?;
                let in_place = changes
                    .iter()
                    .any(|change| !matches!(change.status, HookStatus::Manual { .. }));
                if in_place && !self.json {
                    println!(
                        "{}",
                        self.style.dim(&format!(
                            "Nothing was synced: hooks act the next time Git runs them. \
                             To sync now: markoo sync --pair {}",
                            pair.id
                        ))
                    );
                }
                Ok(())
            }
            HookCommand::Uninstall { pair } => {
                let config = self.config()?;
                self.hook_edit(&config.pair(&pair)?.pair, hooks::uninstall)
                    .map(drop)
            }
            HookCommand::Run {
                hook,
                pair,
                arguments,
            } => {
                // Git carries on whatever happens here, so the only trace of
                // a failure is the log.
                if let Err(error) = self.hook_run(&hook, &pair, &arguments) {
                    self.log_hook_error(&hook, &pair, &format!("{error:#}"));
                }
                Ok(())
            }
        }
    }

    fn hook_edit(
        &self,
        pair: &Pair,
        edit: impl Fn(
            &DiskFs,
            &GitRepo,
            &HookTarget<'_>,
            &hooks::HookSettings,
        ) -> Result<Vec<HookChange>, hooks::HookError>,
    ) -> Result<Vec<HookChange>> {
        let repo = GitRepo::open(&pair.repo)?;
        let program = std::env::current_exe().context("cannot find the markoo binary")?;
        let target = HookTarget {
            pair: &pair.id,
            program: &program,
            config_file: self.paths.config_file(),
            data_dir: self.paths.data_dir(),
        };
        let changes = edit(&DiskFs, &repo, &target, &pair.hooks)?;
        if self.json {
            self.print_json(&serde_json::to_value(&changes)?)?;
            return Ok(changes);
        }
        if changes.is_empty() {
            println!("No hook to change.");
        }
        for change in &changes {
            self.print_hook_change(change);
        }
        Ok(changes)
    }

    fn print_hook_change(&self, change: &HookChange) {
        let hook = change.hook.as_str();
        let done = match &change.status {
            HookStatus::Created => "created",
            HookStatus::Added => "added to the existing hook",
            HookStatus::Updated => "updated",
            HookStatus::Unchanged => "already in place",
            HookStatus::Removed => "removed",
            HookStatus::Manual { reason, snippet } => {
                self.warn(&format!("{hook}: {reason}:"));
                println!("{snippet}");
                return;
            }
        };
        println!("{} {hook}: {done}", self.style.green("✓"));
    }

    fn hook_run(&self, hook: &str, reference: &str, arguments: &[String]) -> Result<()> {
        let hook: HookName = hook.parse().map_err(anyhow::Error::msg)?;
        let config = self.config()?;
        let pair = &config.pair(reference)?.pair;
        if pair.paused {
            return Ok(());
        }
        let repo = GitRepo::open(&pair.repo)?;
        if !hooks::should_act(&DiskFs, &repo, hook, arguments, &pair.hooks)? {
            return Ok(());
        }
        let validation = config.validate();
        if !validation.is_ok(&pair.id) {
            let problems: Vec<_> = validation
                .errors
                .iter()
                .filter(|problem| problem.pair == pair.id)
                .map(|problem| problem.message.as_str())
                .collect();
            bail!("{}", problems.join("; "));
        }
        if hook == HookName::PreCommit {
            let conflicts = self
                .engine()?
                .issues(pair)?
                .iter()
                .filter(|issue| issue.kind == IssueKind::Conflict)
                .count();
            if conflicts > 0 {
                self.report_error(&format!(
                    "{conflicts} Markoo conflict(s) pending in \"{}\": \
                     see `markoo conflicts`, or skip this check with `git commit --no-verify`",
                    pair.label()
                ));
            }
            return Ok(());
        }
        let answer = if config.api.enabled {
            client::request_sync(self.paths.data_dir(), config.api.port, &pair.id)
        } else {
            SyncRequest::Unavailable
        };
        match answer {
            SyncRequest::Queued => {}
            SyncRequest::Refused(message) => bail!(message),
            SyncRequest::Unavailable => {
                self.engine()?.run_recorded(pair, false, "hook")?;
            }
        }
        Ok(())
    }

    fn log_hook_error(&self, hook: &str, pair: &str, message: &str) {
        use std::io::Write;
        let seconds = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());
        let file = self.paths.data_dir().join("hooks.log");
        let opened = std::fs::create_dir_all(self.paths.data_dir()).and_then(|()| {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&file)
        });
        // Nothing more can be done: a hook must not get in the way of Git.
        if let Ok(mut log) = opened {
            let _ = writeln!(log, "{seconds} {hook} {pair}: {message}");
        }
    }

    pub fn mcp(&self) -> Result<()> {
        markoo_mcp::run_stdio(self.paths.clone())?;
        Ok(())
    }

    pub fn token_rotate(&self) -> Result<()> {
        markoo_daemon::token::rotate(self.paths.data_dir())?;
        let file = markoo_daemon::token::path(self.paths.data_dir());
        if self.json {
            return self.print_json(&json!({"token_file": file}));
        }
        println!("{} New token in {}", self.style.green("✓"), file.display());
        Ok(())
    }

    pub fn paths(&self) -> Result<()> {
        let paths = &self.paths;
        if self.json {
            return self.print_json(&json!({
                "config": {"path": paths.config_file(), "source": paths.config_source().to_string()},
                "data": {"path": paths.data_dir(), "source": paths.data_source().to_string()},
            }));
        }
        println!(
            "config  {}  ({})",
            paths.config_file().display(),
            paths.config_source()
        );
        println!(
            "data    {}  ({})",
            paths.data_dir().display(),
            paths.data_source()
        );
        Ok(())
    }

    /// The label of a pair, then its folders.
    fn pair_header(&self, pair: &Pair) {
        self.pair_line(&self.style.bold(&pair.label()), pair);
    }

    /// `title`, then the folders of the pair.
    fn pair_line(&self, title: &str, pair: &Pair) {
        let arrow = match pair.direction {
            Direction::Both => "⇄",
            Direction::KbToRepo => "→",
            Direction::RepoToKb => "←",
        };
        let paused = if pair.paused { "  (paused)" } else { "" };
        println!(
            "{title}  {} {arrow} {}  {}{paused}",
            pair.kb.display(),
            pair.repo.display(),
            self.style.dim(&format!("[{}]", pair.kb_type.id())),
        );
    }

    fn print_attachments(&self, attachments: &[PlannedAttachment]) {
        for attachment in attachments {
            let name = &attachment.name;
            match &attachment.action {
                AttachmentAction::Copy { to: Side::Repo, .. } => {
                    println!("  {} {name}", self.style.green("→"));
                }
                AttachmentAction::Copy { to: Side::Kb, .. } => {
                    println!("  {} {name}", self.style.green("←"));
                }
                AttachmentAction::Conflict => println!(
                    "  {} {name}  {}",
                    self.style.red("!"),
                    self.style.red("attachment changed on both sides")
                ),
                AttachmentAction::Record { .. } => {}
            }
        }
    }

    fn print_entries(&self, entries: &[&PlannedEntry]) {
        if entries.is_empty() {
            println!("  {}", self.style.green("Up to date."));
        }
        for entry in entries {
            let path = &entry.path;
            let line = match &entry.action {
                Action::Copy { to: Side::Repo, .. } => {
                    format!("{} {path}", self.style.green("→"))
                }
                Action::Copy { to: Side::Kb, .. } => {
                    format!("{} {path}", self.style.green("←"))
                }
                Action::Merge { .. } => {
                    format!(
                        "{} {path}  {}",
                        self.style.cyan("⇄"),
                        self.style.dim("merge")
                    )
                }
                Action::Trash { side } => format!(
                    "{} {path}  {}",
                    self.style.yellow("✗"),
                    self.style.dim(&format!("to the trash in {side}"))
                ),
                Action::Rename { side, to, .. } => format!(
                    "{} {path} → {to}  {}",
                    self.style.cyan("↻"),
                    self.style.dim(&format!("rename in {side}"))
                ),
                Action::Pending { issue, .. } => format!(
                    "{} {path}  {}",
                    self.style.red("!"),
                    self.style.red(issue_label(*issue))
                ),
                Action::NotInSource { side } => format!(
                    "{} {path}  {}",
                    self.style.dim("?"),
                    self.style
                        .dim(&format!("only in {side}, not in the source"))
                ),
                Action::RecordBase { .. } | Action::ForgetBase => continue,
            };
            println!("  {line}");
        }
    }

    fn print_report(&self, report: &Report) {
        for name in &report.attachment_conflicts {
            self.warn(&format!(
                "attachment {name} changed on both sides; left alone"
            ));
        }
        for skipped in &report.skipped {
            self.report_error(&format!("{}: {}", skipped.path, skipped.reason));
        }
        if !report.pending.is_empty() {
            println!(
                "  {}",
                self.style.yellow(&format!(
                    "{} file(s) need your decision: markoo conflicts",
                    report.pending.len()
                ))
            );
        }
    }

    fn print_patch(&self, path: &EntryPath, patch: &str) {
        println!("{}", self.style.bold(&format!("--- repo/{path}")));
        println!("{}", self.style.bold(&format!("+++ kb/{path}")));
        for line in patch.lines().skip_while(|line| !line.starts_with("@@")) {
            let colored = match line.chars().next() {
                Some('+') => self.style.green(line),
                Some('-') => self.style.red(line),
                Some('@') => self.style.cyan(line),
                _ => line.to_owned(),
            };
            println!("{colored}");
        }
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

/// `3 min ago`, `2 h ago`, `5 d ago`.
fn age(millis: u64) -> String {
    let minutes = millis / 60_000;
    match minutes {
        0 => "just now".to_owned(),
        1..60 => format!("{minutes} min ago"),
        60..1440 => format!("{} h ago", minutes / 60),
        _ => format!("{} d ago", minutes / 1440),
    }
}

/// `2 to the repo · 1 to the trash`
fn report_summary(report: &Report) -> String {
    let to = |side: Side| {
        report
            .written
            .iter()
            .filter(|file| file.side == side)
            .count()
    };
    let mut parts = Vec::new();
    for (count, label) in [
        (to(Side::Repo), "to the repo"),
        (to(Side::Kb), "to the KB"),
        (report.trashed.len(), "to the trash"),
        (report.skipped.len(), "skipped"),
    ] {
        if count > 0 {
            parts.push(format!("{count} {label}"));
        }
    }
    parts.join(" · ")
}

/// Entries worth showing: base bookkeeping is invisible to the user.
fn visible_entries(entries: &[PlannedEntry]) -> Vec<&PlannedEntry> {
    entries.iter().filter(|entry| entry.is_visible()).collect()
}

fn entry_json(entry: &PlannedEntry) -> Value {
    json!(entry.view())
}

/// Hook files still calling a pair. Once it is gone, `hook uninstall` no
/// longer knows where they are. A repository out of reach has nothing to
/// report: this must not prevent removing the pair.
fn hooks_left(pair: &Pair) -> Vec<PathBuf> {
    GitRepo::open(&pair.repo)
        .and_then(|repo| hooks::files_with_block(&DiskFs, &repo, &pair.id))
        .unwrap_or_default()
}

fn direction_value(direction: Direction) -> &'static str {
    match direction {
        Direction::Both => "both",
        Direction::KbToRepo => "kb-to-repo",
        Direction::RepoToKb => "repo-to-kb",
    }
}

fn target_changes_value(target_changes: TargetChanges) -> &'static str {
    match target_changes {
        TargetChanges::Protect => "protect",
        TargetChanges::Overwrite => "overwrite",
    }
}

fn issue_label(kind: IssueKind) -> &'static str {
    match kind {
        IssueKind::Conflict => "conflict",
        IssueKind::DeleteModify => "deleted on one side, changed on the other",
        IssueKind::Divergence => "divergence",
    }
}

fn entry_path(file: &str) -> Result<EntryPath> {
    EntryPath::from_relative(Path::new(file))
        .with_context(|| format!("{file} must be relative to the pair's folders"))
}

/// Paths in the file must not depend on where the command was run from.
fn absolute(path: &str) -> Result<String> {
    if path.starts_with('~') || Path::new(path).is_absolute() {
        return Ok(path.to_owned());
    }
    let joined = std::env::current_dir()?.join(expand_home(path));
    let absolute = joined.canonicalize().unwrap_or(joined);
    Ok(absolute.display().to_string())
}

fn run_editor(file: &Path) -> Result<()> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| if cfg!(windows) { "notepad" } else { "vi" }.to_owned());
    let mut words = editor.split_whitespace();
    let program = words.next().context("$EDITOR is empty")?;
    let status = Process::new(program)
        .args(words)
        .arg(file)
        .status()
        .with_context(|| format!("cannot start {editor}"))?;
    if !status.success() {
        bail!("{editor} failed; nothing was changed");
    }
    Ok(())
}
