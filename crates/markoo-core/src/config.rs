//! Reading, validating and editing `markoo.toml`.
//!
//! The file stays the source of truth: edits go through `toml_edit`, so the
//! user's comments and layout survive, and an edit that would make the file
//! invalid is refused before anything reaches the disk.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, value};

use crate::entry::Side;
use crate::fs::{DiskFs, Fs, FsError};
use crate::hooks::HookSettings;
use crate::kb::KbType;
use crate::pair::{Direction, Pair, TargetChanges};

/// Content of a newly created configuration file.
pub const TEMPLATE: &str = "\
# Markoo configuration: https://github.com/studoo-app/markoo
# Add a pair with `markoo pair add`, or by hand:
#
# [[pair]]
# name = \"notes\"
# project = \"My project\"      # pairs of a project are shown together
# kb   = \"~/Notes/Project\"
# repo = \"~/code/project/docs\"
# direction = \"both\"          # both | kb-to-repo | repo-to-kb
";

/// Errors raised while reading or editing the configuration.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The file does not exist.
    #[error("no configuration at {}; run `markoo init`", .0.display())]
    Missing(PathBuf),
    /// The file already exists.
    #[error("{} already exists", .0.display())]
    AlreadyExists(PathBuf),
    /// The file could not be read or written.
    #[error(transparent)]
    Fs(#[from] FsError),
    /// The content is not a valid configuration.
    #[error("invalid configuration: {0}")]
    Invalid(String),
    /// No pair has this name.
    #[error("no pair named \"{0}\"")]
    UnknownPair(String),
    /// A pair already has this identifier.
    #[error("a pair with the id \"{0}\" already exists")]
    DuplicatePair(String),
    /// Another pair of the project already has this name.
    #[error("{} already has a pair named \"{name}\"", project_label(.project.as_deref()))]
    DuplicateName {
        /// The project, `None` for pairs without one.
        project: Option<String>,
        /// The name.
        name: String,
    },
    /// The name matches pairs of several projects.
    #[error("\"{reference}\" matches several pairs: {}; use project/name or the id", .candidates.join(", "))]
    AmbiguousPair {
        /// What was asked for.
        reference: String,
        /// `project/name` of each match.
        candidates: Vec<String>,
    },
    /// The identifier of a pair is fixed once written.
    #[error("the id of a pair cannot be changed")]
    ImmutableId,
    /// The pair id cannot be used in commands and URLs.
    #[error("invalid pair id \"{0}\": use lowercase letters, digits and hyphens")]
    InvalidName(String),
    /// The name of a pair is empty, too long or spans several lines.
    #[error(
        "invalid pair name \"{0}\": use 1 to {MAX_PROJECT_LEN} characters on one line, without surrounding spaces or /"
    )]
    InvalidLabel(String),
    /// The project name is empty, too long or spans several lines.
    #[error(
        "invalid project name \"{0}\": use 1 to {MAX_PROJECT_LEN} characters on one line, without surrounding spaces or /"
    )]
    InvalidProject(String),
    /// A new pair must say which project it belongs to.
    #[error("pair \"{0}\": name its project")]
    MissingProject(String),
    /// This KB type has no adapter yet.
    #[error("pair \"{pair}\": kb_type \"{kb_type}\" is not available yet; use \"markdown\"")]
    UnsupportedKbType {
        /// The pair.
        pair: String,
        /// The requested type.
        kb_type: String,
    },
    /// The pair does not pass validation.
    #[error("pair \"{pair}\": {message}")]
    InvalidPair {
        /// The pair.
        pair: String,
        /// What is wrong with it.
        message: String,
    },
    /// This key cannot be set.
    #[error("unknown pair setting \"{0}\"")]
    UnknownKey(String),
}

/// One `[[pair]]` table as written in the file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPair {
    id: Option<String>,
    name: String,
    project: Option<String>,
    kb: String,
    kb_type: Option<String>,
    repo: String,
    include: Option<Vec<String>>,
    #[serde(default)]
    exclude: Vec<String>,
    conflict: Option<String>,
    #[serde(default)]
    direction: Direction,
    #[serde(default)]
    target_changes: TargetChanges,
    #[serde(default)]
    paused: bool,
    transform: Option<RawTransform>,
    kb_options: Option<toml::Table>,
    hooks: Option<HookSettings>,
}

/// `[pair.transform]`
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTransform {
    links: Option<LinksMode>,
    filenames: Option<String>,
    assets_dir: Option<String>,
    frontmatter: Option<FrontmatterMode>,
    strip_comments: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum LinksMode {
    Markdown,
    Keep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum FrontmatterMode {
    Keep,
    Filter,
    Drop,
}

#[derive(Debug, Deserialize)]
struct RawConfig {
    #[serde(default)]
    pair: Vec<RawPair>,
    general: Option<RawGeneral>,
    schedule: Option<RawSchedule>,
    api: Option<RawApi>,
    mcp: Option<RawMcp>,
}

/// `[general]`
#[derive(Debug, Deserialize)]
struct RawGeneral {
    language: Option<String>,
}

/// `[mcp]`
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMcp {
    http: Option<bool>,
    allow_config_write: Option<bool>,
}

/// `[schedule]`
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSchedule {
    interval: Option<String>,
    on_startup: Option<bool>,
    pause_on_battery: Option<bool>,
}

/// `[api]`
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawApi {
    enabled: Option<bool>,
    port: Option<u16>,
}

/// When the daemon syncs on its own (`[schedule]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schedule {
    /// Time between two syncs of every pair, `None` for `"off"`.
    pub interval: Option<Duration>,
    /// Whether to sync as soon as the daemon starts.
    pub on_startup: bool,
}

impl Default for Schedule {
    fn default() -> Self {
        Self {
            interval: Some(Duration::from_secs(5 * 60)),
            on_startup: true,
        }
    }
}

/// The daemon's local HTTP API (`[api]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Api {
    /// Whether the API is served.
    pub enabled: bool,
    /// Port on `127.0.0.1`.
    pub port: u16,
}

impl Default for Api {
    fn default() -> Self {
        Self {
            enabled: true,
            port: 4848,
        }
    }
}

/// Parses `30s`, `5m`, `1h`, or `off` into `None`.
fn parse_interval(text: &str) -> Result<Option<Duration>, ConfigError> {
    if text == "off" {
        return Ok(None);
    }
    let invalid = || {
        ConfigError::Invalid(format!(
            "invalid interval \"{text}\": use a number followed by s, m or h, or \"off\""
        ))
    };
    let digits = text.len().saturating_sub(1);
    let (number, unit) = (text.get(..digits).ok_or_else(invalid)?, &text[digits..]);
    let number: u64 = number.parse().map_err(|_| invalid())?;
    let seconds = match unit {
        "s" => Some(number),
        "m" => number.checked_mul(60),
        "h" => number.checked_mul(3600),
        _ => None,
    }
    .filter(|seconds| *seconds > 0)
    .ok_or_else(invalid)?;
    Ok(Some(Duration::from_secs(seconds)))
}

/// A pair read from the configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairEntry {
    /// The pair, with `~` expanded.
    pub pair: Pair,
    /// Whether `kb_type` was written in the file (otherwise it is detected).
    pub kb_type_explicit: bool,
    /// Options accepted in the file but not honored yet.
    pub not_implemented: Vec<String>,
    /// Keys of `[pair.kb_options]` the adapter does not know.
    pub unknown_options: Vec<String>,
}

/// Language of the configuration UI (`[general] language`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Language {
    /// English, the default.
    #[default]
    En,
    /// French.
    Fr,
}

impl Language {
    /// The value written in the file.
    pub fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Fr => "fr",
        }
    }

    /// Reads a value of the file; `None` if the language is not supported.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "en" => Some(Self::En),
            "fr" => Some(Self::Fr),
            _ => None,
        }
    }
}

/// A parsed configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    /// Pairs, in file order.
    pub pairs: Vec<PairEntry>,
    /// Language of the configuration UI.
    pub language: Language,
    /// When the daemon syncs on its own.
    pub schedule: Schedule,
    /// The daemon's local HTTP API.
    pub api: Api,
    /// The MCP server.
    pub mcp: Mcp,
    /// General options accepted in the file but not honored yet.
    pub not_implemented: Vec<String>,
}

/// The MCP server (`[mcp]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mcp {
    /// Whether the daemon also serves `/mcp` on its API.
    pub http: bool,
    /// Whether an agent may change the configuration.
    pub allow_config_write: bool,
}

impl Default for Mcp {
    fn default() -> Self {
        Self {
            http: true,
            allow_config_write: false,
        }
    }
}

/// A problem found while validating a pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// The pair concerned.
    pub pair: String,
    /// Description.
    pub message: String,
}

/// Result of [`Config::validate`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Validation {
    /// Problems that prevent a pair from syncing.
    pub errors: Vec<Problem>,
    /// Problems worth telling the user about.
    pub warnings: Vec<Problem>,
}

impl Validation {
    /// Whether this pair can sync.
    pub fn is_ok(&self, pair: &str) -> bool {
        !self.errors.iter().any(|problem| problem.pair == pair)
    }
}

impl Config {
    /// Reads the configuration file.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let bytes = read_existing(path)?;
        let text = String::from_utf8(bytes)
            .map_err(|_| ConfigError::Invalid("the file is not valid UTF-8".to_owned()))?;
        Self::parse(&text)
    }

    /// Parses the content of a configuration file.
    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        let raw: RawConfig =
            toml::from_str(text).map_err(|e| ConfigError::Invalid(e.to_string()))?;
        let mut ids = BTreeSet::new();
        let mut names = BTreeSet::new();
        let mut pairs = Vec::with_capacity(raw.pair.len());
        for raw in raw.pair {
            // Pairs written before identifiers existed are known by their name.
            let id = raw.id.clone().unwrap_or_else(|| raw.name.clone());
            if !is_valid_name(&id) {
                return Err(ConfigError::InvalidName(id));
            }
            if !is_valid_label(&raw.name) {
                return Err(ConfigError::InvalidLabel(raw.name));
            }
            if !ids.insert(id.clone()) {
                return Err(ConfigError::DuplicatePair(id));
            }
            let project_key = raw.project.as_deref().map(str::to_lowercase);
            if !names.insert((project_key, raw.name.to_lowercase())) {
                return Err(ConfigError::DuplicateName {
                    project: raw.project,
                    name: raw.name,
                });
            }
            if let Some(project) = &raw.project
                && !is_valid_label(project)
            {
                return Err(ConfigError::InvalidProject(project.clone()));
            }
            pairs.push(PairEntry::from_raw(raw)?);
        }
        let mut language = Language::default();
        if let Some(text) = raw.general.and_then(|general| general.language) {
            language = Language::parse(&text)
                .ok_or_else(|| ConfigError::Invalid(format!("unsupported language: {text}")))?;
        }
        let mut schedule = Schedule::default();
        let mut not_implemented = Vec::new();
        if let Some(raw) = raw.schedule {
            if let Some(interval) = raw.interval {
                schedule.interval = parse_interval(&interval)?;
            }
            schedule.on_startup = raw.on_startup.unwrap_or(schedule.on_startup);
            if raw.pause_on_battery.is_some() {
                not_implemented.push("schedule.pause_on_battery".to_owned());
            }
        }
        let mut api = Api::default();
        if let Some(raw) = raw.api {
            api.enabled = raw.enabled.unwrap_or(api.enabled);
            api.port = raw.port.unwrap_or(api.port);
        }
        let mut mcp = Mcp::default();
        if let Some(raw) = raw.mcp {
            mcp.http = raw.http.unwrap_or(mcp.http);
            mcp.allow_config_write = raw.allow_config_write.unwrap_or(mcp.allow_config_write);
        }
        Ok(Self {
            pairs,
            language,
            schedule,
            api,
            mcp,
            not_implemented,
        })
    }

    /// Finds a pair from what a user typed, in this order: its id,
    /// `project/name`, or its name alone when no other project has a pair of
    /// that name. Projects and names are compared without regard to case.
    pub fn pair(&self, reference: &str) -> Result<&PairEntry, ConfigError> {
        if let Some(entry) = self.pairs.iter().find(|entry| entry.pair.id == reference) {
            return Ok(entry);
        }
        if let Some((project, name)) = reference.rsplit_once('/') {
            let project = project.trim().to_lowercase();
            return self
                .pairs
                .iter()
                .find(|entry| {
                    entry.pair.name.to_lowercase() == name.trim().to_lowercase()
                        && entry.pair.project.as_deref().map(str::to_lowercase)
                            == Some(project.clone())
                })
                .ok_or_else(|| ConfigError::UnknownPair(reference.to_owned()));
        }
        let matches: Vec<&PairEntry> = self
            .pairs
            .iter()
            .filter(|entry| entry.pair.name.to_lowercase() == reference.trim().to_lowercase())
            .collect();
        match matches.as_slice() {
            [] => Err(ConfigError::UnknownPair(reference.to_owned())),
            [entry] => Ok(entry),
            _ => Err(ConfigError::AmbiguousPair {
                reference: reference.to_owned(),
                candidates: matches.iter().map(|entry| entry.pair.label()).collect(),
            }),
        }
    }

    /// An identifier for a new pair that no pair uses yet: the project and
    /// the name in lowercase ASCII (`markoo-documentation`), with a number
    /// added if needed. It is written to the file and never recomputed.
    pub fn new_id(&self, project: Option<&str>, name: &str) -> String {
        let prefix = project.map(slug).filter(|prefix| !prefix.is_empty());
        let name = Some(slug(name))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "pair".to_owned());
        let base = match prefix {
            Some(prefix) if prefix != name => format!("{prefix}-{name}"),
            _ => name,
        };
        let taken = |id: &str| self.pairs.iter().any(|entry| entry.pair.id == id);
        if !taken(&base) {
            return base;
        }
        (2..)
            .map(|number| format!("{base}-{number}"))
            .find(|id| !taken(id))
            .unwrap_or(base)
    }

    /// Names of the projects, in the order they first appear in the file.
    pub fn projects(&self) -> Vec<&str> {
        let mut projects: Vec<&str> = Vec::new();
        for entry in &self.pairs {
            if let Some(project) = entry.pair.project.as_deref()
                && !projects.contains(&project)
            {
                projects.push(project);
            }
        }
        projects
    }

    /// The spelling already used for this project, if one matches it without
    /// regard to case or surrounding spaces: "markoo" joins an existing
    /// "Markoo" instead of starting a second project.
    pub fn known_project(&self, name: &str) -> Option<&str> {
        let name = name.trim().to_lowercase();
        self.projects()
            .into_iter()
            .find(|project| project.to_lowercase() == name)
    }

    /// Checks every pair against the disk.
    pub fn validate(&self) -> Validation {
        let mut validation = Validation::default();
        for entry in &self.pairs {
            entry.validate(&mut validation);
        }
        for (index, first) in self.pairs.iter().enumerate() {
            for second in &self.pairs[index + 1..] {
                for side in [Side::Kb, Side::Repo] {
                    if let Some(nesting) = nesting(second.pair.root(side), first.pair.root(side)) {
                        validation.errors.push(Problem {
                            pair: second.pair.id.clone(),
                            message: overlap_message(side, nesting, &first.pair),
                        });
                    }
                }
            }
        }
        validation
    }

    /// The first pair whose `side` folder shares files with `folder`, and
    /// where `folder` sits relative to it.
    pub fn overlapping_pair(&self, side: Side, folder: &Path) -> Option<(&PairEntry, Nesting)> {
        self.pairs
            .iter()
            .find_map(|entry| nesting(folder, entry.pair.root(side)).map(|found| (entry, found)))
    }
}

impl PairEntry {
    fn from_raw(raw: RawPair) -> Result<Self, ConfigError> {
        let kb = expand_home(&raw.kb);
        let kb_type = match raw.kb_type.as_deref() {
            None => KbType::detect(&kb),
            Some("markdown") => KbType::Markdown,
            Some("obsidian") => KbType::Obsidian,
            Some(other) => {
                return Err(ConfigError::UnsupportedKbType {
                    pair: raw.id.unwrap_or(raw.name),
                    kb_type: other.to_owned(),
                });
            }
        };
        let mut not_implemented = Vec::new();
        if raw.conflict.as_deref().is_some_and(|c| c != "ask") {
            not_implemented.push("conflict (conflicts are always put on hold)".to_owned());
        }
        let mut pair = Pair::new(raw.name, kb, expand_home(&raw.repo));
        if let Some(id) = raw.id {
            pair.id = id;
        }
        pair.project = raw.project;
        pair.hooks = raw.hooks.unwrap_or_default();
        if let Some(transform) = raw.transform {
            if transform
                .filenames
                .as_deref()
                .is_some_and(|f| f != "preserve")
            {
                not_implemented.push("transform.filenames".to_owned());
            }
            if transform.strip_comments == Some(true) {
                not_implemented.push("transform.strip_comments".to_owned());
            }
            if let Some(links) = transform.links {
                pair.kb_options.convert_links = links == LinksMode::Markdown;
            }
            if let Some(assets_dir) = transform.assets_dir {
                pair.assets_dir = assets_dir;
            }
            match transform.frontmatter {
                Some(FrontmatterMode::Keep) => pair.kb_options.frontmatter_drop.clear(),
                Some(FrontmatterMode::Drop) => pair.kb_options.frontmatter_drop_all = true,
                Some(FrontmatterMode::Filter) | None => {}
            }
        }
        let mut unknown_options = Vec::new();
        for (key, value) in raw.kb_options.iter().flatten() {
            match (key.as_str(), value) {
                ("callouts", toml::Value::String(mode)) => {
                    pair.kb_options.convert_callouts = mode != "keep";
                }
                ("inline_embeds", toml::Value::Boolean(inline)) => {
                    if *inline {
                        not_implemented.push("kb_options.inline_embeds".to_owned());
                    }
                }
                ("frontmatter_drop", toml::Value::Array(keys)) => {
                    pair.kb_options.frontmatter_drop = keys
                        .iter()
                        .filter_map(|key| key.as_str().map(str::to_owned))
                        .collect();
                }
                _ => unknown_options.push(key.clone()),
            }
        }
        pair.kb_type = kb_type;
        pair.direction = raw.direction;
        pair.target_changes = raw.target_changes;
        pair.exclude = raw.exclude;
        pair.paused = raw.paused;
        if let Some(include) = raw.include {
            pair.include = include;
        }
        Ok(Self {
            pair,
            kb_type_explicit: raw.kb_type.is_some(),
            not_implemented,
            unknown_options,
        })
    }

    fn validate(&self, validation: &mut Validation) {
        let name = &self.pair.id;
        let mut error = |message: String| {
            validation.errors.push(Problem {
                pair: name.clone(),
                message,
            });
        };
        for (label, folder) in [("kb", &self.pair.kb), ("repo", &self.pair.repo)] {
            if !folder.is_dir() {
                error(format!("{label} folder not found: {}", folder.display()));
            }
        }
        if nested(&self.pair.kb, &self.pair.repo) {
            error("the kb and repo folders overlap".to_owned());
        }
        if self.pair.kb_type == KbType::Obsidian
            && self.pair.kb.is_dir()
            && KbType::detect(&self.pair.kb) != KbType::Obsidian
        {
            error(
                "kb_type is \"obsidian\" but this folder is not in a vault (no .obsidian folder)"
                    .to_owned(),
            );
        }
        if crate::entry::EntryPath::from_relative(Path::new(&self.pair.assets_dir)).is_none() {
            error(format!(
                "assets_dir must stay inside the repo folder: {}",
                self.pair.assets_dir
            ));
        }

        let mut warning = |message: String| {
            validation.warnings.push(Problem {
                pair: name.clone(),
                message,
            });
        };
        if self.pair.target_changes == TargetChanges::Overwrite
            && self.pair.direction == Direction::Both
        {
            warning(
                "target_changes = \"overwrite\" has no effect with direction = \"both\"".into(),
            );
        }
        if self.pair.repo.is_dir() && !in_git_repository(&self.pair.repo) {
            warning("the repo folder is not inside a Git repository".to_owned());
        }
        for option in &self.unknown_options {
            warning(format!("kb_options.{option}: unknown option, ignored"));
        }
        for option in &self.not_implemented {
            warning(format!("{option}: not implemented yet, ignored"));
        }
    }
}

/// Whether a pair id can be used in commands and URLs.
pub fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Lowercase ASCII letters, digits and single hyphens: "Site Web — v2"
/// gives `site-web-v2`; letters with accents lose them when they are common.
pub fn slug(text: &str) -> String {
    let mut slug = String::new();
    for c in text.chars().flat_map(char::to_lowercase) {
        let c = match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
            'ç' => 'c',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'ñ' => 'n',
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' => 'o',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            'ý' | 'ÿ' => 'y',
            c => c,
        };
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_end_matches('-').to_owned()
}

fn project_label(project: Option<&str>) -> String {
    project.map_or_else(
        || "the pairs without a project".to_owned(),
        |project| format!("project \"{project}\""),
    )
}

/// Longest project name, in characters.
pub const MAX_PROJECT_LEN: usize = 64;

/// Whether a project or pair name can be written to the configuration. It
/// is free text ("Markoo", "Documentation technique"), but on one line and
/// without surrounding spaces, so that two spellings cannot differ by a blank.
pub fn is_valid_label(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= MAX_PROJECT_LEN
        && name.trim() == name
        && !name.chars().any(char::is_control)
        && !name.contains('/')
}

/// Settings of a new pair.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewPair {
    /// Identifier of the pair; [`crate::manage::add_pair`] fills it in.
    pub id: Option<String>,
    /// Name of the pair, unique within its project.
    pub name: String,
    /// Project of the pair: an existing one, or a new one named here.
    /// [`crate::manage::add_pair`] refuses a pair without one.
    pub project: Option<String>,
    /// KB folder, as written in the file.
    pub kb: String,
    /// KB type; detected if `None`.
    pub kb_type: Option<String>,
    /// Repo folder, as written in the file.
    pub repo: String,
    /// `markoo.toml` value of the direction.
    pub direction: Option<String>,
    /// `markoo.toml` value of `target_changes`.
    pub target_changes: Option<String>,
    /// Other settings, as for [`ConfigFile::set`]: written with the pair, since
    /// a daemon syncs it as soon as it appears in the file.
    pub settings: Vec<(String, String)>,
}

/// The configuration file, open for editing.
#[derive(Debug)]
pub struct ConfigFile {
    path: PathBuf,
    doc: DocumentMut,
}

impl ConfigFile {
    /// Creates a new file from [`TEMPLATE`]; refuses to replace an existing one.
    pub fn create(path: &Path) -> Result<(), ConfigError> {
        if DiskFs.meta(path)?.is_some() {
            return Err(ConfigError::AlreadyExists(path.to_path_buf()));
        }
        DiskFs.write_atomic(path, TEMPLATE.as_bytes())?;
        Ok(())
    }

    /// Opens an existing file for editing.
    pub fn open(path: &Path) -> Result<Self, ConfigError> {
        let bytes = read_existing(path)?;
        let text = String::from_utf8(bytes)
            .map_err(|_| ConfigError::Invalid("the file is not valid UTF-8".to_owned()))?;
        Self::from_text(path, &text)
    }

    fn from_text(path: &Path, text: &str) -> Result<Self, ConfigError> {
        let doc = text
            .parse::<DocumentMut>()
            .map_err(|e| ConfigError::Invalid(e.to_string()))?;
        Ok(Self {
            path: path.to_path_buf(),
            doc,
        })
    }

    /// Adds a pair.
    pub fn add_pair(&mut self, new: &NewPair) -> Result<(), ConfigError> {
        let mut table = Table::new();
        if let Some(id) = &new.id {
            table["id"] = value(id);
        }
        table["name"] = value(&new.name);
        if let Some(project) = &new.project {
            table["project"] = value(project);
        }
        table["kb"] = value(&new.kb);
        if let Some(kb_type) = &new.kb_type {
            table["kb_type"] = value(kb_type);
        }
        table["repo"] = value(&new.repo);
        if let Some(direction) = &new.direction {
            table["direction"] = value(direction);
        }
        if let Some(target_changes) = &new.target_changes {
            table["target_changes"] = value(target_changes);
        }
        // Comments at the end of the file would otherwise stay below the new
        // table, away from what they describe.
        let trailing = self.doc.trailing().as_str().unwrap_or_default().to_owned();
        if !trailing.is_empty() {
            table.decor_mut().set_prefix(trailing);
            self.doc.set_trailing("");
        }
        let pairs = self
            .doc
            .entry("pair")
            .or_insert(Item::ArrayOfTables(ArrayOfTables::new()));
        pairs
            .as_array_of_tables_mut()
            .ok_or_else(|| ConfigError::Invalid("\"pair\" must be an array of tables".to_owned()))?
            .push(table);
        Ok(())
    }

    /// Changes a setting of the pair with this id. Lists are comma-separated.
    pub fn set(&mut self, id: &str, key: &str, new_value: &str) -> Result<(), ConfigError> {
        let key = key.replace('-', "_");
        let table = self.pair_table(id)?;
        if let Some((section, option)) = key.split_once('.') {
            let new_item = match (section, option) {
                ("transform", "links" | "assets_dir" | "frontmatter")
                | ("kb_options", "callouts") => value(new_value),
                ("kb_options", "frontmatter_drop") | ("hooks", "install" | "branches") => {
                    value(comma_separated(new_value))
                }
                _ => return Err(ConfigError::UnknownKey(key)),
            };
            let section = table
                .entry(section)
                .or_insert_with(toml_edit::table)
                .as_table_mut()
                .ok_or_else(|| ConfigError::Invalid(format!("{section} must be a table")))?;
            section[option] = new_item;
            return Ok(());
        }
        match key.as_str() {
            "direction" | "target_changes" | "kb" | "repo" | "kb_type" | "conflict" => {
                table[key.as_str()] = value(new_value);
            }
            "name" => {
                // A pair known by its name gets its id written down first,
                // so that renaming it keeps its state.
                if !table.contains_key("id") {
                    table.insert("id", value(id));
                    table.sort_values_by(|a, _, b, _| (a.get() != "id").cmp(&(b.get() != "id")));
                }
                let label = new_value.trim();
                if !is_valid_label(label) {
                    return Err(ConfigError::InvalidLabel(new_value.to_owned()));
                }
                table["name"] = value(label);
            }
            "id" => return Err(ConfigError::ImmutableId),
            "project" => {
                let project = new_value.trim();
                if !is_valid_label(project) {
                    return Err(ConfigError::InvalidProject(new_value.to_owned()));
                }
                table["project"] = value(project);
            }
            "paused" => {
                let paused = new_value.parse::<bool>().map_err(|_| {
                    ConfigError::Invalid(format!("paused must be true or false, not {new_value}"))
                })?;
                table["paused"] = value(paused);
            }
            "include" | "exclude" => {
                table[key.as_str()] = value(comma_separated(new_value));
            }
            _ => return Err(ConfigError::UnknownKey(key)),
        }
        Ok(())
    }

    /// Changes a setting that applies to every pair (`schedule.interval`,
    /// `schedule.on_startup`, `api.enabled`, `api.port`, `mcp.http`,
    /// `mcp.allow_config_write`).
    pub fn set_global(&mut self, key: &str, new_value: &str) -> Result<(), ConfigError> {
        let invalid = || ConfigError::Invalid(format!("invalid value for {key}: {new_value}"));
        let new_item = match key {
            "schedule.interval" => value(new_value),
            "schedule.on_startup" | "api.enabled" | "mcp.http" | "mcp.allow_config_write" => {
                value(new_value.parse::<bool>().map_err(|_| invalid())?)
            }
            "api.port" => value(i64::from(new_value.parse::<u16>().map_err(|_| invalid())?)),
            "general.language" => value(Language::parse(new_value).ok_or_else(invalid)?.code()),
            _ => return Err(ConfigError::UnknownKey(key.to_owned())),
        };
        let (section, option) = key.split_once('.').ok_or_else(invalid)?;
        let section = self
            .doc
            .entry(section)
            .or_insert_with(toml_edit::table)
            .as_table_mut()
            .ok_or_else(invalid)?;
        section[option] = new_item;
        Ok(())
    }

    /// Removes the pair with this id.
    pub fn remove_pair(&mut self, name: &str) -> Result<(), ConfigError> {
        let pairs = self
            .doc
            .get_mut("pair")
            .and_then(Item::as_array_of_tables_mut)
            .ok_or_else(|| ConfigError::UnknownPair(name.to_owned()))?;
        let index = pairs
            .iter()
            .position(|table| table_id(table) == Some(name))
            .ok_or_else(|| ConfigError::UnknownPair(name.to_owned()))?;
        pairs.remove(index);
        Ok(())
    }

    /// The configuration as it would be saved.
    pub fn preview(&self) -> Result<Config, ConfigError> {
        Config::parse(&self.doc.to_string())
    }

    /// Writes the file atomically, after checking it is still valid.
    pub fn save(&self) -> Result<Config, ConfigError> {
        let text = self.doc.to_string();
        let config = Config::parse(&text)?;
        DiskFs.write_atomic(&self.path, text.as_bytes())?;
        Ok(config)
    }

    fn pair_table(&mut self, name: &str) -> Result<&mut Table, ConfigError> {
        self.doc
            .get_mut("pair")
            .and_then(Item::as_array_of_tables_mut)
            .and_then(|pairs| pairs.iter_mut().find(|table| table_id(table) == Some(name)))
            .ok_or_else(|| ConfigError::UnknownPair(name.to_owned()))
    }
}

/// The id of a `[[pair]]` table: its `id`, or its name for older pairs.
fn table_id(table: &Table) -> Option<&str> {
    table
        .get("id")
        .or_else(|| table.get("name"))
        .and_then(Item::as_str)
}

fn comma_separated(text: &str) -> Array {
    text.split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .collect()
}

fn read_existing(path: &Path) -> Result<Vec<u8>, ConfigError> {
    match DiskFs.read(path) {
        Ok(bytes) => Ok(bytes),
        Err(FsError::NotFound(_)) => Err(ConfigError::Missing(path.to_path_buf())),
        Err(e) => Err(e.into()),
    }
}

/// Expands a leading `~` to the home folder.
pub fn expand_home(path: &str) -> PathBuf {
    let rest = match path.strip_prefix('~') {
        Some("") => "",
        Some(rest) if rest.starts_with(['/', '\\']) => &rest[1..],
        _ => return PathBuf::from(path),
    };
    match BaseDirs::new() {
        Some(dirs) => dirs.home_dir().join(rest),
        None => PathBuf::from(path),
    }
}

/// Whether one folder contains the other (or both are the same).
/// Where a folder sits relative to another one that shares files with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Nesting {
    /// Both are the same folder.
    Same,
    /// The folder is inside the other one.
    Inside,
    /// The other folder is inside this one.
    Contains,
}

/// Where `a` sits relative to `b`, if one contains the other.
pub fn nesting(a: &Path, b: &Path) -> Option<Nesting> {
    let a = a.canonicalize().unwrap_or_else(|_| a.to_path_buf());
    let b = b.canonicalize().unwrap_or_else(|_| b.to_path_buf());
    if a == b {
        Some(Nesting::Same)
    } else if a.starts_with(&b) {
        Some(Nesting::Inside)
    } else if b.starts_with(&a) {
        Some(Nesting::Contains)
    } else {
        None
    }
}

fn nested(a: &Path, b: &Path) -> bool {
    nesting(a, b).is_some()
}

/// Why a pair cannot have this folder, given the pair `other` that has it.
fn overlap_message(side: Side, nesting: Nesting, other: &Pair) -> String {
    let folder = other.root(side).display();
    let side = match side {
        Side::Kb => "kb",
        Side::Repo => "repo",
    };
    let place = match nesting {
        Nesting::Same => "is the same as",
        Nesting::Inside => "is inside",
        Nesting::Contains => "contains",
    };
    format!(
        "its {side} folder {place} the {side} folder of pair \"{}\" ({}): \
         a file can only be synced by one pair, choose a folder outside it",
        other.label(),
        folder
    )
}

fn in_git_repository(folder: &Path) -> bool {
    folder.ancestors().any(|dir| dir.join(".git").exists())
}

#[cfg(test)]
mod tests {
    fn pair_file() -> (tempfile::TempDir, ConfigFile) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("markoo.toml");
        std::fs::write(
            &path,
            "# mine\n[schedule]\ninterval = \"5m\" # often\n\n[[pair]]\nname = \"a\"\nkb = \"/k\"\nrepo = \"/r\"\n",
        )
        .unwrap();
        let file = ConfigFile::open(&path).unwrap();
        (dir, file)
    }

    #[test]
    fn options_of_a_pair_can_be_set() {
        let (_dir, mut file) = pair_file();
        file.set("a", "transform.links", "keep").unwrap();
        file.set("a", "transform.assets_dir", "img").unwrap();
        file.set("a", "kb_options.callouts", "keep").unwrap();
        file.set("a", "kb_options.frontmatter_drop", "x, y")
            .unwrap();
        file.set("a", "hooks.install", "post-commit").unwrap();
        file.set("a", "hooks.branches", "main").unwrap();
        let config = file.preview().unwrap();
        let pair = &config.pair("a").unwrap().pair;
        assert!(!pair.kb_options.convert_links);
        assert_eq!(pair.assets_dir, "img");
        assert!(!pair.kb_options.convert_callouts);
        assert_eq!(pair.kb_options.frontmatter_drop, ["x", "y"]);
        assert_eq!(pair.hooks.install, [crate::hooks::HookName::PostCommit]);
        assert_eq!(pair.hooks.branches, ["main"]);
    }

    #[test]
    fn an_unknown_option_of_a_pair_is_refused() {
        let (_dir, mut file) = pair_file();
        assert!(matches!(
            file.set("a", "transform.nope", "x"),
            Err(ConfigError::UnknownKey(_))
        ));
    }

    #[test]
    fn a_global_setting_keeps_the_comments() {
        let (_dir, mut file) = pair_file();
        file.set_global("schedule.on_startup", "false").unwrap();
        let text = file.doc.to_string();
        assert!(
            text.contains("# mine") && text.contains("# often"),
            "{text}"
        );
        assert!(!file.preview().unwrap().schedule.on_startup);
    }

    #[test]
    fn global_settings_create_their_section() {
        let (_dir, mut file) = pair_file();
        file.set_global("api.port", "5000").unwrap();
        file.set_global("mcp.allow_config_write", "true").unwrap();
        let config = file.preview().unwrap();
        assert_eq!(config.api.port, 5000);
        assert!(config.mcp.allow_config_write);
        assert_eq!(config.pairs.len(), 1);
    }

    #[test]
    fn the_language_is_read_and_changed() {
        let (_dir, mut file) = pair_file();
        assert_eq!(file.preview().unwrap().language, Language::En);
        file.set_global("general.language", "fr").unwrap();
        assert_eq!(file.preview().unwrap().language, Language::Fr);
        assert!(file.set_global("general.language", "de").is_err());
        assert!(Config::parse("[general]\nlanguage = \"de\"\n").is_err());
    }

    #[test]
    fn an_invalid_global_setting_is_refused() {
        let (_dir, mut file) = pair_file();
        assert!(file.set_global("api.port", "99999").is_err());
        assert!(file.set_global("api.other", "1").is_err());
    }

    #[test]
    fn mcp_defaults_to_http_without_config_writes() {
        let config = Config::parse("").unwrap();
        assert!(config.mcp.http && !config.mcp.allow_config_write);
    }

    #[test]
    fn mcp_settings_are_read() {
        let config = Config::parse("[mcp]\nhttp=false\nallow_config_write=true\n").unwrap();
        assert!(!config.mcp.http && config.mcp.allow_config_write);
    }

    #[test]
    fn an_unknown_mcp_setting_is_refused() {
        assert!(matches!(
            Config::parse("[mcp]\nhtp=false\n"),
            Err(ConfigError::Invalid(_))
        ));
    }

    #[test]
    fn hooks_are_read_from_the_pair() {
        let text = "[[pair]]\nname='a'\nkb='/k'\nrepo='/r'\n[pair.hooks]\ninstall=['post-commit']\nbranches=['main']\n";
        let config = Config::parse(text).unwrap();
        let entry = &config.pairs[0];
        assert_eq!(
            entry.pair.hooks.install,
            [crate::hooks::HookName::PostCommit]
        );
        assert_eq!(entry.pair.hooks.branches, ["main"]);
        assert!(entry.not_implemented.is_empty());
    }

    #[test]
    fn hooks_default_to_post_merge_on_every_branch() {
        let text = "[[pair]]\nname='a'\nkb='/k'\nrepo='/r'\n";
        let hooks = &Config::parse(text).unwrap().pairs[0].pair.hooks;
        assert_eq!(hooks.install, [crate::hooks::HookName::PostMerge]);
        assert!(hooks.branches.is_empty());
    }

    #[test]
    fn an_unknown_hook_is_refused() {
        let text = "[[pair]]\nname='a'\nkb='/k'\nrepo='/r'\n[pair.hooks]\ninstall=['post-push']\n";
        assert!(matches!(Config::parse(text), Err(ConfigError::Invalid(_))));
    }

    #[test]
    fn interval_is_parsed_in_seconds_minutes_and_hours() {
        let durations = ["30s", "5m", "1h"].map(|text| parse_interval(text).unwrap().unwrap());
        assert_eq!(durations.map(|d| d.as_secs()), [30, 300, 3600]);
    }

    #[test]
    fn interval_off_disables_the_schedule() {
        assert_eq!(parse_interval("off").unwrap(), None);
    }

    #[test]
    fn invalid_intervals_are_refused() {
        for text in [
            "",
            "m",
            "0s",
            "5",
            "5d",
            "-5m",
            "é",
            "99999999999999999999h",
        ] {
            assert!(parse_interval(text).is_err(), "{text}");
        }
    }

    #[test]
    fn schedule_and_api_have_defaults() {
        let config = Config::parse("").unwrap();
        assert_eq!(
            (config.schedule, config.api),
            (Schedule::default(), Api::default())
        );
    }

    #[test]
    fn schedule_and_api_are_read() {
        let config = Config::parse(
            "[schedule]\ninterval = \"off\"\non_startup = false\n[api]\nenabled = false\nport = 5000\n",
        )
        .unwrap();
        assert_eq!(
            (
                config.schedule.interval,
                config.schedule.on_startup,
                config.api.enabled,
                config.api.port
            ),
            (None, false, false, 5000)
        );
    }

    #[test]
    fn pause_on_battery_is_reported_as_not_implemented() {
        let config = Config::parse("[schedule]\npause_on_battery = true\n").unwrap();
        assert_eq!(config.not_implemented, ["schedule.pause_on_battery"]);
    }

    use super::*;

    const SAMPLE: &str = r#"
# My pairs
[general]
log_level = "info"

# The Clara docs
[[pair]]
name = "clara"   # short name
kb = "/kb"
repo = "/repo"
direction = "kb-to-repo"
exclude = ["drafts/**"]
"#;

    fn sample() -> Config {
        Config::parse(SAMPLE).unwrap()
    }

    #[test]
    fn parses_direction() {
        assert_eq!(sample().pairs[0].pair.direction, Direction::KbToRepo);
    }

    #[test]
    fn default_include_applies() {
        assert_eq!(sample().pairs[0].pair.include, vec!["**/*.md".to_owned()]);
    }

    #[test]
    fn typo_in_a_key_is_refused() {
        let text = "[[pair]]\nname='a'\nkb='/k'\nrepo='/r'\ndirecton='kb-to-repo'\n";
        assert!(matches!(Config::parse(text), Err(ConfigError::Invalid(_))));
    }

    #[test]
    fn invalid_direction_is_refused() {
        let text = "[[pair]]\nname='a'\nkb='/k'\nrepo='/r'\ndirection='sideways'\n";
        assert!(matches!(Config::parse(text), Err(ConfigError::Invalid(_))));
    }

    #[test]
    fn duplicate_names_are_refused() {
        let pair = "[[pair]]\nname='a'\nkb='/k'\nrepo='/r'\n";
        let text = format!("{pair}{pair}");
        assert!(matches!(
            Config::parse(&text),
            Err(ConfigError::DuplicatePair(_))
        ));
    }

    #[test]
    fn invalid_names_are_refused() {
        let text = "[[pair]]\nname='My Pair'\nkb='/k'\nrepo='/r'\n";
        assert!(matches!(
            Config::parse(text),
            Err(ConfigError::InvalidName(_))
        ));
    }

    #[test]
    fn unavailable_kb_type_is_refused_with_a_hint() {
        let text = "[[pair]]\nname='a'\nkb='/k'\nkb_type='logseq'\nrepo='/r'\n";
        assert!(matches!(
            Config::parse(text),
            Err(ConfigError::UnsupportedKbType { .. })
        ));
    }

    #[test]
    fn options_not_implemented_are_reported() {
        let text = "[[pair]]\nname='a'\nkb='/k'\nrepo='/r'\n[pair.transform]\nfilenames='kebab'\n";
        assert_eq!(
            Config::parse(text).unwrap().pairs[0].not_implemented,
            vec!["transform.filenames".to_owned()]
        );
    }

    #[test]
    fn transform_options_apply() {
        let text = "[[pair]]\nname='a'\nkb='/k'\nrepo='/r'\n[pair.transform]\nlinks='keep'\nassets_dir='img'\nfrontmatter='drop'\n";
        let pair = &Config::parse(text).unwrap().pairs[0].pair;
        assert_eq!(
            (
                pair.kb_options.convert_links,
                pair.assets_dir.as_str(),
                pair.kb_options.frontmatter_drop_all
            ),
            (false, "img", true)
        );
    }

    #[test]
    fn typo_in_a_transform_option_is_refused() {
        let text = "[[pair]]\nname='a'\nkb='/k'\nrepo='/r'\n[pair.transform]\nlink='keep'\n";
        assert!(matches!(Config::parse(text), Err(ConfigError::Invalid(_))));
    }

    #[test]
    fn kb_options_apply() {
        let text = "[[pair]]\nname='a'\nkb='/k'\nrepo='/r'\n[pair.kb_options]\ncallouts='keep'\nfrontmatter_drop=['x']\n";
        let options = &Config::parse(text).unwrap().pairs[0].pair.kb_options;
        assert_eq!(
            (options.convert_callouts, options.frontmatter_drop.clone()),
            (false, vec!["x".to_owned()])
        );
    }

    #[test]
    fn unknown_kb_option_is_reported() {
        let text = "[[pair]]\nname='a'\nkb='/k'\nrepo='/r'\n[pair.kb_options]\nnope=1\n";
        assert_eq!(
            Config::parse(text).unwrap().pairs[0].unknown_options,
            vec!["nope".to_owned()]
        );
    }

    #[test]
    fn tilde_expands_to_home() {
        assert!(!expand_home("~/notes").starts_with("~"));
    }

    #[test]
    fn other_paths_are_left_alone() {
        assert_eq!(expand_home("/a/~b"), PathBuf::from("/a/~b"));
    }

    fn folders(dir: &Path, names: &[&str]) {
        for name in names {
            std::fs::create_dir_all(dir.join(name)).unwrap();
        }
    }

    fn pair_text(name: &str, kb: &Path, repo: &Path) -> String {
        format!(
            "[[pair]]\nname = '{name}'\nkb = '{}'\nrepo = '{}'\n",
            kb.display(),
            repo.display()
        )
    }

    #[test]
    fn missing_folder_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        folders(dir.path(), &["kb"]);
        let text = pair_text("a", &dir.path().join("kb"), &dir.path().join("nope"));
        assert!(!Config::parse(&text).unwrap().validate().is_ok("a"));
    }

    #[test]
    fn nested_kb_and_repo_are_an_error() {
        let dir = tempfile::tempdir().unwrap();
        folders(dir.path(), &["kb/docs"]);
        let kb = dir.path().join("kb");
        let text = pair_text("a", &kb, &kb.join("docs"));
        assert!(!Config::parse(&text).unwrap().validate().is_ok("a"));
    }

    #[test]
    fn overlapping_pairs_are_an_error() {
        let dir = tempfile::tempdir().unwrap();
        folders(dir.path(), &["kb", "kb/sub", "repo1", "repo2"]);
        let text = format!(
            "{}{}",
            pair_text("a", &dir.path().join("kb"), &dir.path().join("repo1")),
            pair_text("b", &dir.path().join("kb/sub"), &dir.path().join("repo2"))
        );
        let validation = Config::parse(&text).unwrap().validate();
        assert!(!validation.is_ok("b"));
        let message = &validation.errors[0].message;
        assert!(message.contains("kb folder is inside the kb folder of pair \"a\""));
        assert!(message.contains(&dir.path().join("kb").display().to_string()));
    }

    #[test]
    fn the_pair_sharing_a_folder_is_found() {
        let dir = tempfile::tempdir().unwrap();
        folders(dir.path(), &["kb/sub", "repo", "other"]);
        let text = pair_text("a", &dir.path().join("kb"), &dir.path().join("repo"));
        let config = Config::parse(&text).unwrap();
        let found = |side, folder: &str| {
            config
                .overlapping_pair(side, &dir.path().join(folder))
                .map(|(entry, nesting)| (entry.pair.id.as_str(), nesting))
        };
        assert_eq!(found(Side::Kb, "kb/sub"), Some(("a", Nesting::Inside)));
        assert_eq!(found(Side::Kb, "kb"), Some(("a", Nesting::Same)));
        assert_eq!(found(Side::Kb, ""), Some(("a", Nesting::Contains)));
        assert_eq!(found(Side::Kb, "other"), None);
        assert_eq!(found(Side::Repo, "kb/sub"), None);
    }

    #[test]
    fn obsidian_vault_is_detected_from_a_subfolder() {
        let dir = tempfile::tempdir().unwrap();
        folders(dir.path(), &["vault/.obsidian", "vault/notes", "repo"]);
        let text = pair_text(
            "a",
            &dir.path().join("vault/notes"),
            &dir.path().join("repo"),
        );
        assert_eq!(
            Config::parse(&text).unwrap().pairs[0].pair.kb_type,
            KbType::Obsidian
        );
    }

    #[test]
    fn obsidian_vault_can_be_forced_to_plain_markdown() {
        let dir = tempfile::tempdir().unwrap();
        folders(dir.path(), &["vault/.obsidian", "repo"]);
        let text = format!(
            "{}kb_type = 'markdown'\n",
            pair_text("a", &dir.path().join("vault"), &dir.path().join("repo"))
        );
        assert_eq!(
            Config::parse(&text).unwrap().pairs[0].pair.kb_type,
            KbType::Markdown
        );
    }

    #[test]
    fn obsidian_type_outside_a_vault_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        folders(dir.path(), &["kb", "repo"]);
        let text = format!(
            "{}kb_type = 'obsidian'\n",
            pair_text("a", &dir.path().join("kb"), &dir.path().join("repo"))
        );
        assert!(!Config::parse(&text).unwrap().validate().is_ok("a"));
    }

    #[test]
    fn assets_dir_cannot_leave_the_repo() {
        let dir = tempfile::tempdir().unwrap();
        folders(dir.path(), &["kb", "repo"]);
        let text = format!(
            "{}[pair.transform]\nassets_dir = '../outside'\n",
            pair_text("a", &dir.path().join("kb"), &dir.path().join("repo"))
        );
        assert!(!Config::parse(&text).unwrap().validate().is_ok("a"));
    }

    #[test]
    fn repo_outside_git_is_only_a_warning() {
        let dir = tempfile::tempdir().unwrap();
        folders(dir.path(), &["kb", "repo"]);
        let text = pair_text("a", &dir.path().join("kb"), &dir.path().join("repo"));
        let validation = Config::parse(&text).unwrap().validate();
        assert_eq!((validation.errors.len(), validation.warnings.len()), (0, 1));
    }

    fn edited(edit: impl FnOnce(&mut ConfigFile)) -> String {
        let mut file = ConfigFile::from_text(Path::new("/unused"), SAMPLE).unwrap();
        edit(&mut file);
        file.doc.to_string()
    }

    #[test]
    fn editing_preserves_comments() {
        let text = edited(|file| file.set("clara", "direction", "both").unwrap());
        assert!(text.contains("# The Clara docs") && text.contains("# short name"));
    }

    #[test]
    fn setting_a_value_changes_it() {
        let text = edited(|file| file.set("clara", "direction", "repo-to-kb").unwrap());
        assert_eq!(
            Config::parse(&text).unwrap().pairs[0].pair.direction,
            Direction::RepoToKb
        );
    }

    #[test]
    fn hyphenated_keys_are_accepted() {
        let text = edited(|file| file.set("clara", "target-changes", "overwrite").unwrap());
        assert_eq!(
            Config::parse(&text).unwrap().pairs[0].pair.target_changes,
            TargetChanges::Overwrite
        );
    }

    #[test]
    fn lists_are_comma_separated() {
        let text = edited(|file| file.set("clara", "exclude", "a/**, b/**").unwrap());
        assert_eq!(
            Config::parse(&text).unwrap().pairs[0].pair.exclude,
            vec!["a/**".to_owned(), "b/**".to_owned()]
        );
    }

    #[test]
    fn unknown_setting_is_refused() {
        let mut file = ConfigFile::from_text(Path::new("/unused"), SAMPLE).unwrap();
        assert!(matches!(
            file.set("clara", "nope", "x"),
            Err(ConfigError::UnknownKey(_))
        ));
    }

    #[test]
    fn adding_a_pair_keeps_the_others() {
        let text = edited(|file| {
            file.add_pair(&NewPair {
                name: "two".into(),
                kb: "/k2".into(),
                repo: "/r2".into(),
                ..NewPair::default()
            })
            .unwrap();
        });
        assert_eq!(Config::parse(&text).unwrap().pairs.len(), 2);
    }

    #[test]
    fn first_pair_is_written_below_the_template_comments() {
        let mut file = ConfigFile::from_text(Path::new("/unused"), TEMPLATE).unwrap();
        file.add_pair(&NewPair {
            name: "two".into(),
            kb: "/k".into(),
            repo: "/r".into(),
            ..NewPair::default()
        })
        .unwrap();
        let text = file.doc.to_string();
        assert!(text.find("# Markoo configuration") < text.find("name = \"two\""));
    }

    #[test]
    fn removing_a_pair_keeps_the_general_section() {
        let text = edited(|file| file.remove_pair("clara").unwrap());
        assert!(text.contains("log_level") && Config::parse(&text).unwrap().pairs.is_empty());
    }

    #[test]
    fn invalid_edit_never_reaches_the_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("markoo.toml");
        DiskFs.write_atomic(&path, SAMPLE.as_bytes()).unwrap();
        let mut file = ConfigFile::open(&path).unwrap();
        file.set("clara", "direction", "sideways").unwrap();
        assert!(file.save().is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), SAMPLE);
    }

    #[test]
    fn create_refuses_to_replace_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("markoo.toml");
        ConfigFile::create(&path).unwrap();
        assert!(matches!(
            ConfigFile::create(&path),
            Err(ConfigError::AlreadyExists(_))
        ));
    }

    #[test]
    fn the_project_of_a_pair_is_read() {
        let text = "[[pair]]\nname='a'\nproject='Markoo'\nkb='/k'\nrepo='/r'\n\
                    [[pair]]\nname='b'\nkb='/k2'\nrepo='/r2'\n";
        let config = Config::parse(text).unwrap();
        assert_eq!(config.pairs[0].pair.project.as_deref(), Some("Markoo"));
        assert_eq!(config.pairs[1].pair.project, None);
    }

    #[test]
    fn invalid_project_names_are_refused() {
        let too_long = "x".repeat(MAX_PROJECT_LEN + 1);
        for project in ["", " Markoo", "a\\nb", too_long.as_str()] {
            let text = format!("[[pair]]\nname='a'\nproject=\"{project}\"\nkb='/k'\nrepo='/r'\n");
            assert!(
                matches!(Config::parse(&text), Err(ConfigError::InvalidProject(_))),
                "{project:?}"
            );
        }
        assert!(is_valid_label("Site web — v2"));
    }

    #[test]
    fn projects_are_listed_once_in_file_order() {
        let text = "[[pair]]\nname='a'\nproject='Zeta'\nkb='/k1'\nrepo='/r1'\n\
                    [[pair]]\nname='b'\nkb='/k2'\nrepo='/r2'\n\
                    [[pair]]\nname='c'\nproject='Alpha'\nkb='/k3'\nrepo='/r3'\n\
                    [[pair]]\nname='d'\nproject='Zeta'\nkb='/k4'\nrepo='/r4'\n";
        let config = Config::parse(text).unwrap();
        assert_eq!(config.projects(), ["Zeta", "Alpha"]);
        assert_eq!(config.known_project(" zeta "), Some("Zeta"));
        assert_eq!(config.known_project("Beta"), None);
    }

    #[test]
    fn a_new_pair_is_written_with_its_project() {
        let text = edited(|file| {
            file.add_pair(&NewPair {
                name: "two".into(),
                project: Some("Markoo".into()),
                kb: "/k2".into(),
                repo: "/r2".into(),
                ..NewPair::default()
            })
            .unwrap();
        });
        let config = Config::parse(&text).unwrap();
        assert_eq!(
            config.pair("two").unwrap().pair.project.as_deref(),
            Some("Markoo")
        );
    }

    #[test]
    fn the_project_of_a_pair_can_be_changed() {
        let mut file = ConfigFile::from_text(Path::new("/unused"), SAMPLE).unwrap();
        file.set("clara", "project", "  Clara  ").unwrap();
        let config = file.preview().unwrap();
        assert_eq!(
            config.pair("clara").unwrap().pair.project.as_deref(),
            Some("Clara")
        );
        assert!(matches!(
            file.set("clara", "project", " "),
            Err(ConfigError::InvalidProject(_))
        ));
    }

    #[test]
    fn a_pair_without_an_id_is_known_by_its_name() {
        let config = Config::parse("[[pair]]\nname='notes'\nkb='/k'\nrepo='/r'\n").unwrap();
        assert_eq!(config.pairs[0].pair.id, "notes");
    }

    #[test]
    fn a_pair_name_is_free_text() {
        let text = "[[pair]]\nid='m-doc'\nname='Documentation technique'\nkb='/k'\nrepo='/r'\n";
        let pair = &Config::parse(text).unwrap().pairs[0].pair;
        assert_eq!(pair.name, "Documentation technique");
        for name in ["", "a/b", " x"] {
            let text = format!("[[pair]]\nid='m'\nname=\"{name}\"\nkb='/k'\nrepo='/r'\n");
            assert!(
                matches!(Config::parse(&text), Err(ConfigError::InvalidLabel(_))),
                "{name:?}"
            );
        }
    }

    #[test]
    fn ids_are_unique_and_names_unique_within_a_project() {
        let pair = |id: &str, name: &str, project: &str| {
            format!(
                "[[pair]]\nid='{id}'\nname='{name}'\nproject='{project}'\nkb='/k{id}'\nrepo='/r{id}'\n"
            )
        };
        let same_id = pair("a", "x", "P") + &pair("a", "y", "Q");
        assert!(matches!(
            Config::parse(&same_id),
            Err(ConfigError::DuplicatePair(_))
        ));
        let same_name = pair("a", "Docs", "P") + &pair("b", "docs", "p");
        assert!(matches!(
            Config::parse(&same_name),
            Err(ConfigError::DuplicateName { .. })
        ));
        let other_project = pair("a", "Docs", "P") + &pair("b", "Docs", "Q");
        assert!(Config::parse(&other_project).is_ok());
    }

    #[test]
    fn new_ids_come_from_the_project_and_the_name() {
        let config =
            Config::parse("[[pair]]\nid='site-web-docs'\nname='x'\nkb='/k'\nrepo='/r'\n").unwrap();
        assert_eq!(config.new_id(Some("Site Web"), "Docs"), "site-web-docs-2");
        assert_eq!(
            config.new_id(Some("Markoo"), "Skill Claude"),
            "markoo-skill-claude"
        );
        assert_eq!(config.new_id(Some("Équipe"), "équipe"), "equipe");
        assert_eq!(config.new_id(Some("日本"), "日本"), "pair");
    }

    #[test]
    fn a_pair_is_found_by_id_project_and_name_or_name() {
        let text = "[[pair]]\nid='m-doc'\nname='Docs'\nproject='Markoo'\nkb='/k1'\nrepo='/r1'\n\
                    [[pair]]\nid='c-doc'\nname='Docs'\nproject='Clara'\nkb='/k2'\nrepo='/r2'\n\
                    [[pair]]\nid='c-skill'\nname='Skill'\nproject='Clara'\nkb='/k3'\nrepo='/r3'\n";
        let config = Config::parse(text).unwrap();
        let id = |reference: &str| config.pair(reference).map(|entry| entry.pair.id.as_str());
        assert_eq!(id("c-doc").unwrap(), "c-doc");
        assert_eq!(id("markoo/docs").unwrap(), "m-doc");
        assert_eq!(id("skill").unwrap(), "c-skill");
        assert!(matches!(id("Docs"), Err(ConfigError::AmbiguousPair { .. })));
        assert!(matches!(id("Clara/Nope"), Err(ConfigError::UnknownPair(_))));
    }

    #[test]
    fn renaming_a_pair_without_an_id_writes_its_id_first() {
        let mut file = ConfigFile::from_text(Path::new("/unused"), SAMPLE).unwrap();
        file.set("clara", "name", "Docs Clara").unwrap();
        let config = file.preview().unwrap();
        let pair = &config.pair("clara").unwrap().pair;
        assert_eq!(
            (pair.id.as_str(), pair.name.as_str()),
            ("clara", "Docs Clara")
        );
        assert!(matches!(
            file.set("clara", "id", "x"),
            Err(ConfigError::ImmutableId)
        ));
    }

    #[test]
    fn template_is_a_valid_empty_configuration() {
        assert!(Config::parse(TEMPLATE).unwrap().pairs.is_empty());
    }
}
