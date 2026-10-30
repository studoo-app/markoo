use std::path::Path;

use proptest::prelude::*;

use super::*;
use crate::entry::EntryPath;

const NOTES: &[&str] = &[
    "Projets/Clara/architecture.md",
    "Projets/Clara/adr/001-stack.md",
    "Projets/Clara/Café Notes.md",
    "Projets/Clara/twin.md",
    "Projets/Clara/sub/twin.md",
    "Réunions/March 12.md",
];
const ATTACHMENTS: &[&str] = &[
    "attachments/schema.png",
    "attachments/spec.pdf",
    "Media/a/logo.png",
    "Media/b/logo.png",
];
const FROM: &str = "architecture.md";

fn index() -> NameIndex {
    NameIndex::for_tests(NOTES, ATTACHMENTS, "Projets/Clara")
}

fn to(text: &str) -> String {
    to_from(text, FROM)
}

fn to_from(text: &str, from: &str) -> String {
    let (path, index) = (EntryPath::from_relative(Path::new(from)).unwrap(), index());
    let ctx = ConvertCtx {
        path: &path,
        index: &index,
    };
    ObsidianAdapter::default().to_canonical(text, &ctx).unwrap()
}

fn back(text: &str) -> String {
    back_from(text, FROM)
}

fn back_from(text: &str, from: &str) -> String {
    let (path, index) = (EntryPath::from_relative(Path::new(from)).unwrap(), index());
    let ctx = ConvertCtx {
        path: &path,
        index: &index,
    };
    ObsidianAdapter::default()
        .from_canonical(text, &ctx)
        .unwrap()
}

// --- Wikilinks ---------------------------------------------------------------

#[test]
fn wikilink_becomes_a_clean_relative_link() {
    assert_eq!(
        to("See [[001-stack]].\n"),
        "See [001-stack](./adr/001-stack.md).\n"
    );
}

#[test]
fn clean_link_becomes_a_wikilink_again() {
    assert_eq!(
        back("See [001-stack](./adr/001-stack.md).\n"),
        "See [[001-stack]].\n"
    );
}

#[test]
fn alias_becomes_the_link_text() {
    assert_eq!(
        to("[[architecture|the archi]]\n"),
        "[the archi](./architecture.md)\n"
    );
}

#[test]
fn link_from_a_subfolder_goes_up() {
    assert_eq!(
        to_from("[[architecture]]\n", "adr/001-stack.md"),
        "[architecture](../architecture.md)\n"
    );
}

#[test]
fn spaces_are_encoded_in_the_href() {
    assert_eq!(
        to("[[Café Notes]]\n"),
        "[Café Notes](./Caf\u{e9}%20Notes.md)\n"
    );
}

#[test]
fn target_outside_the_pair_becomes_plain_text_with_a_comment() {
    assert_eq!(to("[[March 12]]\n"), "March 12<!--markoo:[[March 12]]-->\n");
}

#[test]
fn target_not_found_is_left_as_is() {
    assert_eq!(to("[[Nope]]\n"), "[[Nope]]\n");
}

#[test]
fn target_not_found_is_reported() {
    let (path, index) = (EntryPath::from_relative(Path::new(FROM)).unwrap(), index());
    let ctx = ConvertCtx {
        path: &path,
        index: &index,
    };
    ObsidianAdapter::default()
        .to_canonical("[[Nope]]\n", &ctx)
        .unwrap();
    assert_eq!(index.take_warnings().len(), 1);
}

#[test]
fn anchor_uses_a_github_slug_and_keeps_the_original() {
    assert_eq!(
        to("[[001-stack#Base de données]]\n"),
        "[001-stack](./adr/001-stack.md#base-de-données)<!--markoo:[[001-stack#Base de données]]-->\n"
    );
}

#[test]
fn block_reference_points_to_the_note_and_keeps_the_original() {
    assert_eq!(
        to("[[001-stack#^abc]]\n"),
        "[001-stack](./adr/001-stack.md)<!--markoo:[[001-stack#^abc]]-->\n"
    );
}

#[test]
fn non_default_spelling_keeps_the_original() {
    assert_eq!(
        to("[[adr/001-stack]]\n"),
        "[001-stack](./adr/001-stack.md)<!--markoo:[[adr/001-stack]]-->\n"
    );
}

#[test]
fn twin_names_use_the_full_path() {
    assert_eq!(back("[twin](./twin.md)\n"), "[[Projets/Clara/twin]]\n");
}

#[test]
fn escaped_pipe_in_a_table_is_an_alias() {
    assert_eq!(
        to("| [[architecture\\|the archi]] |\n"),
        "| [the archi](./architecture.md)<!--markoo:[[architecture\\|the archi]]--> |\n"
    );
}

// --- Embeds --------------------------------------------------------------------

#[test]
fn image_embed_points_to_the_assets_folder() {
    assert_eq!(to("![[schema.png]]\n"), "![schema](./assets/schema.png)\n");
}

#[test]
fn image_link_becomes_an_embed_again() {
    assert_eq!(
        back("![schema](./assets/schema.png)\n"),
        "![[schema.png]]\n"
    );
}

#[test]
fn image_size_becomes_an_img_tag() {
    assert_eq!(
        to("![[schema.png|300]]\n"),
        "<img src=\"./assets/schema.png\" width=\"300\"><!--markoo:![[schema.png|300]]-->\n"
    );
}

#[test]
fn note_embed_becomes_a_link_with_the_original() {
    assert_eq!(
        to("![[001-stack]]\n"),
        "[001-stack](./adr/001-stack.md)<!--markoo:![[001-stack]]-->\n"
    );
}

#[test]
fn markdown_image_in_the_vault_is_rewritten_to_the_assets_folder() {
    assert_eq!(
        to_from("![s](../../attachments/schema.png)\n", FROM),
        "![s](./assets/schema.png)<!--markoo:![s](../../attachments/schema.png)-->\n"
    );
}

#[test]
fn referenced_assets_are_listed() {
    let (path, index) = (EntryPath::from_relative(Path::new(FROM)).unwrap(), index());
    let ctx = ConvertCtx {
        path: &path,
        index: &index,
    };
    let canonical = to("![[schema.png]] ![[spec.pdf|300]]\n");
    let adapter = ObsidianAdapter::default();
    assert_eq!(
        adapter.referenced_assets(&canonical, &ctx),
        vec!["schema.png".to_owned(), "spec.pdf".to_owned()]
    );
}

// --- Things left alone ------------------------------------------------------------

#[test]
fn code_is_never_touched() {
    let text = "`[[architecture]]`\n\n```\n[[architecture]]\n> [!info]\n```\n";
    assert_eq!(to(text), text);
}

#[test]
fn obsidian_comments_are_left_alone() {
    let text = "%% [[architecture]] %%\n";
    assert_eq!(to(text), text);
}

#[test]
fn external_links_are_left_alone() {
    let text = "[site](https://example.com/a.md)\n";
    assert_eq!(back(text), text);
}

#[test]
fn links_with_a_title_are_left_alone() {
    let text = "[a](./architecture.md \"title\")\n";
    assert_eq!(back(text), text);
}

#[test]
fn links_to_unknown_files_stay_markdown() {
    let text = "[a](./nope.md)\n";
    assert_eq!(back(text), text);
}

#[test]
fn unusual_relative_spelling_stays_markdown() {
    let text = "[architecture](architecture.md)\n";
    assert_eq!(back(text), text);
}

// --- Markdown links written in the vault --------------------------------------------

#[test]
fn markdown_link_in_the_vault_stays_markdown() {
    let text = "[architecture](./architecture.md)\n";
    assert_eq!(back(&to(text)), text);
}

#[test]
fn markdown_links_setting_keeps_them_clean() {
    let (path, index) = (EntryPath::from_relative(Path::new(FROM)).unwrap(), index());
    let ctx = ConvertCtx {
        path: &path,
        index: &index,
    };
    let adapter = ObsidianAdapter {
        settings: VaultSettings {
            use_markdown_links: true,
            attachment_folder: None,
        },
        ..ObsidianAdapter::default()
    };
    let text = "[architecture](./architecture.md)\n";
    assert_eq!(adapter.from_canonical(text, &ctx).unwrap(), text);
}

#[test]
fn links_setting_keep_leaves_wikilinks() {
    let (path, index) = (EntryPath::from_relative(Path::new(FROM)).unwrap(), index());
    let ctx = ConvertCtx {
        path: &path,
        index: &index,
    };
    let mut adapter = ObsidianAdapter::default();
    adapter.options.convert_links = false;
    assert_eq!(
        adapter.to_canonical("[[architecture]]\n", &ctx).unwrap(),
        "[[architecture]]\n"
    );
}

// --- Links written from the vault root --------------------------------------------------

#[test]
fn link_from_the_vault_root_becomes_relative() {
    let text = "[stack](Projets/Clara/adr/001-stack.md)\n";
    assert_eq!(
        to(text),
        "[stack](./adr/001-stack.md)<!--markoo:[stack](Projets/Clara/adr/001-stack.md)-->\n"
    );
}

#[test]
fn link_from_the_vault_root_comes_back_as_written() {
    let text = "[stack](Projets/Clara/adr/001-stack.md)\n";
    assert_eq!(back(&to(text)), text);
}

#[test]
fn encoded_link_from_the_vault_root_is_found() {
    assert_eq!(
        to("[c](Projets/Clara/Caf%C3%A9%20Notes.md)\n"),
        "[c](./Café%20Notes.md)<!--markoo:[c](Projets/Clara/Caf%C3%A9%20Notes.md)-->\n"
    );
}

#[test]
fn anchor_of_a_link_from_the_vault_root_follows_github() {
    assert_eq!(
        to("[s](Projets/Clara/adr/001-stack.md#Mon%20Titre)\n"),
        "[s](./adr/001-stack.md#mon-titre)<!--markoo:[s](Projets/Clara/adr/001-stack.md#Mon%20Titre)-->\n"
    );
}

#[test]
fn link_from_the_vault_root_outside_the_pair_becomes_text() {
    assert_eq!(
        to("[march](Réunions/March%2012.md)\n"),
        "march<!--markoo:[march](Réunions/March%2012.md)-->\n"
    );
}

#[test]
fn embedded_note_from_the_vault_root_becomes_a_plain_link() {
    assert_eq!(
        to("![stack](Projets/Clara/adr/001-stack.md)\n"),
        "[stack](./adr/001-stack.md)<!--markoo:![stack](Projets/Clara/adr/001-stack.md)-->\n"
    );
}

#[test]
fn image_from_the_vault_root_goes_to_the_assets_folder() {
    assert_eq!(
        to("![](attachments/schema.png)\n"),
        "![](./assets/schema.png)<!--markoo:![](attachments/schema.png)-->\n"
    );
}

#[test]
fn link_to_a_file_from_the_vault_root_goes_to_the_assets_folder() {
    assert_eq!(
        to("[spec](attachments/spec.pdf#page=2)\n"),
        "[spec](./assets/spec.pdf#page=2)<!--markoo:[spec](attachments/spec.pdf#page=2)-->\n"
    );
}

#[test]
fn linked_file_is_listed_as_an_asset() {
    let (path, index) = (EntryPath::from_relative(Path::new(FROM)).unwrap(), index());
    let ctx = ConvertCtx {
        path: &path,
        index: &index,
    };
    assert_eq!(
        ObsidianAdapter::default().referenced_assets("[spec](./assets/spec.pdf#page=2)\n", &ctx),
        vec!["spec.pdf".to_owned()]
    );
}

#[test]
fn image_with_an_ambiguous_name_is_left_alone() {
    let text = "![logo](Media/a/logo.png)\n";
    assert_eq!(to(text), text);
}

#[test]
fn file_next_to_the_note_wins_over_the_vault_root() {
    let text = "[twin](sub/twin.md)\n";
    let (path, index) = (
        EntryPath::from_relative(Path::new(FROM)).unwrap(),
        NameIndex::for_tests(
            &[
                "Projets/Clara/architecture.md",
                "Projets/Clara/sub/twin.md",
                "sub/twin.md",
            ],
            &[],
            "Projets/Clara",
        ),
    );
    let ctx = ConvertCtx {
        path: &path,
        index: &index,
    };
    assert_eq!(
        ObsidianAdapter::default().to_canonical(text, &ctx).unwrap(),
        text
    );
}

#[test]
fn explicitly_relative_link_is_not_looked_up_from_the_root() {
    let text = "[x](./Réunions/March%2012.md)\n";
    assert_eq!(to(text), text);
}

#[test]
fn unknown_path_is_left_alone() {
    let text = "[x](Nowhere/note.md)\n";
    assert_eq!(to(text), text);
}

#[test]
fn edited_link_from_the_vault_root_wins_over_the_comment() {
    let edited =
        "[renamed](./adr/001-stack.md)<!--markoo:[stack](Projets/Clara/adr/001-stack.md)-->\n";
    assert_eq!(back(edited), "[[001-stack|renamed]]\n");
}

// --- Edits on the repo side -----------------------------------------------------------

#[test]
fn edited_link_text_wins_over_the_comment() {
    let edited = "[renamed](./adr/001-stack.md)<!--markoo:[[adr/001-stack]]-->\n";
    assert_eq!(back(edited), "[[001-stack|renamed]]\n");
}

#[test]
fn edited_plain_text_wins_over_the_comment() {
    assert_eq!(back("Other<!--markoo:[[March 12]]-->\n"), "Other\n");
}

// --- Callouts ---------------------------------------------------------------------------

#[test]
fn default_callout_becomes_an_alert() {
    assert_eq!(to("> [!warning]\n> Careful\n"), "> [!WARNING]\n> Careful\n");
}

#[test]
fn alert_becomes_a_callout() {
    assert_eq!(
        back("> [!WARNING]\n> Careful\n"),
        "> [!warning]\n> Careful\n"
    );
}

#[test]
fn other_callout_type_is_kept_in_a_comment() {
    assert_eq!(
        to("> [!info]\n> Hi\n"),
        "<!--markoo:callout info-->\n> [!NOTE]\n> Hi\n"
    );
}

#[test]
fn callout_with_a_title_is_left_alone() {
    let text = "> [!warning] Watch out\n> Careful\n";
    assert_eq!(to(text), text);
}

#[test]
fn unknown_callout_is_left_alone() {
    let text = "> [!example]\n> Sample\n";
    assert_eq!(to(text), text);
}

#[test]
fn foldable_callout_is_left_alone() {
    let text = "> [!note]-\n> Hidden\n";
    assert_eq!(to(text), text);
}

#[test]
fn stale_callout_comment_is_dropped() {
    assert_eq!(back("<!--markoo:callout info-->\n> [!TIP]\n"), "> [!tip]\n");
}

#[test]
fn callouts_setting_keep_leaves_them() {
    let (path, index) = (EntryPath::from_relative(Path::new(FROM)).unwrap(), index());
    let ctx = ConvertCtx {
        path: &path,
        index: &index,
    };
    let mut adapter = ObsidianAdapter::default();
    adapter.options.convert_callouts = false;
    assert_eq!(
        adapter.to_canonical("> [!info]\n", &ctx).unwrap(),
        "> [!info]\n"
    );
}

// --- Frontmatter -------------------------------------------------------------------------

#[test]
fn obsidian_keys_leave_the_repo_side() {
    let canonical = to("---\ntags: [a]\naliases: [b]\n---\n# T\n");
    assert!(
        canonical.starts_with("---\ntags: [a]\n---\n<!--markoo:frontmatter ")
            && !canonical.contains("\naliases")
    );
}

#[test]
fn dropped_keys_come_back_at_their_place() {
    let text = "---\naliases: [b]\ntags: [a]\npublish: true\nowner: x\n---\n# T\n";
    assert_eq!(back(&to(text)), text);
}

#[test]
fn multi_line_values_are_kept_whole() {
    let text = "---\ntags:\n  - a\n  - b\naliases:\n  - c\n  - d\ntitle: T\n---\nBody\n";
    assert_eq!(back(&to(text)), text);
}

#[test]
fn only_dropped_keys_removes_the_block_on_the_repo_side() {
    let canonical = to("---\naliases: [b]\n---\n# T\n");
    assert!(canonical.starts_with("<!--markoo:frontmatter "));
}

#[test]
fn only_dropped_keys_come_back_whole() {
    let text = "---\naliases: [b]\n---\n# T\n";
    assert_eq!(back(&to(text)), text);
}

#[test]
fn drop_all_mode_keeps_everything_for_the_way_back() {
    let (path, index) = (EntryPath::from_relative(Path::new(FROM)).unwrap(), index());
    let ctx = ConvertCtx {
        path: &path,
        index: &index,
    };
    let mut adapter = ObsidianAdapter::default();
    adapter.options.frontmatter_drop_all = true;
    let text = "---\ntags: [a]\n---\n# T\n";
    let canonical = adapter.to_canonical(text, &ctx).unwrap();
    assert_eq!(adapter.from_canonical(&canonical, &ctx).unwrap(), text);
}

#[test]
fn frontmatter_edit_on_the_repo_side_is_kept() {
    let canonical =
        to("---\ntags: [a]\naliases: [b]\n---\n# T\n").replace("tags: [a]", "tags: [z]");
    assert_eq!(back(&canonical), "---\ntags: [z]\naliases: [b]\n---\n# T\n");
}

#[test]
fn value_with_a_comment_closer_survives() {
    let text = "---\naliases: [a --> b]\n---\n# T\n";
    assert_eq!(back(&to(text)), text);
}

#[test]
fn file_without_frontmatter_is_untouched() {
    assert_eq!(to("# T\n---\nx\n"), "# T\n---\nx\n");
}

#[test]
fn unterminated_frontmatter_is_untouched() {
    assert_eq!(to("---\naliases: [a]\n"), "---\naliases: [a]\n");
}

// --- Vault settings -------------------------------------------------------------------------

#[test]
fn known_vaults_are_read_from_the_app_settings() {
    let text = r#"{"vaults":{"a":{"path":"/v/one","ts":1},"b":{"path":"/v/two"}}}"#;
    assert_eq!(
        parse_known_vaults(text),
        vec![PathBuf::from("/v/one"), PathBuf::from("/v/two")]
    );
}

#[test]
fn invalid_app_settings_mean_no_known_vault() {
    assert!(parse_known_vaults("not json").is_empty());
}

#[test]
fn vault_settings_are_read_from_app_json() {
    let fs = crate::fs::MemFs::new();
    fs.write_atomic(
        Path::new("/v/.obsidian/app.json"),
        br#"{"useMarkdownLinks":true,"attachmentFolderPath":"files"}"#,
    )
    .unwrap();
    assert_eq!(
        VaultSettings::read(&fs, Path::new("/v")),
        VaultSettings {
            use_markdown_links: true,
            attachment_folder: Some("files".into())
        }
    );
}

#[test]
fn vault_root_is_found_from_a_subfolder() {
    let fs = crate::fs::MemFs::new();
    fs.write_atomic(Path::new("/v/.obsidian/app.json"), b"{}")
        .unwrap();
    assert_eq!(
        find_vault_root(&fs, Path::new("/v/a/b")),
        Some(PathBuf::from("/v"))
    );
}

// --- Stability: the property that prevents sync loops -----------------------------------------

fn fragment() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("[[architecture]]".to_owned()),
        Just("[[001-stack]]".to_owned()),
        Just("[[adr/001-stack]]".to_owned()),
        Just("[[architecture|see it]]".to_owned()),
        Just("[[001-stack#Base de données]]".to_owned()),
        Just("[[001-stack#^block]]".to_owned()),
        Just("[[Café Notes]]".to_owned()),
        Just("[[March 12]]".to_owned()),
        Just("[[March 12|the meeting]]".to_owned()),
        Just("[[Nope]]".to_owned()),
        Just("[[twin]]".to_owned()),
        Just("![[schema.png]]".to_owned()),
        Just("![[schema.png|300]]".to_owned()),
        Just("![[schema.png|a schema]]".to_owned()),
        Just("![[001-stack]]".to_owned()),
        Just("![[missing.png]]".to_owned()),
        Just("[architecture](./architecture.md)".to_owned()),
        Just("[other](architecture.md)".to_owned()),
        Just("[site](https://example.com)".to_owned()),
        Just("![pic](../../attachments/schema.png)".to_owned()),
        Just("![pic](attachments/schema.png)".to_owned()),
        Just("[stack](Projets/Clara/adr/001-stack.md)".to_owned()),
        Just("[march](Réunions/March%2012.md#Sujet)".to_owned()),
        Just("![](Projets/Clara/adr/001-stack.md)".to_owned()),
        Just("[spec](attachments/spec.pdf#page=2)".to_owned()),
        Just("`[[architecture]]`".to_owned()),
        Just("%% [[architecture]] %%".to_owned()),
        Just("plain text".to_owned()),
        Just("**bold**".to_owned()),
        "[a-z]{1,8}",
    ]
}

fn line() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => prop::collection::vec(fragment(), 1..4).prop_map(|parts| parts.join(" ")),
        1 => Just("> [!note]".to_owned()),
        1 => Just("> [!info]".to_owned()),
        1 => Just("> [!WARNING]".to_owned()),
        1 => Just("> [!example]".to_owned()),
        1 => Just("# A heading".to_owned()),
        1 => Just("- item [[architecture]]".to_owned()),
        1 => Just(String::new()),
    ]
}

fn frontmatter() -> impl Strategy<Value = String> {
    prop_oneof![
        Just(String::new()),
        Just("---\ntags: [a]\n---\n".to_owned()),
        Just("---\naliases: [b]\n---\n".to_owned()),
        Just("---\naliases:\n  - x\ntags: [a]\ncssclasses: y\nowner: z\n---\n".to_owned()),
    ]
}

fn vault_file() -> impl Strategy<Value = String> {
    (frontmatter(), prop::collection::vec(line(), 0..8)).prop_map(|(front, lines)| {
        let body: String = lines.iter().map(|line| format!("{line}\n")).collect();
        front + &body
    })
}

proptest! {
    /// A vault file comes back byte for byte.
    #[test]
    fn vault_files_round_trip(file in vault_file()) {
        prop_assert_eq!(back(&to(&file)), file);
    }

    /// The same, for a note in a subfolder.
    #[test]
    fn vault_files_round_trip_from_a_subfolder(file in vault_file()) {
        let from = "adr/001-stack.md";
        prop_assert_eq!(back_from(&to_from(&file, from), from), file);
    }

    /// Canonical text written to the vault converts back to itself; otherwise
    /// every sync would see a change and write again.
    #[test]
    fn canonical_text_is_stable(file in vault_file()) {
        let canonical = to(&file);
        prop_assert_eq!(to(&back(&canonical)), canonical);
    }

    /// Whatever someone writes on the repo side settles after one conversion.
    #[test]
    fn repo_edits_settle(file in vault_file()) {
        let once = to(&back(&file));
        prop_assert_eq!(to(&back(&once)), once);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    /// Arbitrary text neither panics nor gets lost.
    #[test]
    fn arbitrary_text_round_trips(file in "[ -~\\n]{0,120}") {
        prop_assume!(!file.contains(MARKER_OPEN));
        prop_assert_eq!(back(&to(&file)), file);
    }
}

#[test]
fn slugs_match_github() {
    assert_eq!(github_slug("Base de données: v2!"), "base-de-données-v2");
}

#[test]
fn relative_paths_are_computed_both_ways() {
    assert_eq!(relative_to("a/b/c.md", "a/d.md"), "../d.md");
}

#[test]
fn links_cannot_escape_the_root() {
    assert_eq!(resolve_relative("a.md", "../../etc/passwd"), None);
}
