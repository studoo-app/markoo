# 10. User guide

> 🇫🇷 [Version française](../fr/10-guide-utilisateur.md)

Markoo keeps a folder of your knowledge base (KB) and a folder of a Git repository in sync. This guide covers daily use; the other pages explain how it works.

> **Keep a backup of your notes, and start on a copy.** Markoo is a release candidate. It keeps what it overwrites, but a mistake in the choice of folders copies a lot of files.

## Install

### Desktop app (macOS)

Download the `.dmg` for your Mac from the [releases](https://github.com/studoo-app/markoo/releases) (`aarch64` for Apple Silicon, `x64` for Intel) and drag Markoo to Applications.

The app is signed and notarized by Apple, so it opens normally. It includes the `markoo` command, in `/Applications/Markoo.app/Contents/MacOS/markoo`.

### Command line only

Download the archive for your system from the same page and put `markoo` somewhere in your `PATH`. It runs on macOS, Linux and Windows. On macOS it is signed and notarized too, but a Mac has to reach Apple the first time it runs it, to check. If the system blocks it, offline for example, use the copy inside the app or remove the quarantine mark:

```bash
xattr -d com.apple.quarantine ./markoo
```

To build it from the sources instead, with Rust installed:

```bash
cargo install --path crates/markoo-cli --locked
```

## Create your first pair

A **pair** links one KB folder to one repository folder. Pick narrow folders: a pair tracks everything under them.

**In the app**, click **New pair** and follow the steps. **Read the last one before creating the pair**: it lists what will be copied and in which direction. It warns about files going to the KB that Git does not track in the repository: they may have nothing to do with the project, for example if an earlier pair left them there.

**From the command line**:

```bash
markoo pair add clara --kb ~/Obsidian/Knowledge/Projects/Clara --repo ~/code/clara/docs
```

Then look at the plan, which writes nothing:

```bash
markoo sync --pair clara --dry-run
```

If it is what you expect, sync:

```bash
markoo sync --pair clara
```

> Everything under the KB folder ends up in the repository, including attachments and private notes. Use `exclude` patterns, in the **Filters** tab or with `markoo pair set`, before syncing towards a shared repository.

## Choose a direction

| Direction | Value | Use it when |
|-----------|-------|-------------|
| KB ⇄ Repo | `both` | You write on both sides |
| KB → Repo | `kb-to-repo` | The KB is the reference |
| KB ← Repo | `repo-to-kb` | The repository is the reference |

In a one-way pair, a change in the target is reported as a **divergence** and left alone. See [Sync algorithm](03-sync-algorithm.md).

## Settle conflicts

When both sides changed the same lines, Markoo writes nothing for that file and waits for you. The menu bar icon turns orange.

**In the app**, open the pair, then the **Conflicts** tab: it shows the KB, base and repository versions, and a text area for the result.

**From the command line**, list them, then settle each one:

```bash
markoo conflicts
```

```bash
markoo resolve architecture.md --keep kb
```

`--keep` takes `kb`, `repo` or `both`, and `--edit` opens the merged text in your editor.

## Sync automatically

| Way | How | Note |
|-----|-----|------|
| App | Leave it running in the menu bar | Closing the window does not stop it; **Quit Markoo** does |
| Service (macOS) | `markoo daemon install` | Without the app; `markoo daemon uninstall` removes it |
| Git hooks | **Triggers** tab, or `markoo hook install --pair clara` | Installing syncs nothing: a hook acts the next time Git runs it |
| MCP | `claude mcp add markoo -- markoo mcp` | Lets Claude Code or another client use Markoo |

The interval is set in the app's **Settings**, or in `[schedule]` of `markoo.toml`. Run `markoo hook uninstall` before removing a pair, or the hooks stay in the repository. See [Triggers](05-triggers.md).

> With MCP, conflict contents and diffs are sent to the model you use, so the text of your notes leaves your machine if that model is remote.

## Find your files

`markoo paths` shows the configuration file and the data folder in use. On macOS, both are in `~/Library/Application Support/dev.markoo.markoo/`.

| In the data folder | Content |
|--------------------|---------|
| `backups/<pair>/` | What Markoo overwrote, by sync |
| `trash/<pair>/` | What Markoo deleted, by sync |
| `hooks.log` | Errors of Git hooks |
| `daemon.log` | Output of the `launchd` service |

**There is no automatic rollback yet.** To get a file back, copy it from these folders. The **History** tab of the app lists what each sync changed.

## Stop using Markoo

Removing a pair keeps every file on both sides, and files it copied stay where they are. **Remove the pair before deleting its files**: otherwise the next sync takes the deletion for a change and carries it over to the other side.

1. Run `markoo hook uninstall --pair clara` if you installed hooks.
2. Run `markoo pair remove clara`, or use the **General** tab.
3. Run `markoo daemon uninstall` if you installed the service.
4. Remove the app, then the folder given by `markoo paths`.

## Known limits

- The app runs on macOS only.
- There is no Homebrew formula yet.
- The interval applies to every pair.
- Options that are not implemented are accepted in `markoo.toml` and ignored with a warning.

The [changelog](../../CHANGELOG.md) lists them for each version.
