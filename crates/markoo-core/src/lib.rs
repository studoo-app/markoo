//! Markoo sync engine.
//!
//! All sync logic lives in this crate; the CLI, daemon, API, MCP server and
//! desktop app are thin layers over it.

pub mod apply;
pub mod config;
pub mod engine;
pub mod entry;
pub mod fs;
pub mod hooks;
pub mod kb;
pub mod manage;
pub mod merge;
pub mod pair;
pub mod paths;
pub mod plan;
pub mod scan;
pub mod service;
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
    /// The pair is paused.
    #[error("pair \"{0}\" is paused")]
    Paused(String),
    /// No pending conflict or divergence has this identifier.
    #[error("no conflict or divergence {0}")]
    UnknownIssue(String),
    /// What a remote client asked for makes no sense.
    #[error("{0}")]
    InvalidRequest(String),
    /// Another sync of this pair is running, here or in another process.
    #[error("pair \"{0}\" is busy: another sync is running")]
    Busy(String),
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
