//! Resolution of Markoo's configuration file and data directory.
//!
//! This module is the only place that computes these locations. Both the CLI
//! and the desktop app go through it so they always share the same state.
//!
//! Priority, highest first:
//! 1. explicit overrides (`--config`, `--data-dir`);
//! 2. environment variables (`MARKOO_CONFIG`, `MARKOO_DATA_DIR`);
//! 3. the operating system's standard folders.

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;

/// Environment variable overriding the configuration file path.
pub const CONFIG_ENV: &str = "MARKOO_CONFIG";
/// Environment variable overriding the data directory.
pub const DATA_DIR_ENV: &str = "MARKOO_DATA_DIR";
/// File name of the configuration inside the default config directory.
pub const CONFIG_FILE_NAME: &str = "markoo.toml";

const QUALIFIER: &str = "dev";
const ORGANIZATION: &str = "markoo";
const APPLICATION: &str = "markoo";

/// Errors raised while resolving paths.
#[derive(Debug, thiserror::Error)]
pub enum PathsError {
    /// No home directory could be determined, so there is no default location.
    #[error(
        "cannot determine the standard folders for this system; set {CONFIG_ENV} and {DATA_DIR_ENV}"
    )]
    NoHomeDirectory,
    /// An override was set to an empty value.
    #[error("{0} is set but empty")]
    EmptyOverride(&'static str),
}

/// Where a resolved path came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Passed explicitly, e.g. as a command-line argument.
    Override,
    /// Read from an environment variable.
    Environment,
    /// The operating system's standard folder.
    Default,
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Source::Override => "argument",
            Source::Environment => "environment",
            Source::Default => "default",
        })
    }
}

/// Explicit locations that take precedence over everything else.
#[derive(Debug, Clone, Default)]
pub struct Overrides {
    /// Path to the configuration file.
    pub config_file: Option<PathBuf>,
    /// Path to the data directory.
    pub data_dir: Option<PathBuf>,
}

/// Resolved locations used by Markoo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    config_file: PathBuf,
    config_source: Source,
    data_dir: PathBuf,
    data_source: Source,
}

impl Paths {
    /// Resolves paths from the overrides, the process environment and the OS defaults.
    pub fn resolve(overrides: &Overrides) -> Result<Self, PathsError> {
        Self::resolve_with(overrides, |key| std::env::var_os(key), default_dirs)
    }

    /// Resolves paths with injectable environment and defaults, for tests.
    fn resolve_with(
        overrides: &Overrides,
        env: impl Fn(&str) -> Option<OsString>,
        defaults: impl FnOnce() -> Option<(PathBuf, PathBuf)>,
    ) -> Result<Self, PathsError> {
        let config = pick(
            overrides.config_file.as_deref(),
            "--config",
            CONFIG_ENV,
            &env,
        )?;
        let data = pick(
            overrides.data_dir.as_deref(),
            "--data-dir",
            DATA_DIR_ENV,
            &env,
        )?;

        let defaults = match (&config, &data) {
            (Some(_), Some(_)) => None,
            _ => Some(defaults().ok_or(PathsError::NoHomeDirectory)?),
        };
        let (config_file, config_source) = match (config, &defaults) {
            (Some(found), _) => found,
            (None, Some((config_dir, _))) => (config_dir.join(CONFIG_FILE_NAME), Source::Default),
            (None, None) => unreachable!("defaults are computed whenever an override is missing"),
        };
        let (data_dir, data_source) = match (data, defaults) {
            (Some(found), _) => found,
            (None, Some((_, data_dir))) => (data_dir, Source::Default),
            (None, None) => unreachable!("defaults are computed whenever an override is missing"),
        };

        Ok(Self {
            config_file,
            config_source,
            data_dir,
            data_source,
        })
    }

    /// Path to `markoo.toml`.
    pub fn config_file(&self) -> &Path {
        &self.config_file
    }

    /// Where the configuration file path came from.
    pub fn config_source(&self) -> Source {
        self.config_source
    }

    /// Directory holding the state database, journal, backups, trash and API token.
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Where the data directory came from.
    pub fn data_source(&self) -> Source {
        self.data_source
    }
}

/// Returns the override or environment value, if any, with its source.
fn pick(
    explicit: Option<&Path>,
    flag: &'static str,
    env_key: &'static str,
    env: &impl Fn(&str) -> Option<OsString>,
) -> Result<Option<(PathBuf, Source)>, PathsError> {
    if let Some(path) = explicit {
        if path.as_os_str().is_empty() {
            return Err(PathsError::EmptyOverride(flag));
        }
        return Ok(Some((path.to_path_buf(), Source::Override)));
    }
    match env(env_key) {
        Some(value) if value.is_empty() => Err(PathsError::EmptyOverride(env_key)),
        Some(value) => Ok(Some((PathBuf::from(value), Source::Environment))),
        None => Ok(None),
    }
}

/// Standard config and data directories for this OS.
///
/// - Linux: `~/.config/markoo`, `~/.local/share/markoo`
/// - macOS: `~/Library/Application Support/dev.markoo.markoo` for both
/// - Windows: `%APPDATA%\markoo\markoo\config`, `%LOCALAPPDATA%\markoo\markoo\data`
fn default_dirs() -> Option<(PathBuf, PathBuf)> {
    let dirs = ProjectDirs::from(QUALIFIER, ORGANIZATION, APPLICATION)?;
    Some((
        dirs.config_dir().to_path_buf(),
        dirs.data_local_dir().to_path_buf(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env_from(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let map: HashMap<String, OsString> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), OsString::from(v)))
            .collect();
        move |key| map.get(key).cloned()
    }

    fn defaults() -> Option<(PathBuf, PathBuf)> {
        Some((PathBuf::from("/os/config"), PathBuf::from("/os/data")))
    }

    #[test]
    fn uses_os_defaults_when_nothing_is_set() {
        let paths = Paths::resolve_with(&Overrides::default(), env_from(&[]), defaults).unwrap();
        assert_eq!(paths.config_file(), Path::new("/os/config/markoo.toml"));
        assert_eq!(paths.config_source(), Source::Default);
        assert_eq!(paths.data_dir(), Path::new("/os/data"));
        assert_eq!(paths.data_source(), Source::Default);
    }

    #[test]
    fn environment_beats_defaults() {
        let env = env_from(&[(CONFIG_ENV, "/env/m.toml"), (DATA_DIR_ENV, "/env/data")]);
        let paths = Paths::resolve_with(&Overrides::default(), env, defaults).unwrap();
        assert_eq!(paths.config_file(), Path::new("/env/m.toml"));
        assert_eq!(paths.config_source(), Source::Environment);
        assert_eq!(paths.data_dir(), Path::new("/env/data"));
        assert_eq!(paths.data_source(), Source::Environment);
    }

    #[test]
    fn overrides_beat_environment() {
        let overrides = Overrides {
            config_file: Some("/arg/m.toml".into()),
            data_dir: Some("/arg/data".into()),
        };
        let env = env_from(&[(CONFIG_ENV, "/env/m.toml"), (DATA_DIR_ENV, "/env/data")]);
        let paths = Paths::resolve_with(&overrides, env, defaults).unwrap();
        assert_eq!(paths.config_file(), Path::new("/arg/m.toml"));
        assert_eq!(paths.config_source(), Source::Override);
        assert_eq!(paths.data_dir(), Path::new("/arg/data"));
        assert_eq!(paths.data_source(), Source::Override);
    }

    #[test]
    fn sources_are_resolved_independently() {
        let env = env_from(&[(DATA_DIR_ENV, "/env/data")]);
        let paths = Paths::resolve_with(&Overrides::default(), env, defaults).unwrap();
        assert_eq!(paths.config_source(), Source::Default);
        assert_eq!(paths.data_source(), Source::Environment);
    }

    #[test]
    fn no_home_is_fine_when_everything_is_overridden() {
        let env = env_from(&[(CONFIG_ENV, "/env/m.toml"), (DATA_DIR_ENV, "/env/data")]);
        let paths = Paths::resolve_with(&Overrides::default(), env, || None).unwrap();
        assert_eq!(paths.data_dir(), Path::new("/env/data"));
    }

    #[test]
    fn no_home_without_overrides_is_an_error() {
        let err = Paths::resolve_with(&Overrides::default(), env_from(&[]), || None).unwrap_err();
        assert!(matches!(err, PathsError::NoHomeDirectory));
    }

    #[test]
    fn empty_values_are_rejected() {
        let err = Paths::resolve_with(
            &Overrides::default(),
            env_from(&[(DATA_DIR_ENV, "")]),
            defaults,
        )
        .unwrap_err();
        assert!(matches!(err, PathsError::EmptyOverride(DATA_DIR_ENV)));

        let overrides = Overrides {
            config_file: Some(PathBuf::new()),
            data_dir: None,
        };
        let err = Paths::resolve_with(&overrides, env_from(&[]), defaults).unwrap_err();
        assert!(matches!(err, PathsError::EmptyOverride("--config")));
    }

    #[test]
    fn os_defaults_follow_platform_conventions() {
        let Some((config, data)) = default_dirs() else {
            return; // no home directory in this environment
        };
        let config = config.to_string_lossy().replace('\\', "/");
        let data = data.to_string_lossy().replace('\\', "/");
        if cfg!(target_os = "macos") {
            assert!(config.ends_with("Library/Application Support/dev.markoo.markoo"));
            assert_eq!(config, data);
        } else if cfg!(windows) {
            assert!(config.ends_with("markoo/markoo/config"), "{config}");
            assert!(data.ends_with("markoo/markoo/data"), "{data}");
        } else {
            assert!(config.ends_with("markoo"), "{config}");
            assert!(data.ends_with("markoo"), "{data}");
        }
    }
}
