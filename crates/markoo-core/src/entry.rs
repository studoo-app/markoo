//! Identifiers shared by the scanner, the planner and the state store.

use std::fmt;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Path of an entry relative to the root of a side, always with `/` separators,
/// so the same note has the same key on every operating system.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EntryPath(String);

impl EntryPath {
    /// Builds an entry path from a relative path. Returns `None` for absolute
    /// paths or paths containing `..`, which could escape the pair's folder.
    pub fn from_relative(path: &Path) -> Option<Self> {
        let mut parts = Vec::new();
        for component in path.components() {
            match component {
                Component::Normal(part) => parts.push(part.to_str()?.to_owned()),
                Component::CurDir => {}
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
            }
        }
        if parts.is_empty() {
            return None;
        }
        Some(Self(parts.join("/")))
    }

    /// The path as a `/`-separated string.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Joins this entry to a root folder, using the platform's separator.
    pub fn under(&self, root: &Path) -> PathBuf {
        self.0
            .split('/')
            .fold(root.to_path_buf(), |path, part| path.join(part))
    }
}

impl fmt::Display for EntryPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// BLAKE3 hash of an entry's canonical content, as lowercase hex.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContentHash(String);

impl ContentHash {
    /// Hashes canonical text.
    pub fn of(text: &str) -> Self {
        Self(blake3::hash(text.as_bytes()).to_hex().to_string())
    }

    /// Rebuilds a hash read from storage.
    pub(crate) fn from_stored(hex: String) -> Self {
        Self(hex)
    }

    /// The hash as hex.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One side of a pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Side {
    /// The knowledge base.
    Kb,
    /// The Git repository.
    Repo,
}

impl Side {
    /// The other side.
    pub fn other(self) -> Self {
        match self {
            Side::Kb => Side::Repo,
            Side::Repo => Side::Kb,
        }
    }

    /// Name used in folder names and logs.
    pub fn as_str(self) -> &'static str {
        match self {
            Side::Kb => "kb",
            Side::Repo => "repo",
        }
    }
}

impl fmt::Display for Side {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_path_uses_forward_slashes() {
        let path = EntryPath::from_relative(Path::new("adr").join("001.md").as_path()).unwrap();
        assert_eq!(path.as_str(), "adr/001.md");
    }

    #[test]
    fn entry_path_rejects_parent_components() {
        assert!(EntryPath::from_relative(Path::new("../escape.md")).is_none());
    }

    #[test]
    fn entry_path_rejects_absolute_paths() {
        assert!(EntryPath::from_relative(Path::new("/etc/passwd")).is_none());
    }

    #[test]
    fn entry_path_joins_under_root() {
        let path = EntryPath::from_relative(Path::new("a/b.md")).unwrap();
        assert_eq!(
            path.under(Path::new("/kb")),
            Path::new("/kb").join("a").join("b.md")
        );
    }
}
