# 5. Triggers

> 🇫🇷 [Version française](../fr/05-declencheurs.md)

A sync can be started in four ways, plus the manual CLI. All of them go through the daemon's **single job queue**: two simultaneous triggers on the same pair never run two syncs in parallel.

```mermaid
flowchart LR
    I["⏱️ Interval"] --> Q
    H["🔧 Git hook"] --> Q
    A["🌐 HTTP API"] --> Q
    M["🤖 MCP"] --> Q
    C["⌨️ CLI / app"] --> Q
    Q["Job queue<br/>+ per-pair lock<br/>+ debounce"] --> E["markoo-core<br/>plan → apply"]
    E --> EV["Events<br/>(app, API SSE, notifications)"]
```

**Debounce**: requests received while a sync is running are merged into a single extra sync, started once the current one finishes.

## 1. Time interval

```toml
[schedule]
interval = "5m"        # 30s, 5m, 1h… ; "off" to disable
on_startup = true      # sync when the daemon starts
pause_on_battery = false
```

- The `tokio` scheduler syncs every pair at each interval.
- The fast comparison (`mtime` + size) makes a no-op sync almost free, so short intervals are fine.
- **Planned**: a `watch` mode based on file system events (`notify`, FSEvents on macOS), with a debounce of a few seconds.

## 2. Git hooks

```bash
markoo hook install --pair clara
```

Installs hooks in the pair's repository that call Markoo:

| Hook | When | Purpose |
|------|------|---------|
| `post-merge` | after `git pull` / `git merge` | bring docs changed by the team into the KB |
| `post-checkout` | after switching branches | align the KB with the current branch (optional) |
| `post-commit` | after a local commit | usually nothing new to propagate; acts as a checkpoint |
| `pre-commit` | before a commit | **check** that no Markoo conflict remains (optional, blocking) |

```mermaid
sequenceDiagram
    participant U as User
    participant G as Git
    participant H as post-merge hook
    participant D as Markoo daemon
    participant V as KB

    U->>G: git pull
    G->>G: merge remote commits
    G->>H: run the hook
    H->>D: POST /sync?pair=clara (or CLI if the daemon is stopped)
    D->>V: update the changed notes (through the KB adapter)
    D-->>U: notification “3 notes updated”
```

- The hook **never fails** because of Markoo (except `pre-commit`, if enabled): errors are logged and Git carries on.
- Existing hooks are preserved: Markoo appends itself at the end, between two `# >>> markoo` / `# <<< markoo` markers. The `husky` and `lefthook` managers are detected, and a configuration snippet is suggested instead.
- Branches can be filtered (`hook_branches = ["main"]`) to sync only the main branch.

## 3. Local HTTP API

Served by `axum`, on `127.0.0.1` only (default port `4848`). Authentication uses a random token stored in `<data>/token` (`Authorization: Bearer …` header).

| Method | Route | Purpose |
|--------|-------|---------|
| `GET` | `/health` | daemon status |
| `GET` | `/pairs` | list of configured pairs |
| `GET` | `/pairs/{pair}/status` | current plan (nothing applied) |
| `GET` | `/pairs/{pair}/diff?path=…` | diff of one file |
| `POST` | `/pairs/{pair}/sync` | start a sync (`{"dry_run": false}`) → `202` + job id |
| `GET` | `/jobs/{id}` | status and report of a job |
| `POST` | `/pairs` | create a pair |
| `GET` | `/pairs/{pair}/config` | pair configuration |
| `PATCH` | `/pairs/{pair}/config` | update the pair (direction, filters…); returns the plan preview |
| `DELETE` | `/pairs/{pair}` | delete the pair (files are left untouched) |
| `GET` | `/conflicts` | pending conflicts and divergences |
| `POST` | `/conflicts/{id}/resolve` | `{"keep": "kb" \| "repo" \| "both"}` or `{"content": "…"}` |
| `GET` | `/events` | SSE stream: progress, job completion, new conflicts |

Sample report:

```json
{
  "job": "j_01JB…",
  "pair": "clara",
  "kb_type": "obsidian",
  "dry_run": false,
  "summary": { "kb_to_repo": 2, "repo_to_kb": 1, "merged": 1, "conflicts": 1, "deleted": 0 },
  "conflicts": [{ "id": "c_42", "path": "architecture.md", "hunks": 1 }],
  "duration_ms": 184
}
```

## 4. MCP server

Markoo exposes a **Model Context Protocol** server (`rmcp` crate), usable from Claude Code, Claude Desktop or any MCP client.

- **stdio**: `markoo mcp` (the client spawns the process).
- **HTTP**: `http://127.0.0.1:4848/mcp` while the daemon is running.

```json
{
  "mcpServers": {
    "markoo": { "command": "markoo", "args": ["mcp"] }
  }
}
```

### Tools

| Tool | Parameters | Effect |
|------|-----------|--------|
| `list_pairs` | — | Configured pairs, KB type (e.g. `obsidian`) and status |
| `sync_status` | `pair?` | Current plan: what would be synced |
| `sync_diff` | `pair`, `path` | Unified diff of one file (KB ↔ repo, in canonical form) |
| `sync_run` | `pair?`, `dry_run?` | Starts a sync, returns the report |
| `list_conflicts` | `pair?` | Pending conflicts, with all three versions |
| `resolve_conflict` | `id`, `keep?` \| `content?` | Resolves a conflict or a divergence |
| `get_pair_config` | `pair` | Pair configuration (KB type, direction, filters…) |
| `update_pair_config` | `pair`, `changes` | Updates the pair; **disabled** unless `mcp.allow_config_write = true` |

### Resources

- `markoo://pairs/{pair}/status`: current status.
- `markoo://conflicts/{id}`: KB, base and repo versions of a conflict.

```mermaid
sequenceDiagram
    participant U as User
    participant C as Claude
    participant M as markoo mcp
    participant E as markoo-core

    U->>C: “Sync the Clara docs and summarize the changes”
    C->>M: sync_status(pair="clara")
    M->>E: plan()
    E-->>M: 3 changes, 1 conflict
    M-->>C: plan
    C->>M: list_conflicts(pair="clara")
    M-->>C: KB / base / repo of architecture.md
    C->>U: “Conflict on architecture.md: I suggest this merge…”
    U->>C: “OK”
    C->>M: resolve_conflict(id="c_42", content="…")
    C->>M: sync_run(pair="clara")
    M-->>C: report
    C->>U: summary of the changes
```

> **Safeguard**: the tools that write (`sync_run`, `resolve_conflict`) are annotated as *destructive* in MCP, which lets the client ask the user for confirmation.

## Shared security

- Listens on `127.0.0.1` only, never on `0.0.0.0`.
- Regenerable API token (`markoo token rotate`).
- Incoming paths (API, MCP) are normalized and **confined** to the pair: no reads or writes outside the configured folders (protection against `../`).
