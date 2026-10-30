//! Reading, validating and editing `markoo.toml`.
//!
//! The file stays the source of truth: edits go through `toml_edit`, so the
//! user's comments and layout survive, and an edit that would make the file
//! invalid is refused before anything reaches the disk.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use directories::BaseDirs;
use serde::Deserialize;
use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, value};

use crate::fs::{DiskFs, Fs, FsError};
use crate::kb::KbType;
use crate::pair::{Direction, Pair, TargetChanges};

/// Content of a newly created configuration file.
pub const TEMPLATE: &str = "\
# Markoo configuration: https://github.com/studoo-app/markoo
# Add a pair with `markoo pair add`, or by hand:
#
# [[pair]]
# name = \"notes\"
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
    /// A pair already has this name.
    #[error("a pair named \"{0}\" already exists")]
    DuplicatePair(String),
    /// The pair name cannot be used in commands and URLs.
    #[error("invalid pair name \"{0}\": use lowercase letters, digits and hyphens")]
    InvalidName(String),
    /// This KB type has no adapter yet.
    #[error("pair \"{pair}\": kb_type \"{kb_type}\" is not available yet; use \"markdown\"")]
    UnsupportedKbType {
        /// The pair.
        pair: String,
        /// The requested type.
        kb_type: String,
    },
    /// This key cannot be set.
    #[error("unknown pair setting \"{0}\"")]
    UnknownKey(String),
}

/// One `[[pair]]` table as written in the file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPair {
    name: String,
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
    hooks: Option<toml::Value>,
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

/// A parsed configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    /// Pairs, in file order.
    pub pairs: Vec<PairEntry>,
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
        let mut names = BTreeSet::new();
        let mut pairs = Vec::with_capacity(raw.pair.len());
        for raw in raw.pair {
            if !is_valid_name(&raw.name) {
                return Err(ConfigError::InvalidName(raw.name));
            }
            if !names.insert(raw.name.clone()) {
                return Err(ConfigError::DuplicatePair(raw.name));
            }
            pairs.push(PairEntry::from_raw(raw)?);
        }
        Ok(Self { pairs })
    }

    /// Finds a pair by name.
    pub fn pair(&self, name: &str) -> Result<&PairEntry, ConfigError> {
        self.pairs
            .iter()
            .find(|entry| entry.pair.name == name)
            .ok_or_else(|| ConfigError::UnknownPair(name.to_owned()))
    }

    /// Checks every pair against the disk.
    pub fn validate(&self) -> Validation {
        let mut validation = Validation::default();
        for entry in &self.pairs {
            entry.validate(&mut validation);
        }
        for (index, first) in self.pairs.iter().enumerate() {
            for second in &self.pairs[index + 1..] {
                for (side, a, b) in [
                    ("kb", &first.pair.kb, &second.pair.kb),
                    ("repo", &first.pair.repo, &second.pair.repo),
                ] {
                    if nested(a, b) {
                        validation.errors.push(Problem {
                            pair: second.pair.name.clone(),
                            message: format!(
                                "its {side} folder overlaps with pair \"{}\"",
                                first.pair.name
                            ),
                        });
                    }
                }
            }
        }
        validation
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
                    pair: raw.name,
                    kb_type: other.to_owned(),
                });
            }
        };
        let mut not_implemented = Vec::new();
        if raw.conflict.as_deref().is_some_and(|c| c != "ask") {
            not_implemented.push("conflict (conflicts are always put on hold)".to_owned());
        }
        if raw.hooks.is_some() {
            not_implemented.push("hooks".to_owned());
        }
        let mut pair = Pair::new(raw.name, kb, expand_home(&raw.repo));
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
        let name = &self.pair.name;
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

/// Whether a pair name can be used in commands and URLs.
pub fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Settings of a new pair.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewPair {
    /// Name of the pair.
    pub name: String,
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
        table["name"] = value(&new.name);
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

    /// Changes a setting of a pair. Lists are comma-separated.
    pub fn set(&mut self, name: &str, key: &str, new_value: &str) -> Result<(), ConfigError> {
        let key = key.replace('-', "_");
        let table = self.pair_table(name)?;
        match key.as_str() {
            "direction" | "target_changes" | "kb" | "repo" | "kb_type" | "conflict" | "name" => {
                table[key.as_str()] = value(new_value);
            }
            "paused" => {
                let paused = new_value.parse::<bool>().map_err(|_| {
                    ConfigError::Invalid(format!("paused must be true or false, not {new_value}"))
                })?;
                table["paused"] = value(paused);
            }
            "include" | "exclude" => {
                let patterns: Array = new_value
                    .split(',')
                    .map(str::trim)
                    .filter(|pattern| !pattern.is_empty())
                    .collect();
                table[key.as_str()] = value(patterns);
            }
            _ => return Err(ConfigError::UnknownKey(key)),
        }
        Ok(())
    }

    /// Removes a pair.
    pub fn remove_pair(&mut self, name: &str) -> Result<(), ConfigError> {
        let pairs = self
            .doc
            .get_mut("pair")
            .and_then(Item::as_array_of_tables_mut)
            .ok_or_else(|| ConfigError::UnknownPair(name.to_owned()))?;
        let index = pairs
            .iter()
            .position(|table| table.get("name").and_then(Item::as_str) == Some(name))
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
            .and_then(|pairs| {
                pairs
                    .iter_mut()
                    .find(|table| table.get("name").and_then(Item::as_str) == Some(name))
            })
            .ok_or_else(|| ConfigError::UnknownPair(name.to_owned()))
    }
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
fn nested(a: &Path, b: &Path) -> bool {
    let a = a.canonicalize().unwrap_or_else(|_| a.to_path_buf());
    let b = b.canonicalize().unwrap_or_else(|_| b.to_path_buf());
    a.starts_with(&b) || b.starts_with(&a)
}

fn in_git_repository(folder: &Path) -> bool {
    folder.ancestors().any(|dir| dir.join(".git").exists())
}

#[cfg(test)]
mod tests {
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
        assert!(!Config::parse(&text).unwrap().validate().is_ok("b"));
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
    fn template_is_a_valid_empty_configuration() {
        assert!(Config::parse(TEMPLATE).unwrap().pairs.is_empty());
    }
}
