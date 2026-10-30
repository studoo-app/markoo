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
