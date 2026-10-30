//! Scanning one side of a pair into canonical, hashed entries.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::entry::{ContentHash, EntryPath, Side};
use crate::fs::{FileMeta, Fs, FsError};
use crate::kb::{ConvertCtx, Dialect, NameIndex};
use crate::pair::Filter;
use crate::state::BaseEntry;

/// An entry as found on one side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observed {
    /// Metadata at scan time.
    pub meta: FileMeta,
    /// Hash of the canonical content.
    pub hash: ContentHash,
    /// Canonical content, or `None` when the file was not read because its
    /// metadata matches the base (its content is then the base's).
    pub content: Option<String>,
}

/// A file that could not be scanned; it is left out of the sync.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ScanIssue {
    /// File concerned.
    pub path: PathBuf,
    /// What went wrong.
    pub message: String,
}

/// Result of scanning one side.
#[derive(Debug, Clone, Default)]
pub struct SideScan {
    /// Tracked entries found.
    pub files: BTreeMap<EntryPath, Observed>,
    /// Files skipped because they could not be read or converted.
    pub issues: Vec<ScanIssue>,
    /// Entries the planner must leave alone: unreadable files (they exist but
    /// their content is unknown, so they must not look deleted) and entries
    /// the user chose to exclude.
    pub left_alone: BTreeSet<EntryPath>,
}

/// Scans `root` with `dialect`, reusing the base hash of files whose size and
/// modification time did not change since the last sync.
pub fn scan_side(
    fs: &impl Fs,
    root: &Path,
    side: Side,
    filter: &Filter,
    dialect: &dyn Dialect,
    index: &NameIndex,
    bases: &BTreeMap<EntryPath, BaseEntry>,
) -> Result<SideScan, FsError> {
    let mut scan = SideScan::default();
    for relative in fs.list(root)? {
        let Some(path) = EntryPath::from_relative(&relative) else {
            scan.issues.push(ScanIssue {
                path: root.join(&relative),
                message: "file name is not valid UTF-8".to_owned(),
            });
            continue;
        };
        if !filter.accepts(&path) {
            continue;
        }
        let absolute = path.under(root);
        // The file may disappear between listing and reading: skip it, the
        // next sync will see it as deleted.
        let Some(meta) = fs.meta(&absolute)? else {
            continue;
        };
        let base = bases.get(&path);
        let base_meta = base.and_then(|base| match side {
            Side::Kb => base.kb_meta,
            Side::Repo => base.repo_meta,
        });
        if let (Some(base), Some(base_meta)) = (base, base_meta)
            && base_meta == meta
        {
            scan.files.insert(
                path,
                Observed {
                    meta,
                    hash: base.hash.clone(),
                    content: None,
                },
            );
            continue;
        }
        let ctx = ConvertCtx { path: &path, index };
        match read_canonical(fs, &absolute, dialect, &ctx) {
            Ok(Some(content)) => {
                let hash = ContentHash::of(&content);
                scan.files.insert(
                    path,
                    Observed {
                        meta,
                        hash,
                        content: Some(content),
                    },
                );
            }
            Ok(None) => {}
            Err(ReadError::Fs(e)) => return Err(e),
            Err(ReadError::Invalid(message)) => {
                scan.issues.push(ScanIssue {
                    path: absolute,
                    message,
                });
                scan.left_alone.insert(path);
            }
        }
    }
    Ok(scan)
}

/// Failure to read a file as canonical text.
#[derive(Debug)]
pub(crate) enum ReadError {
    /// The file system failed.
    Fs(FsError),
    /// The file is not valid UTF-8 or its dialect could not convert it.
    Invalid(String),
}

/// Reads a file and converts it to canonical form. `Ok(None)` if it is missing.
pub(crate) fn read_canonical(
    fs: &impl Fs,
    path: &Path,
    dialect: &dyn Dialect,
    ctx: &ConvertCtx<'_>,
) -> Result<Option<String>, ReadError> {
    let bytes = match fs.read(path) {
        Ok(bytes) => bytes,
        Err(FsError::NotFound(_)) => return Ok(None),
        Err(e) => return Err(ReadError::Fs(e)),
    };
    let text =
        String::from_utf8(bytes).map_err(|_| ReadError::Invalid("not valid UTF-8".into()))?;
    let canonical = dialect
        .to_canonical(&normalize(&text), ctx)
        .map_err(|e| ReadError::Invalid(e.to_string()))?;
    Ok(Some(canonical))
}

/// Normalizes line endings so that `\r\n` vs `\n` never counts as a change.
pub(crate) fn normalize(text: &str) -> String {
    text.replace("\r\n", "\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::MemFs;
    use crate::kb::MarkdownAdapter;
    use crate::pair::Pair;

    fn scan(fs: &MemFs, bases: &BTreeMap<EntryPath, BaseEntry>) -> SideScan {
        let pair = Pair::new("p", "/kb", "/repo");
        let filter = Filter::new(&pair, &[]).unwrap();
        scan_side(
            fs,
            Path::new("/kb"),
            Side::Kb,
            &filter,
            &MarkdownAdapter,
            &NameIndex::default(),
            bases,
        )
        .unwrap()
    }

    fn entry(path: &str) -> EntryPath {
        EntryPath::from_relative(Path::new(path)).unwrap()
    }

    #[test]
    fn tracked_files_are_read_and_hashed() {
        let fs = MemFs::new();
        fs.write_atomic(Path::new("/kb/a.md"), b"hello").unwrap();
        let scan = scan(&fs, &BTreeMap::new());
        assert_eq!(scan.files[&entry("a.md")].hash, ContentHash::of("hello"));
    }

    #[test]
    fn untracked_files_are_skipped() {
        let fs = MemFs::new();
        fs.write_atomic(Path::new("/kb/a.png"), b"png").unwrap();
        assert!(scan(&fs, &BTreeMap::new()).files.is_empty());
    }

    #[test]
    fn crlf_is_normalized_before_hashing() {
        let fs = MemFs::new();
        fs.write_atomic(Path::new("/kb/a.md"), b"a\r\nb\r\n")
            .unwrap();
        let scan = scan(&fs, &BTreeMap::new());
        assert_eq!(scan.files[&entry("a.md")].hash, ContentHash::of("a\nb\n"));
    }

    #[test]
    fn invalid_utf8_is_reported_not_fatal() {
        let fs = MemFs::new();
        fs.write_atomic(Path::new("/kb/a.md"), &[0xff, 0xfe])
            .unwrap();
        assert_eq!(scan(&fs, &BTreeMap::new()).issues.len(), 1);
    }

    #[test]
    fn file_matching_base_metadata_is_not_read() {
        let fs = MemFs::new();
        fs.write_atomic(Path::new("/kb/a.md"), b"hello").unwrap();
        let meta = fs.meta(Path::new("/kb/a.md")).unwrap();
        let bases = BTreeMap::from([(
            entry("a.md"),
            BaseEntry {
                content: "hello".into(),
                hash: ContentHash::of("hello"),
                kb_meta: meta,
                repo_meta: None,
            },
        )]);
        assert_eq!(scan(&fs, &bases).files[&entry("a.md")].content, None);
    }
}
