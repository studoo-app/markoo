# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.0-rc.7] - 2026-11-08

### Security

- The `zip` library used to read the updates is updated to 9.4.2. It fixes three advisories: unbounded symlink targets that could exhaust memory during extraction, HMAC authentication bypass on a premature end of file, and dual-header inconsistencies in zip archives.

### Changed

- The updater plugin is updated to 2.14.1, which brings Tauri to 2.12.5 and its window stack (wry 0.57).

## [1.0.0-rc.6] - 2026-11-08

### Added

- Projects: syncs are grouped by project, for example the project "Markoo" holds the syncs "documentation" and "skill". A project is named when its first sync is created; the next syncs pick it or name a new one. It is stored as `project` in each `[[pair]]` of `markoo.toml`, and the spelling already in use is kept regardless of case.
- The home screen shows the syncs project by project, each with **Sync project** and **Add a sync**. The creation wizard asks the project first.
- `markoo pair add --project <name>`, a project question in the interactive wizard, `markoo pair list` grouped by project, and `--project <name>` on `markoo sync` and `markoo status` to act on every sync of a project. `markoo pair set <pair> project <name>` moves a sync to another project.
- Pair listings of the API and MCP include each pair's project, and a whole project can be synced.
- Each sync has a stable id, stored as `id` in its `[[pair]]` of `markoo.toml`. It is generated from the project and the name when the sync is created (`markoo-documentation`) and never changes: the state, history, conflicts, locks and Git hooks are keyed on it. In a file written by hand it is optional, and a sync without one keeps its name as id, so existing configurations keep their state and hooks.
- Sync names are free text, from 1 to 64 characters on one line without `/`, such as "Technical documentation" or "Skill Claude (prod)". The creation wizard asks for one, and a sync can be renamed at any time in the General tab, with `markoo pair set <pair> name "…"`, `PATCH /pairs/{pair}/config` or the MCP `update_pair_config` tool, without losing its history, conflicts or hooks. The General tab shows the id.
- Two projects can each have a sync with the same name, such as "Documentation".
- The CLI, the API and MCP designate a sync by its id, by `Project/Name`, or by its name alone when no other project uses it; an ambiguous name is refused with the list of matching syncs. API and MCP pair listings give `id`, `name`, `label` (`Project/Name`) and `project`, and `POST /pairs` answers with the generated `id`.

### Changed

- Creating a pair now requires a project: `markoo pair add` needs `--project`, the app asks for one, and `POST /pairs` answers `400` without `project`. Syncs created before projects keep working without one and are listed after the projects.
- Sync names no longer need to be unique across all projects, only within their project, regardless of case. Names and project names cannot contain `/`.
- The `"pair"` fields of JSON outputs (`--json`, API, MCP: plans, status, job reports, conflicts, history) and the pairs listed by `GET /projects` are ids, not names. `markoo hook install` writes the id in the hook scripts.

### Known limits

- No Homebrew formula yet, and no app for Linux or Windows.
- No rollback from the history: files overwritten or deleted stay in Markoo's `backups` and `trash` folders.
- Updates are offered from published releases only, not from release candidates.

## [1.0.0-rc.5] - 2026-11-08

### Added

- Activity: a new section that lists the latest syncs that changed something or failed, across all your syncs, the newest first. Runs are grouped by day and can be filtered by sync; opening one shows that sync's history.
- Search field on the Syncs screen, by name or folder.
- Each sync card has quick actions: sync now, settings, and remove (asked to confirm; files stay where they are).

### Changed

- New interface: a sidebar (Syncs, Activity, Settings), serif titles, a cream palette, and rounded cards showing the folders, the last sync that changed something and a status label. Syncs are grouped by knowledge base type.
- The version is shown in the window title instead of next to the logo.
- Texts that show a status are darker in light mode, so they stay readable, and the card or field in use is outlined when you move with the keyboard.
- The README shows the overview as a picture, in light and dark versions.

### Known limits

- No Homebrew formula yet, and no app for Linux or Windows.
- No rollback from the history: files overwritten or deleted stay in Markoo's `backups` and `trash` folders.
- Updates are offered from published releases only, not from release candidates.

## [1.0.0-rc.4] - 2026-11-07

### Added

- The interface can be shown in French: choose the language in Settings. It is stored as `language` in the `[general]` section of `markoo.toml`.
- The version number is shown next to the logo.
- Markoo looks for a new version a few seconds after launch and shows a banner when one is out. Settings installs it, then restarts Markoo. Updates are signed with a key that the app checks before installing.

### Changed

- New logo, drawn from the menu bar icon: a dot above two arrows. New colors in the app, its icon and the command line banner.
- Each pair shows the Obsidian and Git logos, its folder names and a status dot. Screens say "Sync" instead of "Pairs".
- The header shows the Markoo wordmark.

### Known limits

- No Homebrew formula yet, and no app for Linux or Windows.
- No rollback from the history: files overwritten or deleted stay in Markoo's `backups` and `trash` folders.
- Updates are offered from published releases only, not from release candidates.

## [1.0.0-rc.3] - 2026-10-31

### Changed

- The command line archives for macOS are now signed and notarized too. A Mac has to reach Apple the first time it runs one, since a bare binary cannot carry its ticket.

### Known limits

- No Homebrew formula yet, and no app for Linux or Windows.
- No rollback from the history: files overwritten or deleted stay in Markoo's `backups` and `trash` folders.

## [1.0.0-rc.2] - 2026-10-31

### Changed

- The macOS app is now signed and notarized by Apple, so it opens without any workaround. The `markoo` command inside it is signed too.

### Known limits

- The command line archives for macOS are not signed: a copy downloaded on its own may be blocked, and the [user guide](documentation/en/10-user-guide.md) explains what to do.
- No Homebrew formula yet, and no app for Linux or Windows.
- No rollback from the history: files overwritten or deleted stay in Markoo's `backups` and `trash` folders.

## [1.0.0-rc.1] - 2026-10-31

First release candidate. Keep a backup of your notes, and start on a copy.

### Added

- **Sync engine**: 3-way merge between a knowledge base and a Git repository, in both directions or one way, with a plan shown before anything is written, atomic writes, backups and a trash folder. Markoo never commits or pushes.
- **Knowledge bases**: an Obsidian adapter (wikilinks, embeds, attachments, callouts, frontmatter) that round-trips losslessly, and plain Markdown folders.
- **Command line**: `markoo` to set up pairs, see the plan and the differences, sync and settle conflicts.
- **Daemon**: job queue, schedule, local HTTP API on `127.0.0.1` with a token, and a `launchd` service on macOS.
- **Git hooks**: `markoo hook install` and `uninstall`, which keep existing hooks and suggest lines for husky, lefthook and `core.hooksPath`.
- **MCP server**: tools and resources for Claude Code and other clients, over stdio and HTTP. Tools that write are marked as destructive, and changing the configuration is off by default.
- **Desktop app** for macOS: menu bar status, pairs, changes with diffs, three-column conflict view, creation wizard, filters and transformations with previews, triggers, global settings and history. A preview warns about files going to the knowledge base that Git does not track.
- **Releases**: the command line for macOS, Linux and Windows, and the app for macOS with the command included.

### Known limits

- The macOS app is not signed nor notarized: see the [user guide](documentation/en/10-user-guide.md) to open it.
- No Homebrew formula yet, and no app for Linux or Windows.
- No rollback from the history: files overwritten or deleted stay in Markoo's `backups` and `trash` folders.
- Not implemented, and ignored with a warning: `conflict` strategies other than `ask`, `filenames = "kebab"`, `strip_comments`, `inline_embeds`, `schedule.pause_on_battery`, merging frontmatter by key.

[Unreleased]: https://github.com/studoo-app/markoo/compare/v1.0.0-rc.7...HEAD
[1.0.0-rc.7]: https://github.com/studoo-app/markoo/compare/v1.0.0-rc.6...v1.0.0-rc.7
[1.0.0-rc.6]: https://github.com/studoo-app/markoo/compare/v1.0.0-rc.5...v1.0.0-rc.6
[1.0.0-rc.5]: https://github.com/studoo-app/markoo/compare/v1.0.0-rc.4...v1.0.0-rc.5
[1.0.0-rc.4]: https://github.com/studoo-app/markoo/compare/v1.0.0-rc.3...v1.0.0-rc.4
[1.0.0-rc.3]: https://github.com/studoo-app/markoo/compare/v1.0.0-rc.2...v1.0.0-rc.3
[1.0.0-rc.2]: https://github.com/studoo-app/markoo/compare/v1.0.0-rc.1...v1.0.0-rc.2
[1.0.0-rc.1]: https://github.com/studoo-app/markoo/releases/tag/v1.0.0-rc.1
