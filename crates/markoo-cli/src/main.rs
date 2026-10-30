//! `markoo` command-line interface.

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use markoo_core::paths::{Overrides, Paths};

/// Smart sync between a knowledge base and a Git repository.
#[derive(Debug, Parser)]
#[command(name = "markoo", version, about)]
struct Cli {
    /// Path to the configuration file (overrides MARKOO_CONFIG).
    #[arg(long, global = true, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Path to the data directory (overrides MARKOO_DATA_DIR).
    #[arg(long, global = true, value_name = "DIR")]
    data_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Show the configuration file and data directory in use.
    Paths,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let overrides = Overrides {
        config_file: cli.config,
        data_dir: cli.data_dir,
    };

    match cli.command {
        Command::Paths => {
            let paths = Paths::resolve(&overrides)?;
            println!(
                "config  {}  ({})",
                paths.config_file().display(),
                paths.config_source()
            );
            println!(
                "data    {}  ({})",
                paths.data_dir().display(),
                paths.data_source()
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}
