//! Knowledge base adapters and Markdown dialects.
//!
//! Every side of a pair speaks a [`Dialect`]. The engine compares and merges
//! the **canonical form** (GitHub Flavored Markdown with relative links, plus
//! invisible `<!--markoo:…-->` comments) and only ever talks to these traits,
//! never to a specific tool.

pub mod obsidian;

use std::cell::{OnceCell, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

use crate::entry::EntryPath;
use crate::fs::{Fs, FsError};
use crate::pair::{Filter, Pair};

/// Errors raised while converting between a dialect and the canonical form.
#[derive(Debug, thiserror::Error)]
#[error("{dialect}: {message}")]
pub struct DialectError {
    /// Dialect that failed.
    pub dialect: &'static str,
    /// Description of the failure.
    pub message: String,
}

/// What a conversion needs to know about the surroundings of a file.
#[derive(Debug, Clone, Copy)]
pub struct ConvertCtx<'a> {
    /// The file being converted, relative to the pair's root.
    pub path: &'a EntryPath,
    /// The files of the knowledge base.
    pub index: &'a NameIndex,
}

/// Conversion between a Markdown dialect and the canonical form.
///
/// Implementations must round-trip losslessly: `from_canonical(to_canonical(y))
/// == y` for any file `y` of the dialect, and `to_canonical` must be stable on
/// what `from_canonical` produces, otherwise files would look changed at every
/// sync.
pub trait Dialect {
    /// Identifier of the dialect (e.g. `"gfm"`, `"obsidian"`).
    fn id(&self) -> &'static str;
    /// Converts a file of this dialect to canonical form.
    fn to_canonical(&self, text: &str, ctx: &ConvertCtx<'_>) -> Result<String, DialectError>;
    /// Converts canonical text to this dialect.
    // The name mirrors `to_canonical` and the design docs; the method needs
    // `&self` because dialects carry options.
    #[expect(clippy::wrong_self_convention)]
    fn from_canonical(&self, text: &str, ctx: &ConvertCtx<'_>) -> Result<String, DialectError>;
}

/// A knowledge base type.
pub trait KbAdapter: Dialect {
    /// Glob patterns the adapter always ignores (tool settings, trash…),
    /// on top of the pair's own filters.
    fn excluded(&self) -> &'static [&'static str];

    /// Folder names ignored anywhere in the knowledge base.
    fn excluded_folders(&self) -> &'static [&'static str] {
        &[]
    }

    /// Names of the attachments a canonical text refers to.
    fn referenced_assets(&self, _canonical: &str, _ctx: &ConvertCtx<'_>) -> Vec<String> {
        Vec::new()
    }
}

/// Supported values of the `kb_type` key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum KbType {
    /// A plain Markdown folder, without any conversion.
    #[default]
    Markdown,
    /// An Obsidian vault.
    Obsidian,
}

impl KbType {
    /// The `markoo.toml` value of this type.
    pub fn id(self) -> &'static str {
        match self {
            KbType::Markdown => "markdown",
            KbType::Obsidian => "obsidian",
        }
    }

    /// Recognizes a knowledge base from its folder, most specific type first.
    /// `markdown` is the fallback.
    pub fn detect(folder: &Path) -> Self {
        if obsidian::find_vault_root(&crate::fs::DiskFs, folder).is_some() {
            KbType::Obsidian
        } else {
            KbType::Markdown
        }
    }
}

/// Options of a knowledge base adapter (`[pair.kb_options]`, plus the generic
/// `links` and `frontmatter` of `[pair.transform]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KbOptions {
    /// Convert links (`links = "markdown"`), or leave them (`"keep"`).
    pub convert_links: bool,
    /// Convert callouts to GitHub alerts (`callouts = "github"`).
    pub convert_callouts: bool,
    /// Frontmatter keys kept out of the repo side.
    pub frontmatter_drop: Vec<String>,
    /// Keep the whole frontmatter out of the repo side (`frontmatter = "drop"`).
    pub frontmatter_drop_all: bool,
}

impl Default for KbOptions {
    fn default() -> Self {
        Self {
            convert_links: true,
            convert_callouts: true,
            frontmatter_drop: ["cssclasses", "aliases", "publish"]
                .map(str::to_owned)
                .to_vec(),
            frontmatter_drop_all: false,
        }
    }
}

/// Plain Markdown folder: the KB already is in canonical form.
#[derive(Debug, Clone, Copy, Default)]
pub struct MarkdownAdapter;

impl Dialect for MarkdownAdapter {
    fn id(&self) -> &'static str {
        "markdown"
    }

    fn to_canonical(&self, text: &str, _ctx: &ConvertCtx<'_>) -> Result<String, DialectError> {
        Ok(text.to_owned())
    }

    fn from_canonical(&self, text: &str, _ctx: &ConvertCtx<'_>) -> Result<String, DialectError> {
        Ok(text.to_owned())
    }
}

impl KbAdapter for MarkdownAdapter {
    fn excluded(&self) -> &'static [&'static str] {
        &[]
    }
}

/// GitHub Flavored Markdown, the repo side's dialect. It is the canonical form
/// itself, so `markoo` comments are simply kept in the file (GitHub hides them).
#[derive(Debug, Clone, Copy, Default)]
pub struct Gfm;

impl Dialect for Gfm {
    fn id(&self) -> &'static str {
        "gfm"
    }

    fn to_canonical(&self, text: &str, _ctx: &ConvertCtx<'_>) -> Result<String, DialectError> {
        Ok(text.to_owned())
    }

    fn from_canonical(&self, text: &str, _ctx: &ConvertCtx<'_>) -> Result<String, DialectError> {
        Ok(text.to_owned())
    }
}

/// A file name as links match it: without regard to case, or to the way an
/// accent is encoded, which depends on the system that created the file.
fn comparable(name: &str) -> String {
    name.nfc().collect::<String>().to_lowercase()
}

/// Files by comparable name. A vault has thousands of files and notes have
/// thousands of links, so that lookups cannot go through all the files.
#[derive(Debug, Default)]
struct NameLookup {
    /// `(comparable path, path)` by comparable file name, ordered by path.
    by_name: HashMap<String, Vec<(String, String)>>,
}

impl NameLookup {
    fn of(files: &BTreeSet<String>) -> Self {
        let mut by_name: HashMap<String, Vec<(String, String)>> = HashMap::new();
        for file in files {
            let path = comparable(file);
            let name = path.rsplit('/').next().unwrap_or(&path).to_owned();
            by_name.entry(name).or_default().push((path, file.clone()));
        }
        Self { by_name }
    }

    /// Whether several files have this name.
    fn is_shared(&self, name: &str) -> bool {
        self.by_name
            .get(&comparable(name))
            .is_some_and(|files| files.len() > 1)
    }

    /// The files at the comparable path `wanted`, and with `in_any_folder`
    /// those whose path ends with it, as `(comparable path, path)`.
    fn matching(&self, wanted: &str, in_any_folder: bool) -> Vec<(&str, &str)> {
        let name = wanted.rsplit('/').next().unwrap_or(wanted);
        let suffix = format!("/{wanted}");
        self.by_name
            .get(name)
            .into_iter()
            .flatten()
            .filter(|(path, _)| path == wanted || (in_any_folder && path.ends_with(&suffix)))
            .map(|(path, file)| (path.as_str(), file.as_str()))
            .collect()
    }
}

/// The files of a knowledge base, used to resolve links by name.
///
/// It covers the whole vault, not only the pair, to tell a target outside the
/// pair from a missing one.
#[derive(Debug, Default)]
pub struct NameIndex {
    /// Notes, relative to the vault root, with their extension.
    notes: BTreeSet<String>,
    /// Other files, relative to the vault root.
    attachments: BTreeSet<String>,
    /// Attachments found in the repo's assets folder, by name.
    repo_assets: BTreeSet<String>,
    /// Path of the pair's KB folder relative to the vault root; empty at the root.
    pair_prefix: String,
    /// Tracked notes, relative to the pair's root.
    tracked: BTreeSet<String>,
    /// Assets folder relative to the pair's root.
    assets_dir: String,
    note_lookup: OnceCell<NameLookup>,
    attachment_lookup: OnceCell<NameLookup>,
    /// Vault path of the attachments renamed in the assets folder, by new name.
    renamed_assets: OnceCell<HashMap<String, String>>,
    warnings: RefCell<BTreeMap<String, String>>,
}

impl NameIndex {
    /// Builds the index of a pair; empty for the identity adapter, which needs none.
    pub fn build(
        fs: &impl Fs,
        pair: &Pair,
        filter: &Filter,
        vault_root: &Path,
        excluded: &[&str],
    ) -> Result<Self, FsError> {
        if pair.kb_type == KbType::Markdown {
            return Ok(Self::default());
        }
        let accepted = |relative: &str| {
            EntryPath::from_relative(Path::new(relative))
                .is_some_and(|entry| filter.accepts(&entry))
        };
        let is_excluded = |relative: &str| {
            excluded
                .iter()
                .any(|folder| relative.split('/').any(|part| part == *folder))
        };
        let mut index = Self {
            assets_dir: pair.assets_dir.clone(),
            pair_prefix: pair
                .kb
                .strip_prefix(vault_root)
                .ok()
                .and_then(EntryPath::from_relative)
                .map(|path| path.as_str().to_owned())
                .unwrap_or_default(),
            ..Self::default()
        };
        for relative in fs.list(vault_root)? {
            let Some(path) = EntryPath::from_relative(&relative) else {
                continue;
            };
            let path = path.as_str();
            if is_excluded(path) || path.ends_with(crate::fs::TMP_SUFFIX) {
                continue;
            }
            if path.to_lowercase().ends_with(".md") {
                index.notes.insert(path.to_owned());
                if let Some(inside) = index.pair_relative(path)
                    && accepted(&inside)
                {
                    index.tracked.insert(inside);
                }
            } else {
                index.attachments.insert(path.to_owned());
            }
        }
        // Notes that only exist in the repo so far: they are about to reach the
        // vault, and links to them must already convert the way they will later.
        for relative in fs.list(&pair.repo)? {
            let Some(path) = EntryPath::from_relative(&relative) else {
                continue;
            };
            let path = path.as_str();
            if path.to_lowercase().ends_with(".md") && accepted(path) {
                index.notes.insert(index.vault_relative(path));
                index.tracked.insert(path.to_owned());
            }
        }
        let assets = crate::entry::EntryPath::from_relative(Path::new(&pair.assets_dir))
            .map(|dir| dir.under(&pair.repo));
        if let Some(assets) = assets {
            for relative in fs.list(&assets)? {
                if let Some(name) = relative.to_str().filter(|name| !name.contains(['/', '\\'])) {
                    index.repo_assets.insert(name.to_owned());
                }
            }
        }
        Ok(index)
    }

    /// Records a problem to report, once per message.
    pub(crate) fn warn(&self, file: &EntryPath, message: String) {
        self.warnings
            .borrow_mut()
            .insert(message, file.as_str().to_owned());
    }

    /// Takes the problems recorded so far, as `(file, message)`.
    pub fn take_warnings(&self) -> Vec<(String, String)> {
        std::mem::take(&mut *self.warnings.borrow_mut())
            .into_iter()
            .map(|(message, file)| (file, message))
            .collect()
    }

    fn pair_relative(&self, vault_path: &str) -> Option<String> {
        if self.pair_prefix.is_empty() {
            return Some(vault_path.to_owned());
        }
        vault_path
            .strip_prefix(&self.pair_prefix)?
            .strip_prefix('/')
            .map(str::to_owned)
    }

    fn vault_relative(&self, pair_path: &str) -> String {
        if self.pair_prefix.is_empty() {
            pair_path.to_owned()
        } else {
            format!("{}/{pair_path}", self.pair_prefix)
        }
    }
}

/// Reads the app settings of a tool, ignoring a missing file.
pub(crate) fn read_optional(fs: &impl Fs, path: &Path) -> Option<Vec<u8>> {
    fs.read(path).ok()
}

/// Folders known to a tool, suggested when adding a pair.
pub fn known_kbs() -> Vec<PathBuf> {
    obsidian::known_vaults()
}

#[cfg(test)]
impl NameIndex {
    /// Builds an index from file lists, for tests.
    pub(crate) fn for_tests(notes: &[&str], attachments: &[&str], prefix: &str) -> Self {
        let notes: BTreeSet<String> = notes.iter().map(|n| (*n).to_owned()).collect();
        let mut index = Self {
            attachments: attachments.iter().map(|n| (*n).to_owned()).collect(),
            pair_prefix: prefix.to_owned(),
            assets_dir: "assets".to_owned(),
            ..Self::default()
        };
        for note in &notes {
            if let Some(inside) = index.pair_relative(note) {
                index.tracked.insert(inside);
            }
        }
        index.notes = notes;
        index
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# Title\n\nSee [ADR](./adr/001.md).\n";

    fn ctx_parts() -> (EntryPath, NameIndex) {
        (
            EntryPath::from_relative(Path::new("a.md")).unwrap(),
            NameIndex::default(),
        )
    }

    #[test]
    fn markdown_adapter_round_trips() {
        let (path, index) = ctx_parts();
        let ctx = ConvertCtx {
            path: &path,
            index: &index,
        };
        let canonical = MarkdownAdapter.to_canonical(SAMPLE, &ctx).unwrap();
        assert_eq!(
            MarkdownAdapter.from_canonical(&canonical, &ctx).unwrap(),
            SAMPLE
        );
    }

    #[test]
    fn gfm_round_trips() {
        let (path, index) = ctx_parts();
        let ctx = ConvertCtx {
            path: &path,
            index: &index,
        };
        let canonical = Gfm.to_canonical(SAMPLE, &ctx).unwrap();
        assert_eq!(Gfm.from_canonical(&canonical, &ctx).unwrap(), SAMPLE);
    }

    #[test]
    fn default_kb_type_is_markdown() {
        assert_eq!(KbType::default().id(), "markdown");
    }
}
