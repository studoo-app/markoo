//! Description of a synchronized pair (one `[[pair]]` of `markoo.toml`).

use std::path::PathBuf;

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};

use crate::entry::{EntryPath, Side};
use crate::fs::TMP_SUFFIX;
use crate::kb::KbType;

/// Patterns ignored on both sides of every pair, whatever the configuration.
const ALWAYS_EXCLUDED: &[&str] = &["**/.git/**", "**/node_modules/**"];

/// Which way changes flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Direction {
    /// Changes flow both ways and are merged.
    #[default]
    Both,
    /// The KB is the source, the repo the target.
    KbToRepo,
    /// The repo is the source, the KB the target.
    RepoToKb,
}

impl Direction {
    /// The source side in a one-way mode, `None` for [`Direction::Both`].
    pub fn source(self) -> Option<Side> {
        match self {
            Direction::Both => None,
            Direction::KbToRepo => Some(Side::Kb),
            Direction::RepoToKb => Some(Side::Repo),
        }
    }
}

/// What a one-way pair does with changes made in its target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TargetChanges {
    /// Never overwrite them: flag a divergence.
    #[default]
    Protect,
    /// Reset the target to the source (with backup).
    Overwrite,
}

/// A pair of folders kept in sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pair {
    /// Unique name of the pair.
    pub name: String,
    /// Knowledge base folder.
    pub kb: PathBuf,
    /// Type of the knowledge base.
    pub kb_type: KbType,
    /// Repository folder.
    pub repo: PathBuf,
    /// Sync direction.
    pub direction: Direction,
    /// Handling of target changes in one-way modes.
    pub target_changes: TargetChanges,
    /// Glob patterns of tracked files.
    pub include: Vec<String>,
    /// Glob patterns of ignored files.
    pub exclude: Vec<String>,
    /// A paused pair is skipped by every sync.
    pub paused: bool,
}

impl Pair {
    /// A bidirectional pair of plain Markdown folders tracking `**/*.md`.
    pub fn new(name: impl Into<String>, kb: impl Into<PathBuf>, repo: impl Into<PathBuf>) -> Self {
        Self {
            name: name.into(),
            kb: kb.into(),
            kb_type: KbType::default(),
            repo: repo.into(),
            direction: Direction::default(),
            target_changes: TargetChanges::default(),
            include: vec!["**/*.md".to_owned()],
            exclude: Vec::new(),
            paused: false,
        }
    }

    /// Root folder of a side.
    pub fn root(&self, side: Side) -> &std::path::Path {
        match side {
            Side::Kb => &self.kb,
            Side::Repo => &self.repo,
        }
    }
}

/// Decides which entries of a side are tracked.
#[derive(Debug, Clone)]
pub struct Filter {
    include: GlobSet,
    exclude: GlobSet,
}

impl Filter {
    /// Builds the filter of a pair, adding the always-excluded patterns and
    /// the adapter's own exclusions.
    pub fn new(pair: &Pair, adapter_excluded: &[&str]) -> Result<Self, globset::Error> {
        let excluded = pair
            .exclude
            .iter()
            .map(String::as_str)
            .chain(ALWAYS_EXCLUDED.iter().copied())
            .chain(adapter_excluded.iter().copied());
        Ok(Self {
            include: glob_set(pair.include.iter().map(String::as_str))?,
            exclude: glob_set(excluded)?,
        })
    }

    /// Whether an entry is tracked.
    pub fn accepts(&self, path: &EntryPath) -> bool {
        let path = path.as_str();
        !path.ends_with(TMP_SUFFIX) && self.include.is_match(path) && !self.exclude.is_match(path)
    }
}

fn glob_set<'a>(patterns: impl Iterator<Item = &'a str>) -> Result<GlobSet, globset::Error> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(GlobBuilder::new(pattern).literal_separator(true).build()?);
    }
    builder.build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn entry(path: &str) -> EntryPath {
        EntryPath::from_relative(Path::new(path)).unwrap()
    }

    fn filter_with_exclude(exclude: &[&str]) -> Filter {
        let mut pair = Pair::new("p", "/kb", "/repo");
        pair.exclude = exclude.iter().map(|p| (*p).to_owned()).collect();
        Filter::new(&pair, &[".obsidian/**"]).unwrap()
    }

    #[test]
    fn markdown_at_any_depth_is_included() {
        assert!(filter_with_exclude(&[]).accepts(&entry("a/b/c.md")));
    }

    #[test]
    fn markdown_at_root_is_included() {
        assert!(filter_with_exclude(&[]).accepts(&entry("readme.md")));
    }

    #[test]
    fn other_extensions_are_ignored() {
        assert!(!filter_with_exclude(&[]).accepts(&entry("image.png")));
    }

    #[test]
    fn pair_exclusions_apply() {
        assert!(!filter_with_exclude(&["drafts/**"]).accepts(&entry("drafts/idea.md")));
    }

    #[test]
    fn adapter_exclusions_apply() {
        assert!(!filter_with_exclude(&[]).accepts(&entry(".obsidian/notes.md")));
    }

    #[test]
    fn git_folder_is_always_ignored() {
        assert!(!filter_with_exclude(&[]).accepts(&entry("sub/.git/x.md")));
    }

    #[test]
    fn temp_files_are_always_ignored() {
        let mut pair = Pair::new("p", "/kb", "/repo");
        pair.include = vec!["**/*".to_owned()];
        let filter = Filter::new(&pair, &[]).unwrap();
        assert!(!filter.accepts(&entry("a.md.markoo-tmp")));
    }
}
