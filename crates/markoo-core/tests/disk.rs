//! Sync on the real file system, starting from the plain Markdown fixture.

// Test helpers panic on failure: that is what fails the test.
#![expect(clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use markoo_core::fs::{DiskFs, Fs};
use markoo_core::paths::{Overrides, Paths};
use markoo_core::{Engine, Pair};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/markdown/kb")
}

fn copy_dir(from: &Path, to: &Path) {
    for relative in DiskFs.list(from).unwrap() {
        let bytes = DiskFs.read(&from.join(&relative)).unwrap();
        DiskFs.write_atomic(&to.join(&relative), &bytes).unwrap();
    }
}

struct Setup {
    _dir: tempfile::TempDir,
    engine: Engine<DiskFs>,
    pair: Pair,
    data: PathBuf,
}

fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let kb = dir.path().join("kb");
    let repo = dir.path().join("repo/docs");
    copy_dir(&fixture(), &kb);
    std::fs::create_dir_all(&repo).unwrap();
    let paths = Paths::resolve(&Overrides {
        config_file: Some(dir.path().join("markoo.toml")),
        data_dir: Some(dir.path().join("data")),
    })
    .unwrap();
    let engine = Engine::open(DiskFs, &paths).unwrap();
    Setup {
        _dir: dir,
        engine,
        pair: Pair::new("fixture", kb, repo),
        data: paths.data_dir().to_path_buf(),
    }
}

#[test]
fn first_sync_copies_the_whole_kb() {
    let s = setup();
    s.engine.sync(&s.pair).unwrap();
    let mut copied = DiskFs.list(&s.pair.repo).unwrap();
    copied.sort();
    assert_eq!(
        copied,
        vec![
            PathBuf::from("adr").join("001-stack.md"),
            PathBuf::from("architecture.md")
        ]
    );
}

#[test]
fn second_sync_on_disk_has_nothing_to_do() {
    let s = setup();
    s.engine.sync(&s.pair).unwrap();
    assert!(s.engine.plan(&s.pair).unwrap().plan.is_empty());
}

#[test]
fn state_survives_reopening_the_engine() {
    let s = setup();
    s.engine.sync(&s.pair).unwrap();
    let data = s.pair.kb.parent().unwrap().join("data");
    let paths = Paths::resolve(&Overrides {
        config_file: Some(PathBuf::from("unused.toml")),
        data_dir: Some(data),
    })
    .unwrap();
    let reopened = Engine::open(DiskFs, &paths).unwrap();
    assert!(reopened.plan(&s.pair).unwrap().plan.is_empty());
}

#[test]
fn sync_is_refused_while_another_engine_holds_the_lock() {
    let s = setup();
    let other = Engine::new(
        DiskFs,
        markoo_core::state::State::open_in_memory().unwrap(),
        s.data.clone(),
    );
    let _lock = other.lock_pair("fixture").unwrap();
    assert!(matches!(
        s.engine.sync(&s.pair),
        Err(markoo_core::Error::Busy(_))
    ));
}

#[test]
fn a_refused_sync_writes_nothing() {
    let s = setup();
    let _lock = s.engine.lock_pair("fixture").unwrap();
    let _ = s.engine.sync(&s.pair);
    assert!(DiskFs.list(&s.pair.repo).unwrap().is_empty());
}

#[test]
fn the_lock_is_free_again_after_a_sync() {
    let s = setup();
    s.engine.sync(&s.pair).unwrap();
    assert!(s.engine.lock_pair("fixture").is_ok());
}

#[test]
fn a_dry_run_works_while_a_sync_holds_the_lock() {
    let s = setup();
    let _lock = s.engine.lock_pair("fixture").unwrap();
    assert!(s.engine.run(&s.pair, true).is_ok());
}

#[test]
fn locks_of_different_pairs_are_independent() {
    let s = setup();
    let _lock = s.engine.lock_pair("fixture").unwrap();
    assert!(s.engine.lock_pair("other").is_ok());
}
