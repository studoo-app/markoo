//! Changes to the configured pairs, shared by every interface.

use std::path::Path;

use crate::config::{Config, ConfigError, ConfigFile, NewPair, Problem, expand_home};
use crate::engine::Engine;
use crate::fs::{DiskFs, Fs};
use crate::kb::KbType;
use crate::paths::Paths;
use crate::{Direction, Error, Pair};

/// The configuration file with a new pair, not saved yet.
fn with_new_pair(paths: &Paths, new: &NewPair) -> Result<(ConfigFile, String), Error> {
    let mut new = new.clone();
    if new.kb_type.is_none() {
        // Written down so that the behavior does not change afterwards.
        new.kb_type = Some(KbType::detect(&expand_home(&new.kb)).id().to_owned());
    }
    let config_file = paths.config_file();
    if DiskFs.meta(config_file)?.is_none() {
        ConfigFile::create(config_file)?;
    }
    let mut file = ConfigFile::open(config_file)?;
    file.add_pair(&new)?;
    for (key, value) in &new.settings {
        file.set(&new.name, key, value)?;
    }
    Ok((file, new.name))
}

/// Adds a pair and returns the warnings about it. Its folders must already be
/// absolute (or start with `~`); only the caller knows what a relative one means.
pub fn add_pair(paths: &Paths, new: &NewPair) -> Result<Vec<Problem>, Error> {
    let (file, name) = with_new_pair(paths, new)?;
    Ok(save_if_valid(&file, &name)?)
}

/// The pair `add_pair` would create, checked but not saved: a daemon syncs a
/// pair as soon as it appears in the file, so the preview must come first.
pub fn preview_new_pair(paths: &Paths, new: &NewPair) -> Result<Pair, Error> {
    let (file, name) = with_new_pair(paths, new)?;
    Ok(checked_pair(&file, &name)?)
}

/// Changes settings of a pair, all at once, and returns the warnings about it.
/// Leaving `both` for a one-way direction is refused while issues are pending.
pub fn set_pair(
    paths: &Paths,
    name: &str,
    changes: &[(String, String)],
) -> Result<Vec<Problem>, Error> {
    let file = with_changes(paths, name, changes)?;
    Ok(save_if_valid(&file, name)?)
}

/// The pair `set_pair` would leave, checked but not saved.
pub fn preview_set_pair(
    paths: &Paths,
    name: &str,
    changes: &[(String, String)],
) -> Result<Pair, Error> {
    let file = with_changes(paths, name, changes)?;
    Ok(checked_pair(&file, name)?)
}

/// The configuration file with changes to a pair, not saved yet.
fn with_changes(
    paths: &Paths,
    name: &str,
    changes: &[(String, String)],
) -> Result<ConfigFile, Error> {
    let config = Config::load(paths.config_file())?;
    let current = &config.pair(name)?.pair;
    let mut file = ConfigFile::open(paths.config_file())?;
    for (key, value) in changes {
        file.set(name, key, value)?;
    }
    let preview = file.preview()?;
    let updated = &preview.pair(name)?.pair;
    if current.direction == Direction::Both && updated.direction != Direction::Both {
        let pending = Engine::open(DiskFs, paths)?.pending_count(current)?;
        if pending > 0 {
            return Err(Error::DirectionBlocked {
                pair: name.to_owned(),
                pending,
            });
        }
    }
    Ok(file)
}

/// Changes settings that apply to every pair, all at once.
pub fn set_global(paths: &Paths, changes: &[(String, String)]) -> Result<(), Error> {
    let mut file = ConfigFile::open(paths.config_file())?;
    for (key, value) in changes {
        file.set_global(key, value)?;
    }
    file.save()?;
    Ok(())
}

/// Removes a pair and forgets its state. Its files are not touched.
pub fn remove_pair(paths: &Paths, name: &str) -> Result<(), Error> {
    let mut file = ConfigFile::open(paths.config_file())?;
    file.remove_pair(name)?;
    let engine = Engine::open(DiskFs, paths)?;
    // A sync finishing after this point would write the state back.
    let _lock = engine.lock_pair(name)?;
    file.save()?;
    engine.forget_pair(name)
}

/// Whether this pair of the edited file passes validation; returns its warnings.
fn validated(config: &Config, name: &str) -> Result<Vec<Problem>, ConfigError> {
    let validation = config.validate();
    let errors: Vec<String> = validation
        .errors
        .into_iter()
        .filter(|problem| problem.pair == name)
        .map(|problem| problem.message)
        .collect();
    if !errors.is_empty() {
        return Err(ConfigError::InvalidPair {
            pair: name.to_owned(),
            message: errors.join("; "),
        });
    }
    Ok(validation
        .warnings
        .into_iter()
        .filter(|problem| problem.pair == name)
        .collect())
}

/// Saves only if the file parses and this pair passes validation.
fn save_if_valid(file: &ConfigFile, name: &str) -> Result<Vec<Problem>, ConfigError> {
    let warnings = validated(&file.preview()?, name)?;
    file.save()?;
    Ok(warnings)
}

/// A pair of the edited file, if it passes validation.
fn checked_pair(file: &ConfigFile, name: &str) -> Result<Pair, ConfigError> {
    let config = file.preview()?;
    validated(&config, name)?;
    Ok(config.pair(name)?.pair.clone())
}

/// Whether a folder given through a remote interface can be written to the
/// configuration: absolute or under `~`, with no `..` to hide where it leads.
pub fn is_safe_folder(path: &str) -> bool {
    let expanded = expand_home(path);
    expanded.is_absolute()
        && !Path::new(&expanded)
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_with_parent_component_is_refused() {
        assert!(!is_safe_folder("/notes/../etc"));
    }

    #[test]
    fn relative_folder_is_refused() {
        assert!(!is_safe_folder("notes"));
    }

    #[test]
    fn absolute_folder_is_accepted() {
        let root = std::env::temp_dir();
        assert!(is_safe_folder(root.to_str().unwrap()));
    }
}
