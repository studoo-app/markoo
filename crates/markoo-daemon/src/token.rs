//! The API token, stored in `<data>/token`.

use std::io::Write;
use std::path::{Path, PathBuf};

use markoo_core::fs::TMP_SUFFIX;

/// Name of the token file inside `<data>`.
pub const TOKEN_FILE: &str = "token";

/// Errors raised while reading or writing the token.
#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    /// File access failed.
    #[error("cannot access {path}: {source}")]
    Io {
        /// The file.
        path: PathBuf,
        /// Underlying error.
        source: std::io::Error,
    },
    /// The system could not provide randomness.
    #[error("cannot generate a token: {0}")]
    Random(String),
}

/// Path of the token file.
pub fn path(data_dir: &Path) -> PathBuf {
    data_dir.join(TOKEN_FILE)
}

/// Returns the token, creating it the first time.
pub fn ensure(data_dir: &Path) -> Result<String, TokenError> {
    let file = path(data_dir);
    match std::fs::read_to_string(&file) {
        Ok(token) if !token.trim().is_empty() => Ok(token.trim().to_owned()),
        Ok(_) => rotate(data_dir),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => rotate(data_dir),
        Err(source) => Err(TokenError::Io { path: file, source }),
    }
}

/// Replaces the token with a new random one and returns it. A running daemon
/// picks it up at the next request.
pub fn rotate(data_dir: &Path) -> Result<String, TokenError> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| TokenError::Random(e.to_string()))?;
    let token: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    let file = path(data_dir);
    let mut tmp = file.clone().into_os_string();
    tmp.push(TMP_SUFFIX);
    let tmp = PathBuf::from(tmp);
    let io = |path: &Path| {
        let path = path.to_path_buf();
        move |source| TokenError::Io { path, source }
    };
    std::fs::create_dir_all(data_dir).map_err(io(data_dir))?;
    let mut handle = private_file(&tmp).map_err(io(&tmp))?;
    handle.write_all(token.as_bytes()).map_err(io(&tmp))?;
    handle.sync_all().map_err(io(&tmp))?;
    drop(handle);
    std::fs::rename(&tmp, &file).map_err(io(&file))?;
    Ok(token)
}

/// Creates a file only its owner can read. On Windows, the profile folder that
/// holds `<data>` already has this restriction.
fn private_file(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

/// Compares two tokens in a time that does not depend on where they differ.
pub(crate) fn matches(expected: &str, given: &str) -> bool {
    expected.len() == given.len()
        && expected
            .bytes()
            .zip(given.bytes())
            .fold(0, |diff, (a, b)| diff | (a ^ b))
            == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_creates_a_token_then_keeps_it() {
        let dir = tempfile::tempdir().unwrap();
        let first = ensure(dir.path()).unwrap();
        assert_eq!(first, ensure(dir.path()).unwrap());
    }

    #[test]
    fn token_has_256_bits_in_hexadecimal() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(ensure(dir.path()).unwrap().len(), 64);
    }

    #[test]
    fn rotate_changes_the_token() {
        let dir = tempfile::tempdir().unwrap();
        let first = ensure(dir.path()).unwrap();
        assert_ne!(first, rotate(dir.path()).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn only_the_owner_can_read_the_token() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        ensure(dir.path()).unwrap();
        let mode = std::fs::metadata(path(dir.path()))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn matching_needs_the_same_token() {
        assert!(matches("abc", "abc"));
        assert!(!matches("abc", "abd"));
        assert!(!matches("abc", "abcd"));
    }
}
