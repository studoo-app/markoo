//! The Obsidian adapter through the whole engine.

// Test helpers panic on failure: that is what fails the test.
#![expect(clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use markoo_core::fs::{DiskFs, Fs, MemFs};
use markoo_core::kb::KbType;
use markoo_core::paths::{Overrides, Paths};
use markoo_core::state::State;
use markoo_core::{Direction, Engine, Pair, Report, Side, TargetChanges};
use pulldown_cmark::{Event, Options, Parser, Tag};

struct Vault {
    engine: Engine<MemFs>,
    pair: Pair,
}

const KB: &str = "/vault/Projets/Clara";

impl Vault {
    fn new() -> Self {
        Self::with_settings("{}")
    }

    fn with_settings(app_json: &str) -> Self {
        let mut pair = Pair::new("test", KB, "/repo");
        pair.kb_type = KbType::Obsidian;
        let engine = Engine::new(MemFs::new(), State::open_in_memory().unwrap(), "/data");
        engine
            .fs()
            .write_atomic(Path::new("/vault/.obsidian/app.json"), app_json.as_bytes())
            .unwrap();
        Self { engine, pair }
    }

    fn kb(&self, file: &str, text: &str) {
        self.engine
            .fs()
            .write_atomic(&Path::new(KB).join(file), text.as_bytes())
            .unwrap();
    }

    fn vault(&self, file: &str, text: &str) {
        self.engine
            .fs()
            .write_atomic(&Path::new("/vault").join(file), text.as_bytes())
            .unwrap();
    }

    fn repo(&self, file: &str, text: &str) {
        self.engine
            .fs()
            .write_atomic(&Path::new("/repo").join(file), text.as_bytes())
            .unwrap();
    }

    fn read_kb(&self, file: &str) -> Option<String> {
        self.engine.fs().read_string(Path::new(KB).join(file))
    }

    fn read_vault(&self, file: &str) -> Option<String> {
        self.engine.fs().read_string(Path::new("/vault").join(file))
    }

    fn read_repo(&self, file: &str) -> Option<String> {
        self.engine.fs().read_string(Path::new("/repo").join(file))
    }

    fn sync(&self) -> Report {
        self.engine.sync(&self.pair).unwrap()
    }

    fn is_settled(&self) -> bool {
        self.engine.plan(&self.pair).unwrap().plan.is_empty()
    }
}

#[test]
fn links_are_converted_towards_the_repo() {
    let v = Vault::new();
    v.kb("a.md", "[[b]]\n");
    v.kb("b.md", "B\n");
    v.sync();
    assert_eq!(v.read_repo("a.md").as_deref(), Some("[b](./b.md)\n"));
}

#[test]
fn vault_file_is_not_rewritten_by_a_sync() {
    let v = Vault::new();
    let text = "---\naliases: [x]\n---\n[[b]] [[b|alias]] [[b#H]]\n\n> [!info]\n> Hi\n";
    v.kb("a.md", text);
    v.kb("b.md", "B\n");
    v.sync();
    assert_eq!(v.read_kb("a.md").as_deref(), Some(text));
}

#[test]
fn sync_settles_at_once() {
    let v = Vault::new();
    v.kb(
        "a.md",
        "---\naliases: [x]\n---\n[[b]] [[b#H]] [[Elsewhere]]\n\n> [!info]\n> Hi\n",
    );
    v.kb("b.md", "B\n");
    v.vault("Elsewhere.md", "E\n");
    v.sync();
    assert!(v.is_settled());
}

#[test]
fn second_sync_writes_nothing() {
    let v = Vault::new();
    v.kb("a.md", "[[b]] ![[pic.png]]\n");
    v.kb("b.md", "B\n");
    v.vault("attachments/pic.png", "png");
    v.sync();
    let before = v.engine.fs().write_count();
    v.sync();
    assert_eq!(v.engine.fs().write_count(), before);
}

/// The sequence behind the loops this design must avoid: a repo edit reaches the
/// vault, the vault file is then saved again by Obsidian with the same content.
#[test]
fn repo_edit_does_not_bounce_back() {
    let v = Vault::new();
    v.kb("a.md", "[[b]]\n");
    v.kb("b.md", "B\n");
    v.sync();
    v.repo("a.md", "[b](./b.md) and more\n");
    v.sync();
    let vault_text = v.read_kb("a.md").unwrap();
    v.kb("a.md", &vault_text); // touches the file: it is read again
    assert!(v.is_settled());
}

#[test]
fn new_repo_note_linking_to_a_new_repo_note_gets_wikilinks() {
    let v = Vault::new();
    v.repo("a.md", "[b](./b.md)\n");
    v.repo("b.md", "B\n");
    v.sync();
    assert_eq!(v.read_kb("a.md").as_deref(), Some("[[b]]\n"));
}

#[test]
fn new_repo_notes_settle_after_a_later_read() {
    let v = Vault::new();
    v.repo("a.md", "[b](./b.md)\n");
    v.repo("b.md", "B\n");
    v.sync();
    let text = v.read_kb("a.md").unwrap();
    v.kb("a.md", &text);
    assert!(v.is_settled());
}

#[test]
fn merge_keeps_links_converted_on_both_sides() {
    let v = Vault::new();
    v.kb("a.md", "[[b]]\nL2\nL3\nL4\n");
    v.kb("b.md", "B\n");
    v.sync();
    v.kb("a.md", "[[b]]\nL2 kb\nL3\nL4\n");
    v.repo("a.md", "[b](./b.md)\nL2\nL3\nL4 repo\n");
    v.sync();
    assert_eq!(
        (v.read_kb("a.md").as_deref(), v.read_repo("a.md").as_deref()),
        (
            Some("[[b]]\nL2 kb\nL3\nL4 repo\n"),
            Some("[b](./b.md)\nL2 kb\nL3\nL4 repo\n")
        )
    );
}

#[test]
fn notes_excluded_by_the_vault_settings_folder_are_ignored() {
    let v = Vault::new();
    v.kb(".trash/old.md", "old\n");
    v.sync();
    assert_eq!(v.read_repo(".trash/old.md"), None);
}

#[test]
fn excluded_note_counts_as_outside_the_pair() {
    let mut v = Vault::new();
    v.pair.exclude = vec!["drafts/**".to_owned()];
    v.kb("a.md", "[[idea]]\n");
    v.kb("drafts/idea.md", "I\n");
    v.sync();
    assert_eq!(
        v.read_repo("a.md").as_deref(),
        Some("idea<!--markoo:[[idea]]-->\n")
    );
}

// --- Attachments ------------------------------------------------------------------

#[test]
fn referenced_attachment_is_copied_to_the_repo() {
    let v = Vault::new();
    v.kb("a.md", "![[pic.png]]\n");
    v.vault("attachments/pic.png", "png");
    v.sync();
    assert_eq!(v.read_repo("assets/pic.png").as_deref(), Some("png"));
}

#[test]
fn unreferenced_attachment_is_ignored() {
    let v = Vault::new();
    v.kb("a.md", "text\n");
    v.vault("attachments/pic.png", "png");
    v.sync();
    assert_eq!(v.read_repo("assets/pic.png"), None);
}

#[test]
fn repo_attachment_goes_to_the_configured_folder() {
    let v = Vault::with_settings(r#"{"attachmentFolderPath":"files"}"#);
    v.repo("a.md", "![pic](./assets/pic.png)\n");
    v.repo("assets/pic.png", "png");
    v.sync();
    assert_eq!(v.read_vault("files/pic.png").as_deref(), Some("png"));
}

#[test]
fn repo_attachment_goes_next_to_the_note_when_configured() {
    let v = Vault::with_settings(r#"{"attachmentFolderPath":"./"}"#);
    v.repo("sub/a.md", "![pic](../assets/pic.png)\n");
    v.repo("assets/pic.png", "png");
    v.sync();
    assert_eq!(v.read_kb("sub/pic.png").as_deref(), Some("png"));
}

#[test]
fn repo_image_becomes_an_embed_in_the_vault() {
    let v = Vault::new();
    v.repo("a.md", "![pic](./assets/pic.png)\n");
    v.repo("assets/pic.png", "png");
    v.sync();
    assert_eq!(v.read_kb("a.md").as_deref(), Some("![[pic.png]]\n"));
}

#[test]
fn repo_image_flow_settles() {
    let v = Vault::new();
    v.repo("a.md", "![pic](./assets/pic.png)\n");
    v.repo("assets/pic.png", "png");
    v.sync();
    let text = v.read_kb("a.md").unwrap();
    v.kb("a.md", &text);
    assert!(v.is_settled());
}

#[test]
fn changed_attachment_follows() {
    let v = Vault::new();
    v.kb("a.md", "![[pic.png]]\n");
    v.vault("attachments/pic.png", "v1");
    v.sync();
    v.vault("attachments/pic.png", "v2");
    v.sync();
    assert_eq!(v.read_repo("assets/pic.png").as_deref(), Some("v2"));
}

#[test]
fn overwritten_attachment_is_backed_up() {
    let v = Vault::new();
    v.kb("a.md", "![[pic.png]]\n");
    v.vault("attachments/pic.png", "v1");
    v.sync();
    v.vault("attachments/pic.png", "v2");
    v.sync();
    assert_eq!(v.engine.fs().files_under("/data/backups").len(), 1);
}

#[test]
fn attachment_changed_on_both_sides_is_a_conflict() {
    let v = Vault::new();
    v.kb("a.md", "![[pic.png]]\n");
    v.vault("attachments/pic.png", "v1");
    v.sync();
    v.vault("attachments/pic.png", "kb");
    v.repo("assets/pic.png", "repo");
    assert_eq!(v.sync().attachment_conflicts, vec!["pic.png".to_owned()]);
}

#[test]
fn attachment_conflict_touches_nothing() {
    let v = Vault::new();
    v.kb("a.md", "![[pic.png]]\n");
    v.vault("attachments/pic.png", "v1");
    v.sync();
    v.vault("attachments/pic.png", "kb");
    v.repo("assets/pic.png", "repo");
    v.sync();
    assert_eq!(
        (
            v.read_vault("attachments/pic.png").as_deref(),
            v.read_repo("assets/pic.png").as_deref()
        ),
        (Some("kb"), Some("repo"))
    );
}

#[test]
fn deleted_reference_never_deletes_the_attachment() {
    let v = Vault::new();
    v.kb("a.md", "![[pic.png]]\n");
    v.vault("attachments/pic.png", "png");
    v.sync();
    v.kb("a.md", "no more image\n");
    v.sync();
    assert_eq!(v.read_repo("assets/pic.png").as_deref(), Some("png"));
}

#[test]
fn one_way_never_writes_attachments_to_the_source() {
    let mut v = Vault::new();
    v.pair.direction = Direction::KbToRepo;
    v.pair.target_changes = TargetChanges::Protect;
    v.repo("assets/pic.png", "png");
    v.kb("a.md", "![pic](./assets/pic.png)\n");
    let report = v.sync();
    assert!(report.written.iter().all(|file| file.side == Side::Repo));
}

// --- Golden: the fixture vault becomes exactly the fixture repo ----------------------

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn copy_dir(from: &Path, to: &Path) {
    for relative in DiskFs.list(from).unwrap() {
        let bytes = DiskFs.read(&from.join(&relative)).unwrap();
        DiskFs.write_atomic(&to.join(&relative), &bytes).unwrap();
    }
}

/// Reads a text fixture without the line endings Git may add on Windows.
fn normalized(path: &Path) -> Vec<u8> {
    let bytes = DiskFs.read(path).unwrap();
    String::from_utf8(bytes.clone())
        .map(|text| text.replace("\r\n", "\n").into_bytes())
        .unwrap_or(bytes)
}

struct Golden {
    _dir: tempfile::TempDir,
    engine: Engine<DiskFs>,
    pair: Pair,
}

fn golden() -> Golden {
    let dir = tempfile::tempdir().unwrap();
    let kb = dir.path().join("kb");
    let repo = dir.path().join("repo");
    copy_dir(&fixtures().join("obsidian/kb"), &kb);
    std::fs::create_dir_all(&repo).unwrap();
    let paths = Paths::resolve(&Overrides {
        config_file: Some(dir.path().join("c.toml")),
        data_dir: Some(dir.path().join("data")),
    })
    .unwrap();
    let engine = Engine::open(DiskFs, &paths).unwrap();
    let mut pair = Pair::new("golden", kb, repo);
    pair.kb_type = KbType::Obsidian;
    Golden {
        _dir: dir,
        engine,
        pair,
    }
}

#[test]
fn golden_repo_files_are_as_expected() {
    let g = golden();
    g.engine.sync(&g.pair).unwrap();
    let expected_root = fixtures().join("repo/docs");
    let mut produced = DiskFs.list(&g.pair.repo).unwrap();
    let mut expected = DiskFs.list(&expected_root).unwrap();
    produced.sort();
    expected.sort();
    assert_eq!(produced, expected);
    for file in expected {
        assert_eq!(
            String::from_utf8_lossy(&normalized(&g.pair.repo.join(&file))),
            String::from_utf8_lossy(&normalized(&expected_root.join(&file))),
            "{}",
            file.display()
        );
    }
}

#[test]
fn golden_vault_is_left_untouched() {
    let g = golden();
    g.engine.sync(&g.pair).unwrap();
    let source = fixtures().join("obsidian/kb");
    for file in DiskFs.list(&source).unwrap() {
        assert_eq!(
            normalized(&g.pair.kb.join(&file)),
            normalized(&source.join(&file)),
            "{}",
            file.display()
        );
    }
}

#[test]
fn golden_settles() {
    let g = golden();
    g.engine.sync(&g.pair).unwrap();
    assert!(g.engine.plan(&g.pair).unwrap().plan.is_empty());
}

#[test]
fn golden_repo_links_all_resolve() {
    let g = golden();
    g.engine.sync(&g.pair).unwrap();
    assert_eq!(broken_links(&g.pair.repo), Vec::<String>::new());
}

/// Relative links and images of a folder pointing to nothing, like GitHub would see.
fn broken_links(root: &Path) -> Vec<String> {
    let mut broken = Vec::new();
    for relative in DiskFs.list(root).unwrap() {
        if relative.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let file = root.join(&relative);
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        for event in Parser::new_ext(&text, Options::all()) {
            let dest = match &event {
                Event::Start(Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. }) => dest_url,
                _ => continue,
            };
            if dest.contains("://") || dest.starts_with('#') || dest.starts_with("mailto:") {
                continue;
            }
            let path = dest
                .split('#')
                .next()
                .unwrap_or_default()
                .replace("%20", " ");
            if !file.parent().unwrap().join(&path).exists() {
                broken.push(format!("{}: {dest}", relative.display()));
            }
        }
    }
    broken
}

/// The hash of every file of a folder.
fn fingerprints(root: &Path) -> BTreeMap<PathBuf, blake3::Hash> {
    DiskFs
        .list(root)
        .unwrap()
        .into_iter()
        .map(|file| {
            let hash = blake3::hash(&DiskFs.read(&root.join(&file)).unwrap());
            (file, hash)
        })
        .collect()
}

/// Prints the first lines of a long list, so that the results stay on screen.
fn print_some(items: &[String]) {
    const SHOWN: usize = 30;
    for item in items.iter().take(SHOWN) {
        println!("  {item}");
    }
    if items.len() > SHOWN {
        println!("  ... and {} more", items.len() - SHOWN);
    }
}

/// Runs a real vault through the engine, on a copy: the vault itself is only read.
///
/// `MARKOO_CORPUS=/path/to/vault cargo test -p markoo-core --test obsidian -- --ignored --nocapture`
///
/// Prints file names, never their content. The vault may be in use during the
/// run, so it is only read once: everything is compared to the copy made then.
#[test]
#[ignore = "needs MARKOO_CORPUS, the path of a real vault"]
fn real_vault_round_trip() {
    let source = PathBuf::from(std::env::var("MARKOO_CORPUS").unwrap());
    let dir = tempfile::tempdir().unwrap();
    let kb = dir.path().join("vault");
    let repo = dir.path().join("repo");
    assert!(source.is_dir(), "no such folder: {}", source.display());
    copy_dir(&source, &kb);
    let before = fingerprints(&kb);
    std::fs::create_dir_all(&repo).unwrap();
    let paths = Paths::resolve(&Overrides {
        config_file: Some(dir.path().join("c.toml")),
        data_dir: Some(dir.path().join("data")),
    })
    .unwrap();
    let engine = Engine::open(DiskFs, &paths).unwrap();
    let mut pair = Pair::new("corpus", &kb, &repo);
    pair.kb_type = KbType::Obsidian;

    let planned = engine.plan(&pair).unwrap();
    println!(
        "{} entries, {} attachments",
        planned.plan.entries.len(),
        planned.plan.attachments.len()
    );
    let (unresolved, other_issues): (Vec<_>, Vec<_>) = planned
        .scan_issues
        .iter()
        .partition(|issue| issue.message.starts_with("link target not found"));
    let other_issues: Vec<String> = other_issues
        .iter()
        .map(|issue| format!("{}: {}", issue.path.display(), issue.message))
        .collect();
    println!("{} links without a target in the vault", unresolved.len());
    println!("{} other scan issues", other_issues.len());
    print_some(&other_issues);
    let report = engine.apply(&pair, &planned.plan).unwrap();
    println!(
        "{} written, {} skipped",
        report.written.len(),
        report.skipped.len()
    );
    let skipped: Vec<String> = report
        .skipped
        .iter()
        .map(|skipped| format!("{}: {}", skipped.path, skipped.reason))
        .collect();
    print_some(&skipped);

    let after = fingerprints(&kb);
    let changed: Vec<String> = before
        .keys()
        .chain(after.keys())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter(|file| before.get(*file) != after.get(*file))
        .map(|file| file.display().to_string())
        .collect();
    println!("{} vault files changed by the sync", changed.len());
    print_some(&changed);

    let again = engine.plan(&pair).unwrap().plan;
    println!("{} entries left after the sync", again.entries.len());
    let left: Vec<String> = again.entries.iter().map(|e| e.path.to_string()).collect();
    print_some(&left);
    let broken = broken_links(&repo);
    println!("{} broken links on the repo side", broken.len());
    print_some(&broken);
    assert!(changed.is_empty() && again.is_empty());
}
