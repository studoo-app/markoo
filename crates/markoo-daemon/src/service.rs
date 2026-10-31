//! Running the daemon as a `launchd` service on macOS.

use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::Command;

use markoo_core::config::ConfigError;
use markoo_core::paths::Paths;

/// Label of the service.
pub const LABEL: &str = "dev.markoo.daemon";

/// Errors raised while installing or removing the service.
#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    /// This system has no supported service manager yet.
    #[error("running as a service is only supported on macOS for now; run `markoo daemon`")]
    Unsupported,
    /// The configuration is missing or invalid: the daemon could not start.
    #[error(transparent)]
    Config(#[from] ConfigError),
    /// File access failed.
    #[error("cannot access {path}: {source}")]
    Io {
        /// The file.
        path: PathBuf,
        /// Underlying error.
        source: std::io::Error,
    },
    /// `launchctl` refused.
    #[error("launchctl failed: {0}")]
    Launchctl(String),
}

/// The property list describing the service.
pub fn plist(exe: &Path, paths: &Paths) -> String {
    let log = paths.data_dir().join("daemon.log");
    let arguments = [
        exe,
        Path::new("--config"),
        paths.config_file(),
        Path::new("--data-dir"),
        paths.data_dir(),
        Path::new("daemon"),
    ]
    .map(|argument| format!("    <string>{}</string>\n", xml_escape(argument)))
    .concat();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array>
{arguments}  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <dict>
    <key>SuccessfulExit</key>
    <false/>
  </dict>
  <key>StandardOutPath</key>
  <string>{log}</string>
  <key>StandardErrorPath</key>
  <string>{log}</string>
</dict>
</plist>
"#,
        log = xml_escape(&log),
    )
}

fn xml_escape(path: &Path) -> String {
    path.to_string_lossy()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Refuses a configuration the daemon could not start with: launchd would
/// restart it forever, while the installation looks successful.
#[cfg(any(target_os = "macos", test))]
fn check_config(paths: &Paths) -> Result<(), ServiceError> {
    markoo_core::config::Config::load(paths.config_file())?;
    Ok(())
}

/// Where launchd looks for the services of the current user.
#[cfg(target_os = "macos")]
fn plist_path() -> PathBuf {
    markoo_core::config::expand_home(&format!("~/Library/LaunchAgents/{LABEL}.plist"))
}

#[cfg(target_os = "macos")]
fn launchctl(arguments: &[&str]) -> Result<(), ServiceError> {
    let output = Command::new("launchctl")
        .args(arguments)
        .output()
        .map_err(|e| ServiceError::Launchctl(e.to_string()))?;
    if output.status.success() {
        return Ok(());
    }
    Err(ServiceError::Launchctl(
        String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    ))
}

#[cfg(target_os = "macos")]
fn user_domain() -> Result<String, ServiceError> {
    let output = Command::new("id")
        .arg("-u")
        .output()
        .map_err(|e| ServiceError::Launchctl(e.to_string()))?;
    Ok(format!(
        "gui/{}",
        String::from_utf8_lossy(&output.stdout).trim()
    ))
}

/// Installs and starts the service; returns the path of its property list.
#[cfg(target_os = "macos")]
pub fn install(paths: &Paths) -> Result<PathBuf, ServiceError> {
    check_config(paths)?;
    let io = |path: &Path| {
        let path = path.to_path_buf();
        move |source| ServiceError::Io { path, source }
    };
    let exe = std::env::current_exe().map_err(io(Path::new("markoo")))?;
    let file = plist_path();
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(io(parent))?;
    }
    std::fs::create_dir_all(paths.data_dir()).map_err(io(paths.data_dir()))?;
    let domain = user_domain()?;
    // Not being loaded yet is the normal case.
    let _ = launchctl(&["bootout", &format!("{domain}/{LABEL}")]);
    std::fs::write(&file, plist(&exe, paths)).map_err(io(&file))?;
    launchctl(&["bootstrap", &domain, &file.to_string_lossy()])?;
    Ok(file)
}

/// Stops and removes the service. Its property list is Markoo's own file, not
/// a user's: it can really be deleted.
#[cfg(target_os = "macos")]
pub fn uninstall() -> Result<(), ServiceError> {
    let domain = user_domain()?;
    let _ = launchctl(&["bootout", &format!("{domain}/{LABEL}")]);
    let file = plist_path();
    match std::fs::remove_file(&file) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(ServiceError::Io { path: file, source }),
    }
}

/// Installs and starts the service; returns the path of its property list.
#[cfg(not(target_os = "macos"))]
pub fn install(_paths: &Paths) -> Result<PathBuf, ServiceError> {
    Err(ServiceError::Unsupported)
}

/// Stops and removes the service.
#[cfg(not(target_os = "macos"))]
pub fn uninstall() -> Result<(), ServiceError> {
    Err(ServiceError::Unsupported)
}

#[cfg(test)]
mod tests {
    use super::*;
    use markoo_core::paths::Overrides;

    fn sample() -> String {
        let paths = Paths::resolve(&Overrides {
            config_file: Some(PathBuf::from("/tmp/a&b/markoo.toml")),
            data_dir: Some(PathBuf::from("/tmp/data")),
        })
        .unwrap();
        plist(Path::new("/opt/markoo"), &paths)
    }

    #[test]
    fn plist_keeps_the_locations_in_use() {
        let plist = sample();
        assert!(
            plist.contains(
                "<string>--config</string>\n    <string>/tmp/a&amp;b/markoo.toml</string>"
            )
        );
        assert!(plist.contains("<string>--data-dir</string>\n    <string>/tmp/data</string>"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn plist_is_accepted_by_plutil() {
        use std::io::Write;
        use std::process::Stdio;
        let mut child = Command::new("plutil")
            .args(["-lint", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(sample().as_bytes()).unwrap();
        drop(stdin);
        assert!(child.wait().unwrap().success());
    }

    #[test]
    fn plist_runs_the_daemon_command_last() {
        assert!(sample().contains("<string>daemon</string>\n  </array>"));
    }

    fn paths_with_config(content: Option<&str>) -> (tempfile::TempDir, Paths) {
        let dir = tempfile::tempdir().unwrap();
        let config_file = dir.path().join("markoo.toml");
        if let Some(content) = content {
            std::fs::write(&config_file, content).unwrap();
        }
        let paths = Paths::resolve(&Overrides {
            config_file: Some(config_file),
            data_dir: Some(dir.path().join("data")),
        })
        .unwrap();
        (dir, paths)
    }

    #[test]
    fn missing_configuration_is_refused() {
        let (_dir, paths) = paths_with_config(None);
        assert!(matches!(
            check_config(&paths),
            Err(ServiceError::Config(ConfigError::Missing(_)))
        ));
    }

    #[test]
    fn invalid_configuration_is_refused() {
        let (_dir, paths) = paths_with_config(Some("not toml ="));
        assert!(matches!(
            check_config(&paths),
            Err(ServiceError::Config(ConfigError::Invalid(_)))
        ));
    }

    #[test]
    fn configuration_without_pairs_is_accepted() {
        let (_dir, paths) = paths_with_config(Some(""));
        assert!(check_config(&paths).is_ok());
    }

    #[test]
    fn plist_restarts_only_after_a_failure() {
        assert!(sample().contains("<key>SuccessfulExit</key>\n    <false/>"));
    }
}
