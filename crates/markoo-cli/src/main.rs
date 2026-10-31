//! `markoo` command-line interface.

mod commands;
mod output;
mod wizard;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{ArgGroup, CommandFactory, Parser, Subcommand, ValueEnum};

/// Smart sync between a knowledge base and a Git repository.
#[derive(Debug, Parser)]
#[command(name = "markoo", about, disable_version_flag = true)]
struct Cli {
    /// Print the version.
    #[arg(short = 'V', long)]
    version: bool,

    /// Path to the configuration file (overrides MARKOO_CONFIG).
    #[arg(long, global = true, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Path to the data directory (overrides MARKOO_DATA_DIR).
    #[arg(long, global = true, value_name = "DIR")]
    data_dir: Option<PathBuf>,

    /// Print machine-readable JSON.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create the configuration file and the data directory.
    Init,
    /// Show what a sync would do.
    Status {
        /// Only this pair.
        #[arg(long)]
        pair: Option<String>,
    },
    /// Show the differences between the two sides.
    Diff {
        /// Only this file, relative to the pair's folders.
        file: Option<String>,
        /// Only this pair.
        #[arg(long)]
        pair: Option<String>,
    },
    /// Synchronize.
    Sync {
        /// Show the plan without writing anything.
        #[arg(long)]
        dry_run: bool,
        /// Only this pair.
        #[arg(long)]
        pair: Option<String>,
    },
    /// List conflicts and divergences.
    Conflicts {
        /// Only this pair.
        #[arg(long)]
        pair: Option<String>,
    },
    /// Settle a conflict or a divergence.
    #[command(group(ArgGroup::new("how").required(true).args(["keep", "edit", "content_file", "push_to_source"])))]
    Resolve {
        /// The file, relative to the pair's folders.
        file: String,
        /// The pair, when several have this file.
        #[arg(long)]
        pair: Option<String>,
        /// Keep this version.
        #[arg(long, value_enum)]
        keep: Option<Keep>,
        /// Edit the merged text in $EDITOR.
        #[arg(long)]
        edit: bool,
        /// Use the content of this file on both sides.
        #[arg(long, value_name = "FILE")]
        content_file: Option<PathBuf>,
        /// One-way pairs: copy the target's version to the source, once.
        #[arg(long)]
        push_to_source: bool,
    },
    /// Manage pairs.
    #[command(subcommand)]
    Pair(PairCommand),
    /// Show the configuration file and data directory in use.
    Paths,
    /// Run the scheduler and the local API until stopped.
    Daemon {
        #[command(subcommand)]
        action: Option<DaemonCommand>,
    },
    /// Manage the token of the local API.
    #[command(subcommand)]
    Token(TokenCommand),
    /// Run the MCP server on stdin and stdout, for a client that spawns it.
    Mcp,
    /// Manage the Git hooks of a pair.
    #[command(subcommand)]
    Hook(HookCommand),
}

#[derive(Debug, Subcommand)]
enum HookCommand {
    /// Install the hooks listed in the pair's `[pair.hooks]`, by default `post-merge`.
    Install {
        /// The pair.
        #[arg(long)]
        pair: String,
    },
    /// Remove the pair's hooks and nothing else.
    Uninstall {
        /// The pair.
        #[arg(long)]
        pair: String,
    },
    /// What a hook runs; a failure is logged and Git carries on.
    #[command(hide = true)]
    Run {
        /// The hook.
        hook: String,
        /// The pair.
        #[arg(long)]
        pair: String,
        /// What Git passes to the hook.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        arguments: Vec<String>,
    },
}

#[derive(Debug, Subcommand)]
enum DaemonCommand {
    /// Install and start the daemon as a launchd service (macOS).
    Install,
    /// Stop and remove the launchd service (macOS).
    Uninstall,
}

#[derive(Debug, Subcommand)]
enum TokenCommand {
    /// Replace the token; a running daemon picks it up at once.
    Rotate,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Keep {
    Kb,
    Repo,
    Both,
}

#[derive(Debug, Subcommand)]
enum PairCommand {
    /// Add a pair; without arguments, asks the questions one by one.
    Add {
        /// Name of the pair.
        name: Option<String>,
        /// Knowledge base folder.
        #[arg(long)]
        kb: Option<String>,
        /// Knowledge base type; detected if omitted.
        #[arg(long)]
        kb_type: Option<String>,
        /// Repository folder.
        #[arg(long)]
        repo: Option<String>,
        /// both, kb-to-repo or repo-to-kb.
        #[arg(long)]
        direction: Option<String>,
        /// protect or overwrite (one-way pairs).
        #[arg(long)]
        target_changes: Option<String>,
    },
    /// List the pairs.
    List,
    /// Show a pair.
    Show {
        /// Name of the pair.
        name: String,
    },
    /// Change a setting of a pair.
    Set {
        /// Name of the pair.
        name: String,
        /// direction, target_changes, kb, repo, kb_type, include, exclude, paused.
        key: String,
        /// New value; lists are comma-separated.
        value: String,
        /// Also set target_changes.
        #[arg(long)]
        target_changes: Option<String>,
    },
    /// Remove a pair. Its files are not touched.
    Remove {
        /// Name of the pair.
        name: String,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

/// Runs the command; `Ok(false)` means it reported errors along the way.
fn run(cli: Cli) -> Result<bool> {
    let style = output::Style::detect();
    if cli.version {
        print!("{}", style.banner());
        println!("markoo {}", env!("CARGO_PKG_VERSION"));
        return Ok(true);
    }
    let Some(command) = cli.command else {
        Cli::command().print_help()?;
        return Ok(true);
    };
    let ctx = commands::Ctx::new(cli.config, cli.data_dir, cli.json, style)?;
    match command {
        Command::Init => ctx.init()?,
        Command::Status { pair } => ctx.status(pair.as_deref())?,
        Command::Diff { file, pair } => ctx.diff(file.as_deref(), pair.as_deref())?,
        Command::Sync { dry_run, pair } => ctx.sync(dry_run, pair.as_deref())?,
        Command::Conflicts { pair } => ctx.conflicts(pair.as_deref())?,
        Command::Resolve {
            file,
            pair,
            keep,
            edit,
            content_file,
            push_to_source,
        } => ctx.resolve(
            &file,
            pair.as_deref(),
            commands::How::from_flags(keep, edit, content_file, push_to_source),
        )?,
        Command::Pair(command) => ctx.pair(command)?,
        Command::Paths => ctx.paths()?,
        Command::Daemon { action } => ctx.daemon(action)?,
        Command::Token(TokenCommand::Rotate) => ctx.token_rotate()?,
        Command::Hook(command) => ctx.hook(command)?,
        Command::Mcp => ctx.mcp()?,
    }
    Ok(ctx.succeeded())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}
