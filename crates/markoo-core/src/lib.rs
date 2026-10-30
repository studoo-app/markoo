//! Markoo sync engine.
//!
//! All sync logic lives in this crate; the CLI, daemon, API, MCP server and
//! desktop app are thin layers over it.

pub mod apply;
pub mod config;
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

use std::path::PathBuf;

pub use apply::Report;
pub use engine::{Engine, FileDiff, Planned, Resolution, RunOutcome};
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
    /// Configuration failure.
    #[error(transparent)]
    Config(#[from] config::ConfigError),
    /// Invalid include or exclude pattern.
    #[error("invalid glob pattern: {0}")]
    Filter(#[from] globset::Error),
    /// A folder of the pair is unavailable; syncing would look like a mass deletion.
    #[error("pair \"{pair}\": the {side} folder is unavailable: {}", path.display())]
    MissingRoot {
        /// The pair.
        pair: String,
        /// Side of the folder.
        side: Side,
        /// The folder.
        path: PathBuf,
    },
    /// Moving from `both` to a one-way mode with unresolved issues.
    #[error(
        "pair \"{pair}\": {pending} conflict(s) must be resolved before switching to a one-way direction"
    )]
    DirectionBlocked {
        /// The pair.
        pair: String,
        /// Number of pending issues.
        pending: usize,
    },
    /// The entry has no conflict or divergence to settle.
    #[error("{0} has no conflict or divergence to resolve")]
    NotPending(EntryPath),
    /// The entry was excluded; only restoring the source brings it back.
    #[error("{0} is excluded from sync; keep the source version to bring it back")]
    AlreadyExcluded(EntryPath),
    /// This resolution writes to both sides, which a one-way pair forbids.
    #[error("this resolution needs a bidirectional pair")]
    NeedsBidirectional,
    /// This resolution only makes sense for a one-way pair.
    #[error("this resolution needs a one-way pair")]
    NeedsOneWay,
    /// Keeping both versions when one was deleted.
    #[error("{0} exists on one side only; keep one side instead")]
    KeepBothNeedsTwoVersions(EntryPath),
}
