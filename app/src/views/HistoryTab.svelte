<script lang="ts">
  import { api, errorText, type RunRecord } from "../lib/api";

  let { name, revision }: { name: string; revision: number } = $props();

  let runs = $state<RunRecord[] | null>(null);
  let error = $state<string | null>(null);

  $effect(() => {
    void revision;
    api
      .pairHistory(name)
      .then((value) => {
        runs = value;
        error = null;
      })
      .catch((failure) => (error = errorText(failure)));
  });

  const triggers: Record<string, string> = {
    startup: "at startup",
    interval: "on schedule",
    config: "after a settings change",
    api: "from the API",
    mcp: "from an MCP client",
    app: "from the app",
    hook: "from a Git hook",
    cli: "from the command line",
    resolve: "settling a conflict",
  };

  function summary(run: RunRecord): string {
    const report = run.report;
    if (!report) return "";
    const toRepo = report.written.filter((file) => file.side === "repo").length;
    const toKb = report.written.filter((file) => file.side === "kb").length;
    const parts = [];
    if (toRepo > 0) parts.push(`${toRepo} to the repo`);
    if (toKb > 0) parts.push(`${toKb} to the KB`);
    if (report.trashed.length > 0) parts.push(`${report.trashed.length} to the trash`);
    if (report.skipped.length > 0) parts.push(`${report.skipped.length} skipped`);
    return parts.join(" · ");
  }

  const when = (run: RunRecord) => new Date(run.started_at).toLocaleString();
</script>

{#if error}
  <p class="error" role="alert">{error}</p>
{:else if runs === null}
  <p class="muted">Loading…</p>
{:else if runs.length === 0}
  <p class="muted">No sync has changed anything yet.</p>
{:else}
  <ul>
    {#each runs as run (run.id)}
      <li class="card">
        <details>
          <summary>
            <span class="muted">{when(run)}</span>
            <span class="muted">{triggers[run.trigger] ?? run.trigger}</span>
            {#if run.error}
              <span class="error">Failed</span>
            {:else}
              <strong>{summary(run)}</strong>
            {/if}
          </summary>
          {#if run.error}
            <p class="error path">{run.error}</p>
          {/if}
          {#if run.report}
            <ul class="files">
              {#each run.report.written as file}
                <li class="path">{file.side === "repo" ? "→" : "←"} {file.path}</li>
              {/each}
              {#each run.report.trashed as file}
                <li class="path">🗑 {file.path}</li>
              {/each}
              {#each run.report.skipped as file}
                <li class="path warn">⚠ {file.path}: {file.reason}</li>
              {/each}
            </ul>
          {/if}
          <p class="muted hint">Took {run.duration_ms} ms.</p>
        </details>
      </li>
    {/each}
  </ul>
  <p class="muted hint">The latest 100 syncs that changed something, or failed.</p>
{/if}

<style>
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  li.card {
    margin-bottom: 6px;
    padding: 8px 12px;
  }
  summary {
    cursor: pointer;
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
    align-items: baseline;
  }
  .files {
    margin-top: 8px;
    font-size: 13px;
  }
  details p {
    margin: 6px 0 0;
  }
</style>
