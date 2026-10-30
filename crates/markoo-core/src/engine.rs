//! Entry point of the sync engine: scan, plan, apply.

use std::path::PathBuf;

use serde::Serialize;

use crate::Error;
use crate::apply::{Applier, Report};
use crate::entry::Side;
use crate::fs::Fs;
use crate::kb::Gfm;
use crate::pair::{Filter, Pair};
use crate::paths::Paths;
use crate::plan::{self, Plan};
use crate::scan::{ScanIssue, scan_side};
use crate::state::{PendingIssue, State};

/// Name of the state database inside `<data>`.
pub const STATE_FILE: &str = "state.db";

/// A plan together with the files the scan had to leave out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Planned {
    /// What the sync would do.
    pub plan: Plan,
    /// Files that could not be scanned (unreadable, not UTF-8…).
    pub scan_issues: Vec<ScanIssue>,
}

/// The sync engine for one data directory.
#[derive(Debug)]
pub struct Engine<F: Fs> {
    fs: F,
    state: State,
    data_dir: PathBuf,
}

impl<F: Fs> Engine<F> {
    /// Creates an engine from its parts.
    pub fn new(fs: F, state: State, data_dir: impl Into<PathBuf>) -> Self {
        Self {
            fs,
            state,
            data_dir: data_dir.into(),
        }
    }

    /// Opens the engine on the resolved data directory (`<data>/state.db`).
    pub fn open(fs: F, paths: &Paths) -> Result<Self, Error> {
        let state = State::open(&paths.data_dir().join(STATE_FILE))?;
        Ok(Self::new(fs, state, paths.data_dir()))
    }

    /// The file system used by the engine.
    pub fn fs(&self) -> &F {
        &self.fs
    }

    /// Computes what a sync of `pair` would do, without writing anything.
    pub fn plan(&self, pair: &Pair) -> Result<Planned, Error> {
        let adapter = pair.kb_type.adapter();
        let filter = Filter::new(pair, adapter.excluded())?;
        let bases = self.state.bases(&pair.name)?;
        let kb = scan_side(&self.fs, &pair.kb, Side::Kb, &filter, adapter, &bases)?;
        let repo = scan_side(&self.fs, &pair.repo, Side::Repo, &filter, &Gfm, &bases)?;
        let plan = plan::plan(pair, &bases, &kb, &repo);
        let mut scan_issues = kb.issues;
        scan_issues.extend(repo.issues);
        Ok(Planned { plan, scan_issues })
    }

    /// Applies a plan computed by [`Engine::plan`].
    pub fn apply(&self, pair: &Pair, plan: &Plan) -> Result<Report, Error> {
        let applier = Applier {
            fs: &self.fs,
            state: &self.state,
            pair,
            kb_dialect: pair.kb_type.adapter(),
            repo_dialect: &Gfm,
            data_dir: &self.data_dir,
        };
        Ok(applier.apply(plan)?)
    }

    /// Plans and applies in one go.
    pub fn sync(&self, pair: &Pair) -> Result<Report, Error> {
        let planned = self.plan(pair)?;
        self.apply(pair, &planned.plan)
    }

    /// Conflicts and divergences waiting for the user.
    pub fn pending(&self, pair: &Pair) -> Result<Vec<PendingIssue>, Error> {
        Ok(self.state.pending(&pair.name)?)
    }
}
