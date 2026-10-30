//! Persistent sync state (`<data>/state.db`): the base of every entry and the
//! conflicts and divergences waiting for the user.

use std::collections::BTreeMap;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::entry::{ContentHash, EntryPath};
use crate::fs::FileMeta;

/// Errors raised by the state store.
#[derive(Debug, thiserror::Error)]
pub enum StateError {
    /// SQLite failure.
    #[error("state database: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// The database folder could not be created.
    #[error("cannot create {path}: {source}")]
    CreateDir {
        /// Folder that could not be created.
        path: std::path::PathBuf,
        /// Underlying error.
        source: std::io::Error,
    },
    /// A stored path is not a valid entry path.
    #[error("invalid entry path in state database: {0}")]
    InvalidPath(String),
    /// A stored issue kind is unknown.
    #[error("unknown issue kind in state database: {0}")]
    InvalidKind(String),
}

/// State of an entry at the last successful sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseEntry {
    /// Canonical content shared by both sides.
    pub content: String,
    /// Hash of `content`.
    pub hash: ContentHash,
    /// Metadata of the KB file right after the sync.
    pub kb_meta: Option<FileMeta>,
    /// Metadata of the repo file right after the sync.
    pub repo_meta: Option<FileMeta>,
}

/// Why an entry is waiting for the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IssueKind {
    /// Both sides changed the same lines.
    Conflict,
    /// One side deleted the entry, the other changed it.
    DeleteModify,
    /// The target of a one-way pair was changed.
    Divergence,
}

impl IssueKind {
    fn as_str(self) -> &'static str {
        match self {
            IssueKind::Conflict => "conflict",
            IssueKind::DeleteModify => "delete-modify",
            IssueKind::Divergence => "divergence",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "conflict" => Some(IssueKind::Conflict),
            "delete-modify" => Some(IssueKind::DeleteModify),
            "divergence" => Some(IssueKind::Divergence),
            _ => None,
        }
    }
}

/// An entry waiting for a decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingIssue {
    /// The entry.
    pub path: EntryPath,
    /// Kind of issue.
    pub kind: IssueKind,
    /// Canonical KB content, `None` if deleted.
    pub kb: Option<String>,
    /// Canonical repo content, `None` if deleted.
    pub repo: Option<String>,
}

/// SQLite-backed state store.
pub struct State {
    conn: Connection,
}

impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("State").finish_non_exhaustive()
    }
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS entries (
    pair        TEXT NOT NULL,
    path        TEXT NOT NULL,
    content     TEXT NOT NULL,
    hash        TEXT NOT NULL,
    kb_size     INTEGER,
    kb_mtime    INTEGER,
    repo_size   INTEGER,
    repo_mtime  INTEGER,
    PRIMARY KEY (pair, path)
);
CREATE TABLE IF NOT EXISTS pending (
    pair        TEXT NOT NULL,
    path        TEXT NOT NULL,
    kind        TEXT NOT NULL,
    kb          TEXT,
    repo        TEXT,
    PRIMARY KEY (pair, path)
);
";

impl State {
    /// Opens (or creates) the database at `path`.
    pub fn open(path: &Path) -> Result<Self, StateError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| StateError::CreateDir {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        Self::init(Connection::open(path)?)
    }

    /// Opens a throwaway in-memory database, for tests.
    pub fn open_in_memory() -> Result<Self, StateError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self, StateError> {
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    /// Loads the base of every entry of a pair.
    pub fn bases(&self, pair: &str) -> Result<BTreeMap<EntryPath, BaseEntry>, StateError> {
        let mut stmt = self.conn.prepare(
            "SELECT path, content, hash, kb_size, kb_mtime, repo_size, repo_mtime
             FROM entries WHERE pair = ?1",
        )?;
        let rows = stmt.query_map(params![pair], |row| {
            Ok((
                row.get::<_, String>(0)?,
                BaseEntry {
                    content: row.get(1)?,
                    hash: ContentHash::from_stored(row.get(2)?),
                    kb_meta: meta_from(row.get(3)?, row.get(4)?),
                    repo_meta: meta_from(row.get(5)?, row.get(6)?),
                },
            ))
        })?;
        let mut bases = BTreeMap::new();
        for row in rows {
            let (path, base) = row?;
            bases.insert(parse_path(path)?, base);
        }
        Ok(bases)
    }

    /// Records the base of an entry.
    pub fn put_base(
        &self,
        pair: &str,
        path: &EntryPath,
        base: &BaseEntry,
    ) -> Result<(), StateError> {
        let (kb_size, kb_mtime) = meta_to(base.kb_meta);
        let (repo_size, repo_mtime) = meta_to(base.repo_meta);
        self.conn.execute(
            "INSERT OR REPLACE INTO entries
             (pair, path, content, hash, kb_size, kb_mtime, repo_size, repo_mtime)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                pair,
                path.as_str(),
                base.content,
                base.hash.as_str(),
                kb_size,
                kb_mtime,
                repo_size,
                repo_mtime
            ],
        )?;
        Ok(())
    }

    /// Forgets the base of an entry.
    pub fn forget_base(&self, pair: &str, path: &EntryPath) -> Result<(), StateError> {
        self.conn.execute(
            "DELETE FROM entries WHERE pair = ?1 AND path = ?2",
            params![pair, path.as_str()],
        )?;
        Ok(())
    }

    /// Records (or replaces) an issue waiting for the user.
    pub fn set_pending(&self, pair: &str, issue: &PendingIssue) -> Result<(), StateError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO pending (pair, path, kind, kb, repo)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                pair,
                issue.path.as_str(),
                issue.kind.as_str(),
                issue.kb,
                issue.repo
            ],
        )?;
        Ok(())
    }

    /// Removes the issue of an entry, if any.
    pub fn clear_pending(&self, pair: &str, path: &EntryPath) -> Result<(), StateError> {
        self.conn.execute(
            "DELETE FROM pending WHERE pair = ?1 AND path = ?2",
            params![pair, path.as_str()],
        )?;
        Ok(())
    }

    /// Issues waiting for the user, ordered by path.
    pub fn pending(&self, pair: &str) -> Result<Vec<PendingIssue>, StateError> {
        let mut stmt = self
            .conn
            .prepare("SELECT path, kind, kb, repo FROM pending WHERE pair = ?1 ORDER BY path")?;
        let rows = stmt.query_map(params![pair], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?;
        let mut issues = Vec::new();
        for row in rows {
            let (path, kind, kb, repo) = row?;
            let kind =
                IssueKind::parse(&kind).ok_or_else(|| StateError::InvalidKind(kind.clone()))?;
            issues.push(PendingIssue {
                path: parse_path(path)?,
                kind,
                kb,
                repo,
            });
        }
        Ok(issues)
    }

    /// Whether an entry has a pending issue.
    pub fn has_pending(&self, pair: &str, path: &EntryPath) -> Result<bool, StateError> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM pending WHERE pair = ?1 AND path = ?2",
                params![pair, path.as_str()],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }
}

fn parse_path(path: String) -> Result<EntryPath, StateError> {
    EntryPath::from_relative(Path::new(&path)).ok_or(StateError::InvalidPath(path))
}

fn meta_from(size: Option<i64>, modified: Option<i64>) -> Option<FileMeta> {
    Some(FileMeta {
        size: u64::try_from(size?).ok()?,
        modified: u64::try_from(modified?).ok()?,
    })
}

fn meta_to(meta: Option<FileMeta>) -> (Option<i64>, Option<i64>) {
    match meta {
        Some(meta) => (
            Some(i64::try_from(meta.size).unwrap_or(i64::MAX)),
            Some(i64::try_from(meta.modified).unwrap_or(i64::MAX)),
        ),
        None => (None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str) -> EntryPath {
        EntryPath::from_relative(Path::new(path)).unwrap()
    }

    fn base(content: &str) -> BaseEntry {
        BaseEntry {
            content: content.to_owned(),
            hash: ContentHash::of(content),
            kb_meta: Some(FileMeta {
                size: 3,
                modified: 10,
            }),
            repo_meta: None,
        }
    }

    #[test]
    fn stored_base_is_loaded_back() {
        let state = State::open_in_memory().unwrap();
        state.put_base("p", &entry("a/b.md"), &base("abc")).unwrap();
        let bases = state.bases("p").unwrap();
        assert_eq!(bases.get(&entry("a/b.md")), Some(&base("abc")));
    }

    #[test]
    fn bases_are_scoped_by_pair() {
        let state = State::open_in_memory().unwrap();
        state.put_base("p", &entry("a.md"), &base("abc")).unwrap();
        assert!(state.bases("other").unwrap().is_empty());
    }

    #[test]
    fn forgotten_base_is_gone() {
        let state = State::open_in_memory().unwrap();
        state.put_base("p", &entry("a.md"), &base("abc")).unwrap();
        state.forget_base("p", &entry("a.md")).unwrap();
        assert!(state.bases("p").unwrap().is_empty());
    }

    #[test]
    fn pending_issue_is_listed_until_cleared() {
        let state = State::open_in_memory().unwrap();
        let issue = PendingIssue {
            path: entry("a.md"),
            kind: IssueKind::Conflict,
            kb: Some("k".into()),
            repo: None,
        };
        state.set_pending("p", &issue).unwrap();
        assert_eq!(state.pending("p").unwrap(), vec![issue]);
        state.clear_pending("p", &entry("a.md")).unwrap();
        assert!(!state.has_pending("p", &entry("a.md")).unwrap());
    }

    #[test]
    fn database_on_disk_survives_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/state.db");
        State::open(&path)
            .unwrap()
            .put_base("p", &entry("a.md"), &base("abc"))
            .unwrap();
        assert_eq!(State::open(&path).unwrap().bases("p").unwrap().len(), 1);
    }
}
