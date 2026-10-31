//! Git hooks of a pair: where they live and the block Markoo adds to them.
//!
//! A hook file may belong to the user or to a hook manager. Markoo only ever
//! touches its own block, between two markers naming the pair; anything it
//! cannot safely edit is left alone, with lines to add by hand instead.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::apply::run_id;
use crate::fs::{Fs, FsError};

/// Files recognizing a repository managed by `lefthook`.
const LEFTHOOK_FILES: &[&str] = &[
    "lefthook.yml",
    ".lefthook.yml",
    "lefthook.yaml",
    ".lefthook.yaml",
    "lefthook.toml",
    ".lefthook.toml",
    "lefthook.json",
    ".lefthook.json",
    ".config/lefthook.yml",
    ".config/lefthook.yaml",
];

/// Files present in the Git folder while an operation is under way.
const OPERATIONS_IN_PROGRESS: &[&str] = &[
    "rebase-merge",
    "rebase-apply",
    "MERGE_HEAD",
    "CHERRY_PICK_HEAD",
    "REVERT_HEAD",
    "BISECT_LOG",
];

/// Shells that can run the lines Markoo appends.
const SHELLS: &[&str] = &["sh", "bash", "dash", "zsh", "ksh", "ash"];

/// Errors raised while inspecting a repository or editing its hooks.
#[derive(Debug, thiserror::Error)]
pub enum HookError {
    /// `git` failed, or the folder is not in a repository.
    #[error("git failed in {}: {message}", folder.display())]
    Git {
        /// Folder it ran in.
        folder: PathBuf,
        /// What it said.
        message: String,
    },
    /// File access failed.
    #[error(transparent)]
    Fs(#[from] FsError),
}

/// A Git hook Markoo can use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HookName {
    /// After `git pull` or `git merge`.
    PostMerge,
    /// After switching branches.
    PostCheckout,
    /// After a local commit.
    PostCommit,
    /// Before a commit: refuses it while conflicts are pending.
    PreCommit,
}

impl HookName {
    /// Every hook.
    pub const ALL: [Self; 4] = [
        Self::PostMerge,
        Self::PostCheckout,
        Self::PostCommit,
        Self::PreCommit,
    ];

    /// File name of the hook.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PostMerge => "post-merge",
            Self::PostCheckout => "post-checkout",
            Self::PostCommit => "post-commit",
            Self::PreCommit => "pre-commit",
        }
    }
}

impl FromStr for HookName {
    type Err = String;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|hook| hook.as_str() == name)
            .ok_or_else(|| format!("unknown hook \"{name}\""))
    }
}

/// `[pair.hooks]`
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct HookSettings {
    /// Hooks `markoo hook install` sets up.
    pub install: Vec<HookName>,
    /// Branches the hooks act on; empty means all of them.
    pub branches: Vec<String>,
}

impl Default for HookSettings {
    fn default() -> Self {
        Self {
            install: vec![HookName::PostMerge],
            branches: Vec::new(),
        }
    }
}

/// What Git says about the repository holding a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitRepo {
    /// Root of the working tree.
    pub top_level: PathBuf,
    /// The Git folder of this working tree.
    pub git_dir: PathBuf,
    /// Where Git looks for hooks.
    pub hooks_dir: PathBuf,
    /// Value of `core.hooksPath`, if set.
    pub hooks_path: Option<String>,
    /// Current branch, `None` on a detached `HEAD`.
    pub branch: Option<String>,
}

impl GitRepo {
    /// Asks Git about the repository holding `folder`.
    pub fn open(folder: &Path) -> Result<Self, HookError> {
        let required = |args: &[&str]| {
            git(folder, args)?.ok_or_else(|| HookError::Git {
                folder: folder.to_path_buf(),
                message: "not a Git working tree".to_owned(),
            })
        };
        Ok(Self {
            top_level: PathBuf::from(required(&["rev-parse", "--show-toplevel"])?),
            git_dir: PathBuf::from(required(&["rev-parse", "--absolute-git-dir"])?),
            // Git answers relative to the folder it runs in.
            hooks_dir: folder.join(required(&["rev-parse", "--git-path", "hooks"])?),
            hooks_path: git(folder, &["config", "--get", "core.hooksPath"])?,
            branch: git(folder, &["symbolic-ref", "--short", "-q", "HEAD"])?,
        })
    }
}

/// Files under a folder that Git does not track (ignored ones included),
/// relative to it. `None` outside of a repository.
pub fn untracked_files(folder: &Path) -> Result<Option<Vec<String>>, HookError> {
    let listed = git(folder, &["ls-files", "--others", "-z"])?;
    Ok(listed.map(|text| {
        text.split('\0')
            .filter(|file| !file.is_empty())
            .map(str::to_owned)
            .collect()
    }))
}

/// Runs `git`, returning its output or `None` if it answered with a failure.
fn git(folder: &Path, args: &[&str]) -> Result<Option<String>, HookError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(folder)
        // Inside a hook, these would override the folder it should run in.
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .map_err(|error| HookError::Git {
            folder: folder.to_path_buf(),
            message: error.to_string(),
        })?;
    if !output.status.success() {
        return Ok(None);
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    Ok(Some(text))
}

/// Who owns the hooks when it is not Markoo's place to edit them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookManager {
    /// `husky`.
    Husky,
    /// `lefthook`.
    Lefthook,
    /// A folder set by `core.hooksPath`: it may be versioned, or shared by
    /// every repository of the user.
    HooksPath(String),
}

/// Finds a hook manager in a repository.
pub fn detect_manager<F: Fs>(fs: &F, repo: &GitRepo) -> Result<Option<HookManager>, HookError> {
    let hooks_path = repo.hooks_path.as_deref();
    if hooks_path.is_some_and(|path| path.contains(".husky"))
        || fs.has_folder(&repo.top_level.join(".husky"))
    {
        return Ok(Some(HookManager::Husky));
    }
    for file in LEFTHOOK_FILES {
        if fs.meta(&repo.top_level.join(file))?.is_some() {
            return Ok(Some(HookManager::Lefthook));
        }
    }
    Ok(hooks_path.map(|path| HookManager::HooksPath(path.to_owned())))
}

/// What happened to a hook.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "status")]
pub enum HookStatus {
    /// The file was created.
    Created,
    /// Markoo's block was added to an existing file.
    Added,
    /// Markoo's block changed.
    Updated,
    /// Already as it should be: nothing was written.
    Unchanged,
    /// Markoo's block was removed.
    Removed,
    /// Nothing was written: this has to be done by hand.
    Manual {
        /// Why.
        reason: String,
        /// Lines to add or remove.
        snippet: String,
    },
}

/// The outcome for one hook.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HookChange {
    /// The hook.
    pub hook: HookName,
    /// What happened.
    #[serde(flatten)]
    pub status: HookStatus,
}

/// What the lines of a hook need to know.
#[derive(Debug, Clone, Copy)]
pub struct HookTarget<'a> {
    /// The pair.
    pub pair: &'a str,
    /// The `markoo` binary.
    pub program: &'a Path,
    /// The configuration file in use.
    pub config_file: &'a Path,
    /// The data folder in use, also where backups go.
    pub data_dir: &'a Path,
}

/// Quotes a value for a POSIX shell.
fn quote(value: &Path) -> String {
    format!("'{}'", value.to_string_lossy().replace('\'', r"'\''"))
}

/// The line Markoo adds to a hook file. Except `pre-commit`, it runs in the
/// background: it can neither fail a Git command nor make it wait for a sync.
fn hook_line(hook: HookName, target: &HookTarget<'_>) -> String {
    let program = quote(target.program);
    let command = format!(
        "{program} --config {} --data-dir {} hook run {} --pair {}",
        quote(target.config_file),
        quote(target.data_dir),
        hook.as_str(),
        target.pair,
    );
    match hook {
        // A removed binary must not block every commit.
        HookName::PreCommit => format!("if [ -x {program} ]; then {command} || exit 1; fi"),
        _ => format!("{command} \"$@\" </dev/null >/dev/null 2>&1 &"),
    }
}

/// Lines for a versioned or shared hook: they cannot name this machine's
/// files, and must do nothing for someone who has no `markoo`.
fn portable_line(hook: HookName, pair: &str, arguments: &str) -> String {
    let name = hook.as_str();
    let command = format!("markoo hook run {name} --pair {pair}");
    match hook {
        HookName::PreCommit => {
            format!("! command -v markoo >/dev/null 2>&1 || {command}")
        }
        _ => format!(
            "command -v markoo >/dev/null 2>&1 && {command} {arguments} </dev/null >/dev/null 2>&1 &"
        ),
    }
}

fn manager_status(manager: &HookManager, hook: HookName, pair: &str) -> HookStatus {
    let name = hook.as_str();
    let (owner, snippet) = match manager {
        HookManager::Husky => (
            format!("husky manages the hooks of this repository, in .husky/{name}"),
            portable_line(hook, pair, "\"$@\""),
        ),
        HookManager::Lefthook => (
            "lefthook manages the hooks of this repository, in its configuration".to_owned(),
            format!(
                "{name}:\n  commands:\n    markoo-{pair}:\n      run: {}",
                portable_line(hook, pair, "{0}")
            ),
        ),
        HookManager::HooksPath(path) => (
            format!("core.hooksPath points to {path}, in {name}"),
            portable_line(hook, pair, "\"$@\""),
        ),
    };
    HookStatus::Manual {
        reason: format!("{owner}: `markoo` must be in the PATH"),
        snippet,
    }
}

/// Markers of the block of a pair.
fn markers(pair: &str) -> (String, String) {
    (
        format!("# >>> markoo {pair}"),
        format!("# <<< markoo {pair}"),
    )
}

fn block(pair: &str, line: &str) -> String {
    let (start, end) = markers(pair);
    format!("{start}\n{line}\n{end}\n")
}

/// Byte range of the block of a pair, `Err` if only one marker is there.
fn find_block(text: &str, pair: &str) -> Result<Option<std::ops::Range<usize>>, String> {
    let (start_marker, end_marker) = markers(pair);
    let mut start = None;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let content = line.trim_end();
        if content == start_marker {
            start = Some(offset);
        }
        offset += line.len();
        if content == end_marker {
            return match start {
                Some(start) => Ok(Some(start..offset)),
                None => Err(format!("\"{end_marker}\" has no opening marker")),
            };
        }
    }
    match start {
        Some(_) => Err(format!("\"{start_marker}\" has no closing marker")),
        None => Ok(None),
    }
}

/// Whether the lines Markoo appends can run after this file: `Err` says why not.
fn can_append(text: &str) -> Result<(), String> {
    if let Some(shebang) = text.lines().next().and_then(|l| l.strip_prefix("#!")) {
        let mut words = shebang.split_whitespace();
        let mut interpreter = words.next().unwrap_or_default();
        if interpreter.ends_with("/env") {
            interpreter = words.next().unwrap_or_default();
        }
        let name = interpreter.rsplit('/').next().unwrap_or_default();
        if !SHELLS.contains(&name) {
            return Err(format!("it is not a shell script (\"{name}\")"));
        }
    }
    let last = text
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty() && !line.starts_with('#'));
    let stops = last.is_some_and(|line| {
        line == "exit" || line.starts_with("exit ") || line.starts_with("exec ")
    });
    if stops {
        return Err("it ends before anything appended to it could run".to_owned());
    }
    Ok(())
}

/// The content of a hook once it has the block of a pair. `Ok(None)` means
/// nothing changes; `Err` says why Markoo must not edit it.
fn with_block(existing: Option<&str>, pair: &str, line: &str) -> Result<Option<String>, String> {
    let wanted = block(pair, line);
    let Some(text) = existing else {
        return Ok(Some(format!("#!/bin/sh\n{wanted}")));
    };
    if let Some(range) = find_block(text, pair)? {
        if text[range.clone()].replace("\r\n", "\n") == wanted {
            return Ok(None);
        }
        return Ok(Some(format!(
            "{}{wanted}{}",
            &text[..range.start],
            &text[range.end..]
        )));
    }
    can_append(text)?;
    let separator = if text.is_empty() || text.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    Ok(Some(format!("{text}{separator}{wanted}")))
}

/// The content of a hook once the block of a pair is gone, `Ok(None)` if it
/// has no such block.
fn without_block(text: &str, pair: &str) -> Result<Option<String>, String> {
    Ok(find_block(text, pair)?
        .map(|range| format!("{}{}", &text[..range.start], &text[range.end..])))
}

/// Whether nothing would run in this hook file.
fn is_empty_hook(text: &str) -> bool {
    text.lines()
        .enumerate()
        .all(|(index, line)| line.trim().is_empty() || (index == 0 && line.starts_with("#!")))
}

/// Sets up the hooks a pair asks for and removes its other ones.
pub fn install<F: Fs>(
    fs: &F,
    repo: &GitRepo,
    target: &HookTarget<'_>,
    settings: &HookSettings,
) -> Result<Vec<HookChange>, HookError> {
    converge(fs, repo, target, &settings.install)
}

/// Removes the hooks of a pair.
pub fn uninstall<F: Fs>(
    fs: &F,
    repo: &GitRepo,
    target: &HookTarget<'_>,
    settings: &HookSettings,
) -> Result<Vec<HookChange>, HookError> {
    if let Some(manager) = detect_manager(fs, repo)? {
        return Ok(settings
            .install
            .iter()
            .map(|&hook| {
                let status = match manager_status(&manager, hook, target.pair) {
                    HookStatus::Manual { reason, snippet } => HookStatus::Manual {
                        reason: format!("{reason}. If you added these lines, remove them"),
                        snippet,
                    },
                    other => other,
                };
                HookChange { hook, status }
            })
            .collect());
    }
    converge(fs, repo, target, &[])
}

fn converge<F: Fs>(
    fs: &F,
    repo: &GitRepo,
    target: &HookTarget<'_>,
    wanted: &[HookName],
) -> Result<Vec<HookChange>, HookError> {
    if let Some(manager) = detect_manager(fs, repo)? {
        return Ok(wanted
            .iter()
            .map(|&hook| HookChange {
                hook,
                status: manager_status(&manager, hook, target.pair),
            })
            .collect());
    }
    let run = run_id();
    let mut changes = Vec::new();
    for hook in HookName::ALL {
        let path = repo.hooks_dir.join(hook.as_str());
        let status = if wanted.contains(&hook) {
            Some(add_to_file(fs, &path, hook, target, &run)?)
        } else {
            remove_from_file(fs, &path, hook, target, &run)?
        };
        if let Some(status) = status {
            changes.push(HookChange { hook, status });
        }
    }
    Ok(changes)
}

/// `<data>/<kind>/<pair>/<run>/hooks/<hook>`
fn keep_path(target: &HookTarget<'_>, kind: &str, run: &str, hook: HookName) -> PathBuf {
    target
        .data_dir
        .join(kind)
        .join(target.pair)
        .join(run)
        .join("hooks")
        .join(hook.as_str())
}

/// Reads a hook file: `Ok(None)` if it does not exist.
fn read_hook<F: Fs>(fs: &F, path: &Path) -> Result<Option<Vec<u8>>, HookError> {
    match fs.read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(FsError::NotFound(_)) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn add_to_file<F: Fs>(
    fs: &F,
    path: &Path,
    hook: HookName,
    target: &HookTarget<'_>,
    run: &str,
) -> Result<HookStatus, HookError> {
    let line = hook_line(hook, target);
    let manual = |reason: String| HookStatus::Manual {
        reason: format!("{} was left as it is: {reason}", path.display()),
        snippet: line.clone(),
    };
    // Writing through a link would replace it with a file.
    if fs.is_symlink(path) {
        return Ok(manual("it is a link".to_owned()));
    }
    let Some(bytes) = read_hook(fs, path)? else {
        let Ok(Some(content)) = with_block(None, target.pair, &line) else {
            return Ok(HookStatus::Unchanged);
        };
        fs.write_atomic(path, content.as_bytes())?;
        fs.make_executable(path)?;
        return Ok(HookStatus::Created);
    };
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return Ok(manual("it is not a text file".to_owned()));
    };
    let had_block = find_block(text, target.pair).is_ok_and(|range| range.is_some());
    // Making it executable would turn on a hook the user switched off.
    if !had_block && !fs.is_executable(path) {
        return Ok(manual("it is not executable, so Git ignores it".to_owned()));
    }
    match with_block(Some(text), target.pair, &line) {
        Err(reason) => Ok(manual(reason)),
        Ok(None) => Ok(HookStatus::Unchanged),
        Ok(Some(content)) => {
            fs.write_atomic(&keep_path(target, "backups", run, hook), &bytes)?;
            fs.write_atomic(path, content.as_bytes())?;
            fs.make_executable(path)?;
            Ok(if had_block {
                HookStatus::Updated
            } else {
                HookStatus::Added
            })
        }
    }
}

fn remove_from_file<F: Fs>(
    fs: &F,
    path: &Path,
    hook: HookName,
    target: &HookTarget<'_>,
    run: &str,
) -> Result<Option<HookStatus>, HookError> {
    if fs.is_symlink(path) {
        return Ok(None);
    }
    let Some(bytes) = read_hook(fs, path)? else {
        return Ok(None);
    };
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return Ok(None);
    };
    let content = match without_block(text, target.pair) {
        Ok(Some(content)) => content,
        Ok(None) => return Ok(None),
        Err(reason) => {
            return Ok(Some(HookStatus::Manual {
                reason: format!("{} was left as it is: {reason}", path.display()),
                snippet: markers(target.pair).0,
            }));
        }
    };
    if is_empty_hook(&content) {
        fs.rename(path, &keep_path(target, "trash", run, hook))?;
    } else {
        fs.write_atomic(&keep_path(target, "backups", run, hook), &bytes)?;
        fs.write_atomic(path, content.as_bytes())?;
        fs.make_executable(path)?;
    }
    Ok(Some(HookStatus::Removed))
}

/// Hook files holding a block of a pair, whatever the settings say now.
pub fn files_with_block<F: Fs>(
    fs: &F,
    repo: &GitRepo,
    pair: &str,
) -> Result<Vec<PathBuf>, HookError> {
    let mut files = Vec::new();
    for hook in HookName::ALL {
        let path = repo.hooks_dir.join(hook.as_str());
        let Some(bytes) = read_hook(fs, &path)? else {
            continue;
        };
        let (start, _) = markers(pair);
        if String::from_utf8_lossy(&bytes)
            .lines()
            .any(|line| line.trim_end() == start)
        {
            files.push(path);
        }
    }
    Ok(files)
}

/// Whether a hook that just fired should act, given its arguments.
pub fn should_act<F: Fs>(
    fs: &F,
    repo: &GitRepo,
    hook: HookName,
    arguments: &[String],
    settings: &HookSettings,
) -> Result<bool, HookError> {
    if hook == HookName::PostCheckout {
        // Arguments: previous HEAD, new HEAD, 1 when switching branches. Same
        // commit or a single file restored: the branch's content did not move.
        let [previous, new, flag, ..] = arguments else {
            return Ok(false);
        };
        if flag != "1" || previous == new {
            return Ok(false);
        }
    }
    // A rebase checks out and commits half-finished states: syncing them would
    // bring them into the knowledge base.
    for file in OPERATIONS_IN_PROGRESS {
        let path = repo.git_dir.join(file);
        if fs.has_folder(&path) || fs.meta(&path)?.is_some() {
            return Ok(false);
        }
    }
    Ok(settings.branches.is_empty()
        || repo
            .branch
            .as_ref()
            .is_some_and(|branch| settings.branches.contains(branch)))
}

#[cfg(test)]
#[path = "hooks/tests.rs"]
mod tests;
