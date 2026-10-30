//! Decides when to sync on its own: at startup, at each interval, and when
//! the configuration of a pair changes.

use std::time::Duration;

use markoo_core::config::Config;
use markoo_core::paths::Paths;
use tokio::sync::watch;
use tokio::time::{Instant, sleep};

use crate::Options;
use crate::queue::{Event, Queue, Trigger};

pub(crate) async fn run(
    paths: Paths,
    mut config: Config,
    queue: Queue,
    options: Options,
    mut stopped: watch::Receiver<bool>,
) {
    report_problems(&config).await;
    if config.schedule.on_startup {
        submit_all(&config, &queue, Trigger::Startup).await;
    }
    let mut next_due = due_after(&config);
    let mut last_error = None;
    loop {
        tokio::select! {
            () = sleep(options.config_poll) => {}
            _ = stopped.changed() => return,
        }
        let file = paths.config_file().to_path_buf();
        let loaded = tokio::task::spawn_blocking(move || Config::load(&file)).await;
        match loaded {
            Ok(Ok(new)) => {
                last_error = None;
                if new != config {
                    if new.schedule.interval != config.schedule.interval {
                        next_due = due_after(&new);
                    }
                    apply_change(&config, &new, &queue).await;
                    config = new;
                }
            }
            Ok(Err(error)) => {
                // Syncing with the previous settings could write in a direction
                // the user has just changed, so nothing runs until it is fixed.
                // Said once: the file is checked every few seconds.
                let message = error.to_string();
                if last_error.as_ref() != Some(&message) {
                    eprintln!("error: syncs are suspended until this is fixed: {message}");
                    last_error = Some(message);
                }
            }
            Err(error) => eprintln!("error: cannot read the configuration: {error}"),
        }
        if last_error.is_none() && next_due.is_some_and(|due| due <= Instant::now()) {
            submit_all(&config, &queue, Trigger::Interval).await;
            next_due = due_after(&config);
        }
    }
}

fn due_after(config: &Config) -> Option<Instant> {
    let interval: Duration = config.schedule.interval?;
    Some(Instant::now() + interval)
}

/// Syncs the pairs whose settings changed; a new pair counts as changed.
async fn apply_change(old: &Config, new: &Config, queue: &Queue) {
    if new.api != old.api {
        eprintln!("warning: restart the daemon to apply the changes to [api]");
    }
    report_problems(new).await;
    queue.publish(Event::ConfigReloaded);
    let changed: Vec<String> = new
        .pairs
        .iter()
        .filter(|entry| old.pair(&entry.pair.name).ok() != Some(*entry))
        .map(|entry| entry.pair.name.clone())
        .collect();
    let runnable = runnable(new).await;
    for name in changed.iter().filter(|name| runnable.contains(*name)) {
        queue.submit(name, false, Trigger::Config);
    }
}

async fn submit_all(config: &Config, queue: &Queue, trigger: Trigger) {
    for name in runnable(config).await {
        queue.submit(&name, false, trigger);
    }
}

/// The pairs that can sync: neither paused nor failing validation.
async fn runnable(config: &Config) -> Vec<String> {
    let config = config.clone();
    tokio::task::spawn_blocking(move || {
        let validation = config.validate();
        config
            .pairs
            .iter()
            .map(|entry| &entry.pair)
            .filter(|pair| !pair.paused && validation.is_ok(&pair.name))
            .map(|pair| pair.name.clone())
            .collect()
    })
    .await
    .unwrap_or_default()
}

async fn report_problems(config: &Config) {
    for option in &config.not_implemented {
        eprintln!("warning: {option} is not implemented yet and is ignored");
    }
    let config = config.clone();
    let validation = tokio::task::spawn_blocking(move || config.validate()).await;
    let Ok(validation) = validation else { return };
    for problem in &validation.errors {
        eprintln!("error: pair \"{}\": {}", problem.pair, problem.message);
    }
    for problem in &validation.warnings {
        eprintln!("warning: pair \"{}\": {}", problem.pair, problem.message);
    }
}
