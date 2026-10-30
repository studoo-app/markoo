//! File access used by the engine.
//!
//! The engine never touches `std::fs` directly: it goes through [`Fs`], so the
//! whole sync can run against [`MemFs`] in tests and [`DiskFs`] in production.
//!
//! There is deliberately no `remove` operation. Markoo never deletes a user
//! file: a deletion is a [`Fs::rename`] into the trash folder.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

/// Suffix of the temporary file used for atomic writes.
pub const TMP_SUFFIX: &str = ".markoo-tmp";

/// Errors raised by a [`Fs`] implementation.
#[derive(Debug, thiserror::Error)]
pub enum FsError {
    /// The file does not exist.
    #[error("file not found: {0}")]
    NotFound(PathBuf),
    /// Any other I/O failure.
    #[error("I/O error on {path}: {source}")]
    Io {
        /// File the operation was about.
        path: PathBuf,
        /// Underlying error.
        source: io::Error,
    },
    /// Walking a directory failed.
    #[error("cannot list {path}: {message}")]
    Walk {
        /// Directory being listed.
        path: PathBuf,
        /// Description of the failure.
        message: String,
    },
}

/// Size and modification time of a file, used to skip unchanged files without reading them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileMeta {
    /// Size in bytes.
    pub size: u64,
    /// Modification time, in milliseconds since the Unix epoch (a logical clock for [`MemFs`]).
    pub modified: u64,
}

/// Minimal file system interface needed by the engine.
pub trait Fs {
    /// Lists every file under `root`, recursively, as paths relative to `root`.
    /// A missing `root` is an empty list.
    fn list(&self, root: &Path) -> Result<Vec<PathBuf>, FsError>;
    /// Whether `path` is an existing folder.
    fn is_dir(&self, path: &Path) -> bool;
    /// Whether `path` is a folder that really exists. Unlike [`Fs::is_dir`], this
    /// is used to recognize a tool's settings folder.
    fn has_folder(&self, path: &Path) -> bool {
        self.is_dir(path)
    }
    /// Reads a whole file.
    fn read(&self, path: &Path) -> Result<Vec<u8>, FsError>;
    /// Returns the file's metadata, or `None` if it does not exist.
    fn meta(&self, path: &Path) -> Result<Option<FileMeta>, FsError>;
    /// Writes a file atomically (temporary file, sync, rename), creating parent folders.
    fn write_atomic(&self, path: &Path, bytes: &[u8]) -> Result<(), FsError>;
    /// Moves a file, creating the destination's parent folders.
    fn rename(&self, from: &Path, to: &Path) -> Result<(), FsError>;
}

/// The real file system.
#[derive(Debug, Default, Clone, Copy)]
pub struct DiskFs;

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> FsError + '_ {
    move |source| {
        if source.kind() == io::ErrorKind::NotFound {
            FsError::NotFound(path.to_path_buf())
        } else {
            FsError::Io {
                path: path.to_path_buf(),
                source,
            }
        }
    }
}

fn create_parent(path: &Path) -> Result<(), FsError> {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => {
            std::fs::create_dir_all(parent).map_err(io_err(parent))
        }
        _ => Ok(()),
    }
}

impl Fs for DiskFs {
    fn list(&self, root: &Path) -> Result<Vec<PathBuf>, FsError> {
        if !root.exists() {
            return Ok(Vec::new());
        }
        let mut files = Vec::new();
        for entry in walkdir::WalkDir::new(root).follow_links(false) {
            let entry = entry.map_err(|e| FsError::Walk {
                path: root.to_path_buf(),
                message: e.to_string(),
            })?;
            if !entry.file_type().is_file() {
                continue;
            }
            // `strip_prefix` cannot fail: walkdir only yields paths under `root`.
            if let Ok(relative) = entry.path().strip_prefix(root) {
                files.push(relative.to_path_buf());
            }
        }
        files.sort();
        Ok(files)
    }

    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }

    fn read(&self, path: &Path) -> Result<Vec<u8>, FsError> {
        std::fs::read(path).map_err(io_err(path))
    }

    fn meta(&self, path: &Path) -> Result<Option<FileMeta>, FsError> {
        match std::fs::metadata(path) {
            Ok(meta) => {
                let modified = meta
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                    .map_or(0, |elapsed| {
                        u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
                    });
                Ok(Some(FileMeta {
                    size: meta.len(),
                    modified,
                }))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io_err(path)(e)),
        }
    }

    fn write_atomic(&self, path: &Path, bytes: &[u8]) -> Result<(), FsError> {
        create_parent(path)?;
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(TMP_SUFFIX);
        let tmp = PathBuf::from(tmp);
        let mut file = std::fs::File::create(&tmp).map_err(io_err(&tmp))?;
        file.write_all(bytes).map_err(io_err(&tmp))?;
        file.sync_all().map_err(io_err(&tmp))?;
        drop(file);
        std::fs::rename(&tmp, path).map_err(io_err(path))
    }

    fn rename(&self, from: &Path, to: &Path) -> Result<(), FsError> {
        create_parent(to)?;
        match std::fs::rename(from, to) {
            Ok(()) => Ok(()),
            // The trash may live on another volume than the KB or the repo:
            // copy it safely first, and only then drop the original.
            Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {
                let bytes = self.read(from)?;
                self.write_atomic(to, &bytes)?;
                std::fs::remove_file(from).map_err(io_err(from))
            }
            Err(e) => Err(io_err(from)(e)),
        }
    }
}

/// An in-memory file system for tests, with a logical clock as modification time.
#[derive(Debug, Default)]
pub struct MemFs {
    inner: Mutex<MemInner>,
}

#[derive(Debug, Default)]
struct MemInner {
    files: BTreeMap<PathBuf, (Vec<u8>, u64)>,
    clock: u64,
    writes: u64,
    offline: BTreeSet<PathBuf>,
}

impl MemFs {
    /// Creates an empty file system.
    pub fn new() -> Self {
        Self::default()
    }

    /// Simulates a folder becoming unavailable (unplugged disk, unmounted volume):
    /// its files vanish from listings and [`Fs::is_dir`] returns `false`.
    pub fn set_offline(&self, root: impl AsRef<Path>) {
        let root = root.as_ref().to_path_buf();
        let mut inner = self.lock();
        inner.files.retain(|path, _| !path.starts_with(&root));
        inner.offline.insert(root);
    }

    /// Number of writes and renames performed so far.
    pub fn write_count(&self) -> u64 {
        self.lock().writes
    }

    /// Returns a file's content as text, if it exists and is valid UTF-8.
    pub fn read_string(&self, path: impl AsRef<Path>) -> Option<String> {
        let inner = self.lock();
        let (bytes, _) = inner.files.get(path.as_ref())?;
        String::from_utf8(bytes.clone()).ok()
    }

    /// Whether a file exists.
    pub fn exists(&self, path: impl AsRef<Path>) -> bool {
        self.lock().files.contains_key(path.as_ref())
    }

    /// Lists every file whose path starts with `prefix`.
    pub fn files_under(&self, prefix: impl AsRef<Path>) -> Vec<PathBuf> {
        let prefix = prefix.as_ref();
        self.lock()
            .files
            .keys()
            .filter(|path| path.starts_with(prefix))
            .cloned()
            .collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, MemInner> {
        // A poisoned lock only means another test thread panicked mid-write;
        // the map itself is still consistent.
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Fs for MemFs {
    fn list(&self, root: &Path) -> Result<Vec<PathBuf>, FsError> {
        Ok(self
            .lock()
            .files
            .keys()
            .filter_map(|path| path.strip_prefix(root).ok())
            .filter(|relative| !relative.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .collect())
    }

    fn is_dir(&self, path: &Path) -> bool {
        !self.lock().offline.contains(path)
    }

    fn has_folder(&self, path: &Path) -> bool {
        self.lock().files.keys().any(|file| file.starts_with(path))
    }

    fn read(&self, path: &Path) -> Result<Vec<u8>, FsError> {
        self.lock()
            .files
            .get(path)
            .map(|(bytes, _)| bytes.clone())
            .ok_or_else(|| FsError::NotFound(path.to_path_buf()))
    }

    fn meta(&self, path: &Path) -> Result<Option<FileMeta>, FsError> {
        Ok(self
            .lock()
            .files
            .get(path)
            .map(|(bytes, modified)| FileMeta {
                size: bytes.len() as u64,
                modified: *modified,
            }))
    }

    fn write_atomic(&self, path: &Path, bytes: &[u8]) -> Result<(), FsError> {
        let mut inner = self.lock();
        inner.clock += 1;
        inner.writes += 1;
        let clock = inner.clock;
        inner
            .files
            .insert(path.to_path_buf(), (bytes.to_vec(), clock));
        Ok(())
    }

    fn rename(&self, from: &Path, to: &Path) -> Result<(), FsError> {
        let mut inner = self.lock();
        let (bytes, _) = inner
            .files
            .remove(from)
            .ok_or_else(|| FsError::NotFound(from.to_path_buf()))?;
        inner.clock += 1;
        inner.writes += 1;
        let clock = inner.clock;
        inner.files.insert(to.to_path_buf(), (bytes, clock));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mem_list_returns_paths_relative_to_root() {
        let fs = MemFs::new();
        fs.write_atomic(Path::new("/kb/a.md"), b"a").unwrap();
        fs.write_atomic(Path::new("/kb/sub/b.md"), b"b").unwrap();
        fs.write_atomic(Path::new("/repo/c.md"), b"c").unwrap();
        let listed = fs.list(Path::new("/kb")).unwrap();
        assert_eq!(
            listed,
            vec![PathBuf::from("a.md"), PathBuf::from("sub/b.md")]
        );
    }

    #[test]
    fn mem_write_advances_the_clock() {
        let fs = MemFs::new();
        let path = Path::new("/kb/a.md");
        fs.write_atomic(path, b"a").unwrap();
        let first = fs.meta(path).unwrap().unwrap().modified;
        fs.write_atomic(path, b"a").unwrap();
        assert!(fs.meta(path).unwrap().unwrap().modified > first);
    }

    #[test]
    fn disk_write_atomic_creates_parents_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/note.md");
        DiskFs.write_atomic(&path, b"hello").unwrap();
        let listed = DiskFs.list(dir.path()).unwrap();
        assert_eq!(listed, vec![PathBuf::from("sub/note.md")]);
    }

    #[test]
    fn disk_rename_moves_into_new_folder() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("a.md");
        let to = dir.path().join("trash/x/a.md");
        DiskFs.write_atomic(&from, b"a").unwrap();
        DiskFs.rename(&from, &to).unwrap();
        assert_eq!(DiskFs.read(&to).unwrap(), b"a");
    }

    #[test]
    fn disk_meta_of_missing_file_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(DiskFs.meta(&dir.path().join("nope.md")).unwrap().is_none());
    }

    #[test]
    fn disk_list_of_missing_root_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(DiskFs.list(&dir.path().join("nope")).unwrap().is_empty());
    }
}
