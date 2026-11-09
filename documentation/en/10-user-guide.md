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

Pairs are grouped by **project**: the project `Markoo`, for example, can hold a `documentation` pair and a `skill` pair. You name the project when you create its first pair; the next pairs pick it, or name a new one.

**In the app**, click **Add a sync** and follow the steps. The first one asks the project: name it for your first pair, then pick it from the list. **Read the last one before creating the pair**: it lists what will be copied and in which direction. It warns about files going to the KB that Git does not track in the repository: they may have nothing to do with the project, for example if an earlier pair left them there.

**From the command line**:

```bash
markoo pair add clara --project Clara --kb ~/Obsidian/Knowledge/Projects/Clara --repo ~/code/clara/docs
```

Without arguments, `markoo pair add` asks the same questions in the terminal, the project first.

Then look at the plan, which writes nothing:

```bash
markoo sync --pair clara --dry-run
```

If it is what you expect, sync:

```bash
markoo sync --pair clara
```

## Group pairs by project

Add the next pairs of a project with the same `--project`. The spelling already in use is kept, so `--project clara` joins `Clara`:

```bash
markoo pair add Skill --project Clara --kb ~/Obsidian/Knowledge/Skills/Clara --repo ~/code/clara/skill
```

`markoo pair list` shows the pairs project by project. To act on all of a project's pairs at once:

```bash
markoo status --project Clara
```

```bash
markoo sync --project Clara
```

In the app, the home screen shows one block per project, with **Sync project** and **Add a sync**. Pairs created before projects existed have none and are listed last; `markoo pair set clara project Clara` gives one, or moves a pair to another project.

## Name and rename a pair

A pair's name is free text, such as `"Documentation technique"` or `"Skill Claude (prod)"`, and two projects can each have a `Documentation` pair. Rename a pair whenever you like, in its **General** tab or with:

```bash
markoo pair set clara name "Documentation Clara"
```

Renaming keeps the history, the pending conflicts and the Git hooks. They are tied to the pair's **id**, written once when the pair is created and never changed: `clara` here, `clara-skill` for the `Skill` pair above, `markoo-documentation` for the `Documentation` pair of `Markoo`. `markoo pair show` and the **General** tab show it.

In a command, designate a pair by its id, by `Project/Name` (`"Clara/Documentation Clara"`), or by its name alone when no other project has a pair with that name; otherwise Markoo refuses and lists the matching pairs. **Use the id in scripts**: it does not change when the pair is renamed. See [Designate a pair](06-configuration.md#designate-a-pair).

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
| `backups/<id>/` | What Markoo overwrote, by sync |
| `trash/<id>/` | What Markoo deleted, by sync |
| `hooks.log` | Errors of Git hooks |
| `daemon.log` | Output of the `launchd` service |

### Revert a sync

The **History** tab of the app lists what each sync changed. Open a sync and choose **Revert this sync…**; on the command line:

```bash
markoo history clara
markoo revert clara 12
```

Markoo puts back the files this sync touched: an overwritten file gets its previous version back, a created file goes to the trash, a deleted file comes back and a moved file goes back to where it was. A file changed since is left alone, and reported. The next sync carries the old versions to the other side.

The revert is itself recorded in the history, with backups of its own, so it can be reverted in turn. Limits:

- Syncs recorded before the version that added reverting cannot be reverted; copy the files from the folders above.
- For a sync that merged a file on both sides, both earlier versions come back, and the next sync reports a conflict.
- In a one-way pair, reverting a copy to the target becomes a divergence if the pair protects its target; it is refused if the pair overwrites changes in its target, since the next sync would write the same files again.

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
