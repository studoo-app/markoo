//! Markoo sync engine.
//!
//! All sync logic lives in this crate; the CLI, daemon, API, MCP server and
//! desktop app are thin layers over it.

pub mod apply;
pub mod engine;
pub mod entry;
pub mod fs;
pub mod kb;
pub mod merge;
pub mod pair;
pub mod paths;
pub mod plan;
pub mod scan;
pub mod state;

pub use apply::Report;
pub use engine::{Engine, Planned};
pub use entry::{EntryPath, Side};
pub use pair::{Direction, Pair, TargetChanges};

/// Errors raised by the engine.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// File system failure.
    #[error(transparent)]
    Fs(#[from] fs::FsError),
    /// State database failure.
    #[error(transparent)]
    State(#[from] state::StateError),
    /// Invalid include or exclude pattern.
    #[error("invalid glob pattern: {0}")]
    Filter(#[from] globset::Error),
}
