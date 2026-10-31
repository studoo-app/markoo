# AGENTS.md

Guidance for AI coding agents (Claude Code, Codex, Cursor…) working on **Markoo**.

## What Markoo is

Markoo syncs Markdown documentation between a **knowledge base (KB)** and a **local Git repository**, using a 3-way merge (KB, repo, and the *base* from the last sync). KB tools are supported through **adapters** (`KbAdapter` trait in `markoo-core::kb`); **Obsidian is the first adapter implemented**, others (Logseq, Foam, Dendron…) come later. It is written in **Rust** (Cargo workspace) with a **Tauri 2** desktop app. A sync can be triggered by an interval, a Git hook, a local HTTP API, an MCP server, the CLI, or the app.

The design lives in [`documentation/`](documentation/INDEX.md). **Read it before changing behaviour**; start with the architecture and the sync algorithm:

- [Architecture](documentation/en/02-architecture.md)
- [Sync algorithm](documentation/en/03-sync-algorithm.md) (classification table, sync directions, conflicts)
- [Transformations](documentation/en/04-transformations.md) (conversion pipeline, Obsidian rules)
- [Knowledge bases](documentation/en/09-knowledge-bases.md) (adapter model, supported KB types)
- [Configuration](documentation/en/06-configuration.md) (`markoo.toml`, per-OS paths)
- [Roadmap](documentation/en/07-roadmap.md) (milestones M0–M8)

## Current status

**M0 to M2, M4 to M6 done** (`markoo-daemon`: job queue, scheduler, HTTP API, `launchd` service, tried on a real `launchd`), **M3 Obsidian adapter in place** (`markoo-core::kb::obsidian`): wikilinks, embeds, attachments, callouts, frontmatter, detection; validation on real vaults under way (`MARKOO_CORPUS`, see `crates/markoo-core/tests/obsidian.rs`). Not implemented yet: rollback from the history, `schedule.pause_on_battery`, the `conflict` strategies other than `ask`, `filenames = "kebab"`, `strip_comments`, `inline_embeds`, merging frontmatter by key (all accepted in `markoo.toml`, ignored with a warning where they are options). **M7 done on macOS** (`app/`, Svelte 5 and Tauri 2, its own lock file outside the Cargo workspace): every screen is in place. **M8 in progress**: release candidates, release workflow in `.github/workflows/release.yml`, signed and notarized on macOS through the Apple secrets; Homebrew is postponed. Don't build ahead of the roadmap without being asked.

## Repository layout (target)

```
markoo/
├── Cargo.toml              # workspace
├── crates/
│   ├── markoo-core/        # sync engine library — all the logic lives here
│   ├── markoo-cli/         # `markoo` binary (clap)
│   ├── markoo-daemon/      # scheduler + HTTP API (axum) + MCP host
│   └── markoo-mcp/         # MCP tool definitions (rmcp)
├── app/                    # Tauri 2 app
├── documentation/          # bilingual design docs (fr/ + en/)
└── tests/fixtures/         # sample knowledge bases (per adapter) and repos
```

## Invariants — never break these

1. **Never lose user data.** Anything Markoo overwrites goes to `<data>/backups/`, anything it deletes goes to `<data>/trash/`. Never hard-delete a user file.
2. **Plan before applying.** Every sync first builds a serializable `Plan`; dry-run, the app, the API and MCP all display that same plan. Writing only happens in the `applier`.
3. **Atomic writes.** Write to `*.markoo-tmp`, `fsync`, then `rename`. Re-check the target's state right before writing; if it changed since the scan, skip it and replan later.
4. **Idempotence.** Re-running a sync with no changes must produce zero writes. Every new case needs a test for this.
5. **Reversible transformations.** Every adapter must round-trip losslessly: `from_canonical(to_canonical(x)) == x` for every KB file, and `to_canonical` must be stable on what `from_canonical` produces, or files look changed at every sync. The canonical form is GitHub Flavored Markdown with relative links. If a construct can't round-trip, leave it untouched (or keep the lost information in a `<!--markoo:…-->` comment).
6. **Conflicts are never resolved silently.** In `both` mode, overlapping changes wait for the user; in one-way modes with `target_changes = "protect"`, edits to the target become *divergences*.
7. **KB-specific code lives in its adapter.** The engine (scanner, planner, merge, applier) never mentions Obsidian or any other tool; it only talks to `KbAdapter`.
8. **One logic, many entry points.** The CLI, daemon, API, MCP and Tauri app are thin layers over `markoo-core`. Don't put sync logic in them.
9. **`markoo-core` does no network I/O.** It works on files through an `Fs` trait so it can be tested in memory.
10. **No hard-coded paths.** Only `markoo-core::paths` (crate `directories`) resolves `<config>` and `<data>`. The Tauri app uses it too, never `app_data_dir()`.
11. **Local only.** The HTTP API binds to `127.0.0.1`, never `0.0.0.0`. Paths received through the API or MCP must be normalized and confined to the pair's folders.
12. **Markoo never commits or pushes** to the user's repository.

## Rust conventions

- Stable toolchain, edition 2024.
- Before finishing any change:

  ```bash
  cargo fmt --all
  ```

  ```bash
  cargo clippy --workspace --all-targets -- -D warnings
  ```

  ```bash
  cargo test --workspace
  ```

- For the app: `npm run check` in `app/`, then `cargo fmt`, `cargo clippy --all-targets -- -D warnings` and `cargo test` in `app/src-tauri/`.
- Errors: `thiserror` in library crates, `anyhow` only in binaries.
- No `unwrap()` / `expect()` outside tests, except for invariants that are truly impossible to break (explain why in a comment).
- Tests:
  - each row of the [classification table](documentation/en/03-sync-algorithm.md#classification) has a test, for each sync direction;
  - each adapter has golden files in `tests/fixtures/transform/<kb_type>/` and `proptest` round-trip tests;
  - tests never touch the real `<config>` / `<data>` folders: set `MARKOO_CONFIG` / `MARKOO_DATA_DIR` to a temp dir.
- Code, identifiers, comments, logs and error messages are in **English**.

## Documentation rules

- The documentation is **bilingual**: `documentation/fr/` and `documentation/en/` contain the same files with the same numbering (see the mapping in [`documentation/INDEX.md`](documentation/INDEX.md)).
- **Any change to one language must be mirrored in the other in the same commit**: same headings, same tables, same diagrams.
- Each page starts with its H1, then a link to its translation (`> 🇬🇧 [English version](../en/…)` / `> 🇫🇷 [Version française](../fr/…)`).
- Diagrams use **Mermaid** (flowchart, sequence, state, ER, gantt, mindmap). Keep labels in double quotes, use ASCII node IDs.
- Before finishing a docs change, check that:
  - every Mermaid block parses (e.g. `mermaid.parse()` from the `mermaid` npm package);
  - every relative link and `#anchor` points to an existing file and heading (GitHub slug rules);
  - the FR and EN versions of each changed file have the same number of headings, Mermaid blocks and table rows.
- If code behaviour diverges from the design docs, update the docs (both languages) in the same change, or ask.

## Skills

Project skills live in `.agents/skills/` (linked from `.claude/skills/` and `.vibe/skills/`, pinned in `skills-lock.json`): `rust-best-practices`, `tauri-v2`, `semver`, `semver-changelog`.

- **This file wins over any skill.** Skills give generic advice; when one conflicts with the invariants or conventions above, follow AGENTS.md.
- Known conflict: `tauri-v2` recommends Tauri's path APIs (`app.path()`). For Markoo's config and state, always use `markoo-core::paths` instead (invariant 10). Tauri's path APIs are fine only for Tauri-owned files (WebView cache, window state).

## Git

- Commit messages, PR titles and PR descriptions are **always in English**, whatever the language of the conversation or of the docs being changed.
- Use Conventional Commits prefixes (`feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`…).

## Working with the user

- Configuration file keys and enum values are part of the public interface (`kb`, `kb_type`, `direction`, `target_changes`, `both`, `kb-to-repo`…): don't rename them without asking.
- Ask before adding a dependency that isn't already named in the docs.
