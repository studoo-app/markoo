//! The desktop app. Commands only translate between the window and
//! `markoo-core`; the sync logic is not here.

#[cfg(target_os = "macos")]
mod tray_macos;

use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use markoo_core::config::{ConfigError, ConfigFile, NewPair};
use markoo_core::fs::{DiskFs, Fs};
use markoo_core::paths::{Overrides, Paths};
use markoo_core::service::{self, SyncOutcome};
use markoo_core::{manage, merge};
use markoo_daemon::{Daemon, Error as DaemonError, Options, Queue, Trigger};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tauri::image::Image;
use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, RunEvent, State, WindowEvent};

const TRAY: &str = "main";
const WINDOW: &str = "main";
/// Tells the window that it should read everything again.
const CHANGED: &str = "markoo-changed";
/// How often the menu bar looks at the state, which a CLI or another daemon
/// may change without any event reaching the app.
const REFRESH: Duration = Duration::from_secs(5);

/// What the app runs on.
struct Backend {
    paths: Paths,
    /// The daemon embedded in the app, if it could start.
    daemon: Mutex<Option<Daemon>>,
    queue: Option<Queue>,
    /// Why the app does not sync on its own, if it does not.
    note: Option<String>,
}

type CommandResult<T = Value> = Result<T, String>;

/// Runs engine work outside the async runtime.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, markoo_core::Error> + Send + 'static,
) -> CommandResult<T> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn app_info(backend: State<'_, Backend>) -> CommandResult {
    Ok(json!({
        "config_file": backend.paths.config_file(),
        "data_dir": backend.paths.data_dir(),
        "embedded_daemon": backend.queue.is_some(),
        "note": backend.note,
    }))
}

#[tauri::command]
async fn list_pairs(backend: State<'_, Backend>) -> CommandResult {
    let paths = backend.paths.clone();
    let active = backend.queue.as_ref().map(Queue::active_pairs);
    let mut views = blocking(move || service::list_pairs(&paths)).await?;
    for view in &mut views {
        let name = view["name"].clone();
        let running = active
            .as_ref()
            .is_some_and(|active| active.iter().any(|pair| name == pair.as_str()));
        view["active"] = json!(running);
    }
    Ok(Value::Array(views))
}

#[tauri::command]
async fn pair_status(backend: State<'_, Backend>, pair: String) -> CommandResult {
    let paths = backend.paths.clone();
    blocking(move || service::status(&paths, &pair)).await
}

#[tauri::command]
async fn pair_diff(backend: State<'_, Backend>, pair: String, path: String) -> CommandResult {
    let paths = backend.paths.clone();
    blocking(move || service::diff(&paths, &pair, Some(&path))).await
}

#[derive(Debug, Deserialize)]
struct NewPairArgs {
    name: String,
    kb: String,
    kb_type: Option<String>,
    repo: String,
    direction: Option<String>,
    target_changes: Option<String>,
    #[serde(default)]
    settings: Vec<(String, String)>,
}

impl From<NewPairArgs> for NewPair {
    fn from(args: NewPairArgs) -> Self {
        Self {
            name: args.name,
            kb: args.kb,
            kb_type: args.kb_type,
            repo: args.repo,
            direction: args.direction,
            target_changes: args.target_changes,
            settings: args.settings,
        }
    }
}

#[tauri::command]
async fn preview_new_pair(backend: State<'_, Backend>, new: NewPairArgs) -> CommandResult {
    let paths = backend.paths.clone();
    blocking(move || service::preview_new_pair(&paths, &new.into())).await
}

#[tauri::command]
async fn add_pair(backend: State<'_, Backend>, new: NewPairArgs) -> CommandResult {
    let paths = backend.paths.clone();
    blocking(move || {
        let new = NewPair::from(new);
        service::check_folder("kb", &new.kb)?;
        service::check_folder("repo", &new.repo)?;
        let warnings = {
            let _edit = service::config_edit();
            manage::add_pair(&paths, &new)?
        };
        Ok(json!(service::warnings_view(&warnings)))
    })
    .await
}

#[tauri::command]
async fn preview_pair_settings(
    backend: State<'_, Backend>,
    pair: String,
    settings: Map<String, Value>,
) -> CommandResult {
    let paths = backend.paths.clone();
    blocking(move || service::preview_config(&paths, &pair, &settings)).await
}

#[tauri::command]
async fn update_pair_settings(
    backend: State<'_, Backend>,
    pair: String,
    settings: Map<String, Value>,
) -> CommandResult {
    let paths = backend.paths.clone();
    blocking(move || service::update_config(&paths, &pair, &settings)).await
}

#[tauri::command]
async fn remove_pair(backend: State<'_, Backend>, pair: String) -> CommandResult<()> {
    let paths = backend.paths.clone();
    blocking(move || {
        let _edit = service::config_edit();
        manage::remove_pair(&paths, &pair)
    })
    .await
}

#[tauri::command]
async fn list_conflicts(backend: State<'_, Backend>, pair: Option<String>) -> CommandResult {
    let paths = backend.paths.clone();
    blocking(move || service::conflicts(&paths, pair.as_deref())).await
}

#[tauri::command]
async fn conflict_draft(backend: State<'_, Backend>, id: String) -> CommandResult<String> {
    let paths = backend.paths.clone();
    blocking(move || service::conflict_draft(&paths, &id)).await
}

#[tauri::command]
async fn resolve_conflict(
    backend: State<'_, Backend>,
    id: String,
    keep: Option<String>,
    content: Option<String>,
    push_to_source: bool,
) -> CommandResult {
    if let Some(content) = &content {
        if merge::has_markers(content) {
            return Err("conflict markers are still there; nothing was changed".to_owned());
        }
        if content.trim().is_empty() {
            return Err("the result is empty; nothing was changed".to_owned());
        }
    }
    let paths = backend.paths.clone();
    blocking(move || {
        let resolution = service::resolution(keep.as_deref(), content, push_to_source)?;
        service::resolve(&paths, &id, resolution)
    })
    .await
}

async fn sync_one(backend: &Backend, pair: String) -> CommandResult<SyncOutcome> {
    if let Some(queue) = &backend.queue {
        let id = queue.submit(&pair, false, Trigger::App);
        return queue.finished(&id).await;
    }
    // Without an embedded daemon, the lock of the pair answers if another
    // process is syncing it.
    let paths = backend.paths.clone();
    blocking(move || service::run_sync(&paths, &pair, false)).await
}

async fn sync_everything(backend: &Backend) -> CommandResult {
    let paths = backend.paths.clone();
    let names = blocking(move || service::runnable_pairs(&paths)).await?;
    let mut results = Vec::with_capacity(names.len());
    for pair in names {
        results.push(match sync_one(backend, pair.clone()).await {
            Ok(outcome) => json!({"pair": pair, "outcome": outcome}),
            Err(error) => json!({"pair": pair, "error": error}),
        });
    }
    Ok(Value::Array(results))
}

#[tauri::command]
async fn sync_pair(backend: State<'_, Backend>, pair: String) -> CommandResult {
    let outcome = sync_one(&backend, pair).await?;
    Ok(json!(outcome))
}

#[tauri::command]
async fn preview_filters(
    backend: State<'_, Backend>,
    pair: String,
    settings: Map<String, Value>,
) -> CommandResult {
    let paths = backend.paths.clone();
    blocking(move || service::preview_filters(&paths, &pair, &settings)).await
}

#[tauri::command]
async fn preview_new_pair_filters(backend: State<'_, Backend>, new: NewPairArgs) -> CommandResult {
    let paths = backend.paths.clone();
    blocking(move || service::preview_new_pair_filters(&paths, &new.into())).await
}

#[tauri::command]
async fn preview_transform(
    backend: State<'_, Backend>,
    pair: String,
    settings: Map<String, Value>,
) -> CommandResult {
    let paths = backend.paths.clone();
    blocking(move || service::preview_transform(&paths, &pair, &settings)).await
}

#[tauri::command]
async fn global_settings(backend: State<'_, Backend>) -> CommandResult {
    let paths = backend.paths.clone();
    blocking(move || service::global_settings(&paths)).await
}

#[tauri::command]
async fn update_global_settings(
    backend: State<'_, Backend>,
    settings: Map<String, Value>,
) -> CommandResult<()> {
    let paths = backend.paths.clone();
    blocking(move || service::update_global_settings(&paths, &settings)).await
}

/// The `markoo` command, which hooks run. The app does not ship it yet, and an
/// app started from the Finder has a short `PATH`: the usual folders are tried too.
fn find_cli() -> CommandResult<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let usual = [
        home.map(|home| home.join(".cargo/bin")),
        Some(PathBuf::from("/opt/homebrew/bin")),
        Some(PathBuf::from("/usr/local/bin")),
    ];
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain(usual.into_iter().flatten())
        .map(|folder| folder.join("markoo"))
        .find(|file| file.is_file())
        .ok_or_else(|| {
            "Git hooks run the markoo command, which was not found: install it first".to_owned()
        })
}

#[tauri::command]
async fn hooks_state(backend: State<'_, Backend>, pair: String) -> CommandResult {
    let paths = backend.paths.clone();
    blocking(move || service::hooks_state(&paths, &pair)).await
}

#[tauri::command]
async fn install_hooks(backend: State<'_, Backend>, pair: String) -> CommandResult {
    let program = find_cli()?;
    let paths = backend.paths.clone();
    blocking(move || service::install_hooks(&paths, &pair, &program)).await
}

#[tauri::command]
async fn uninstall_hooks(backend: State<'_, Backend>, pair: String) -> CommandResult {
    let program = find_cli().unwrap_or_default();
    let paths = backend.paths.clone();
    blocking(move || service::uninstall_hooks(&paths, &pair, &program)).await
}

#[tauri::command]
fn inspect_kb(path: String) -> Value {
    service::inspect_kb(&path)
}

#[tauri::command]
fn inspect_repo(path: String) -> Value {
    service::inspect_repo(&path)
}

#[tauri::command]
fn known_kbs() -> Vec<PathBuf> {
    service::known_kbs()
}

/// Starts the daemon inside the app, unless something prevents it. The first
/// launch creates the configuration, as `markoo init` does.
fn start_daemon(paths: &Paths) -> (Option<Daemon>, Option<String>) {
    let config_file = paths.config_file();
    if !DiskFs.meta(config_file).is_ok_and(|meta| meta.is_some())
        && let Err(error) = ConfigFile::create(config_file)
        && !matches!(error, ConfigError::AlreadyExists(_))
    {
        return (None, Some(error.to_string()));
    }
    let started = tauri::async_runtime::block_on(Daemon::start(paths.clone(), Options::default()));
    match started {
        Ok(daemon) => (Some(daemon), None),
        // The launchd service does the job: there is nothing to report.
        Err(DaemonError::AlreadyRunning) => (None, None),
        Err(error) => (
            None,
            Some(format!("Markoo cannot sync on its own: {error}")),
        ),
    }
}

/// What the menu bar shows: the ring of the anchor takes the color of the state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrayState {
    UpToDate,
    Syncing,
    NeedsAttention,
}

impl TrayState {
    /// The whole icon, for the systems where it cannot be built in two layers.
    #[cfg(not(target_os = "macos"))]
    fn icon(self) -> &'static [u8] {
        match self {
            Self::UpToDate => include_bytes!("../icons/tray-ok.png"),
            Self::Syncing => include_bytes!("../icons/tray-syncing.png"),
            Self::NeedsAttention => include_bytes!("../icons/tray-attention.png"),
        }
    }

    /// The colored dot alone, on the same canvas as the anchor.
    #[cfg(target_os = "macos")]
    fn dot(self) -> &'static [u8] {
        match self {
            Self::UpToDate => include_bytes!("../icons/tray-dot-ok.png"),
            Self::Syncing => include_bytes!("../icons/tray-dot-syncing.png"),
            Self::NeedsAttention => include_bytes!("../icons/tray-dot-attention.png"),
        }
    }
}

/// Shows a state in the menu bar. It returns whether it worked.
#[cfg(target_os = "macos")]
fn show_tray_state(app: &AppHandle, state: TrayState) -> bool {
    app.run_on_main_thread(move || tray_macos::show_dot(state.dot()))
        .is_ok()
}

/// Shows a state in the menu bar. It returns whether it worked.
#[cfg(not(target_os = "macos"))]
fn show_tray_state(app: &AppHandle, state: TrayState) -> bool {
    let Some(tray) = app.tray_by_id(TRAY) else {
        return false;
    };
    Image::from_bytes(state.icon()).is_ok_and(|icon| tray.set_icon(Some(icon)).is_ok())
}

/// The state of every pair, with a sentence for the tooltip.
fn tray_status(paths: &Paths, queue: Option<&Queue>) -> (TrayState, String) {
    let Ok(pairs) = service::list_pairs(paths) else {
        return (
            TrayState::NeedsAttention,
            "Markoo cannot read its configuration".to_owned(),
        );
    };
    let pending: u64 = pairs
        .iter()
        .filter_map(|pair| pair["pending"].as_u64())
        .sum();
    let broken = pairs
        .iter()
        .filter(|pair| pair["errors"].as_array().is_some_and(|e| !e.is_empty()))
        .count();
    if pending > 0 || broken > 0 {
        return (
            TrayState::NeedsAttention,
            format!("Markoo: {pending} to settle, {broken} pair(s) in error"),
        );
    }
    if queue.is_some_and(|queue| !queue.active_pairs().is_empty()) {
        return (TrayState::Syncing, "Markoo: syncing".to_owned());
    }
    (TrayState::UpToDate, "Markoo: up to date".to_owned())
}

/// Keeps the menu bar and the window up to date.
async fn watch_state(app: AppHandle, paths: Paths, queue: Option<Queue>) {
    let mut events = queue.as_ref().map(Queue::subscribe);
    let mut shown = TrayState::UpToDate;
    loop {
        let event = async {
            match events.as_mut() {
                Some(events) => events.recv().await.is_ok(),
                None => std::future::pending().await,
            }
        };
        let from_daemon = tokio::select! {
            received = event => received,
            () = tokio::time::sleep(REFRESH) => false,
        };
        let (status_paths, status_queue) = (paths.clone(), queue.clone());
        let status = tauri::async_runtime::spawn_blocking(move || {
            tray_status(&status_paths, status_queue.as_ref())
        })
        .await;
        if let Ok((state, tooltip)) = status {
            if state != shown && show_tray_state(&app, state) {
                shown = state;
            }
            if let Some(tray) = app.tray_by_id(TRAY) {
                let _ = tray.set_tooltip(Some(tooltip));
            }
        }
        if from_daemon {
            let _ = app.emit(CHANGED, ());
        }
    }
}

fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(WINDOW) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn on_menu(app: &AppHandle, event: &MenuEvent) {
    match event.id().as_ref() {
        "open" => show_window(app),
        "sync" => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let backend = app.state::<Backend>();
                let _ = sync_everything(&backend).await;
                let _ = app.emit(CHANGED, ());
            });
        }
        "quit" => app.exit(0),
        _ => {}
    }
}

fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "open", "Open Markoo", true, None::<&str>)?,
            &MenuItem::with_id(app, "sync", "Sync now", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "quit", "Quit Markoo", true, None::<&str>)?,
        ],
    )?;
    let builder = TrayIconBuilder::with_id(TRAY)
        .menu(&menu)
        .on_menu_event(|app, event| on_menu(app, &event));
    // macOS tints a monochrome image to match each menu bar, and the dot, which
    // keeps its color, goes on top of it.
    #[cfg(target_os = "macos")]
    let builder = builder
        .icon(Image::from_bytes(include_bytes!(
            "../icons/tray-template.png"
        ))?)
        .icon_as_template(true);
    #[cfg(not(target_os = "macos"))]
    let builder = builder.icon(Image::from_bytes(TrayState::UpToDate.icon())?);
    builder.build(app)?;
    #[cfg(target_os = "macos")]
    tray_macos::show_dot(TrayState::UpToDate.dot());
    Ok(())
}

/// Runs the app.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let paths = Paths::resolve(&Overrides::default())?;
            let (daemon, note) = start_daemon(&paths);
            let queue = daemon.as_ref().map(|daemon| daemon.queue().clone());
            tauri::async_runtime::spawn(watch_state(
                app.handle().clone(),
                paths.clone(),
                queue.clone(),
            ));
            app.manage(Backend {
                paths,
                daemon: Mutex::new(daemon),
                queue,
                note,
            });
            build_tray(app)?;
            Ok(())
        })
        // Closing the window leaves the app in the menu bar, where it keeps syncing.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            list_pairs,
            pair_status,
            pair_diff,
            preview_new_pair,
            add_pair,
            preview_pair_settings,
            update_pair_settings,
            remove_pair,
            list_conflicts,
            conflict_draft,
            resolve_conflict,
            sync_pair,
            preview_filters,
            preview_new_pair_filters,
            preview_transform,
            global_settings,
            update_global_settings,
            hooks_state,
            install_hooks,
            uninstall_hooks,
            inspect_kb,
            inspect_repo,
            known_kbs,
        ])
        .build(tauri::generate_context!());
    let app = match result {
        Ok(app) => app,
        Err(error) => {
            eprintln!("error: cannot start Markoo: {error}");
            std::process::exit(1);
        }
    };
    app.run(|app, event| {
        if let RunEvent::Exit = event {
            let backend = app.state::<Backend>();
            let daemon = backend
                .daemon
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            if let Some(daemon) = daemon {
                tauri::async_runtime::block_on(daemon.stop());
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths_with_config(content: &str) -> (tempfile::TempDir, Paths) {
        let dir = tempfile::tempdir().unwrap();
        let config_file = dir.path().join("markoo.toml");
        std::fs::write(&config_file, content).unwrap();
        let paths = Paths::resolve(&Overrides {
            config_file: Some(config_file),
            data_dir: Some(dir.path().join("data")),
        })
        .unwrap();
        (dir, paths)
    }

    #[test]
    fn every_state_has_a_valid_icon() {
        for state in [
            TrayState::UpToDate,
            TrayState::Syncing,
            TrayState::NeedsAttention,
        ] {
            #[cfg(target_os = "macos")]
            let bytes = state.dot();
            #[cfg(not(target_os = "macos"))]
            let bytes = state.icon();
            assert!(Image::from_bytes(bytes).is_ok(), "{state:?}");
        }
    }

    #[test]
    fn no_pair_is_up_to_date() {
        let (_dir, paths) = paths_with_config("");
        assert_eq!(tray_status(&paths, None).0, TrayState::UpToDate);
    }

    #[test]
    fn an_invalid_configuration_is_a_warning() {
        let (_dir, paths) = paths_with_config("not toml =");
        assert_eq!(tray_status(&paths, None).0, TrayState::NeedsAttention);
    }

    #[test]
    fn a_pair_in_error_is_a_warning() {
        let pair = "[[pair]]\nname='a'\nkb='/nowhere/kb'\nrepo='/nowhere/repo'\n";
        let (_dir, paths) = paths_with_config(pair);
        assert_eq!(tray_status(&paths, None).0, TrayState::NeedsAttention);
    }

    #[test]
    fn a_missing_configuration_is_created_at_first_launch() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(&Overrides {
            config_file: Some(dir.path().join("sub/markoo.toml")),
            data_dir: Some(dir.path().join("data")),
        })
        .unwrap();
        // Whether the daemon starts depends on the default port being free.
        let (daemon, _) = tauri::async_runtime::block_on(start_daemon_for_test(&paths));
        if let Some(daemon) = daemon {
            tauri::async_runtime::block_on(daemon.stop());
        }
        assert!(paths.config_file().exists());
    }

    /// Avoids nesting `block_on` while keeping a runtime around the daemon.
    async fn start_daemon_for_test(paths: &Paths) -> (Option<Daemon>, Option<String>) {
        let paths = paths.clone();
        tauri::async_runtime::spawn_blocking(move || start_daemon(&paths))
            .await
            .unwrap()
    }
}
