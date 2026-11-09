<script lang="ts">
  import { locale, t } from "../lib/i18n.svelte";
  import { api, errorText, type RunRecord } from "../lib/api";

  let { id, revision, onchange }: { id: string; revision: number; onchange: () => void } =
    $props();

  let runs = $state<RunRecord[] | null>(null);
  let error = $state<string | null>(null);
  /** The sync whose revert waits for confirmation. */
  let confirming = $state<number | null>(null);
  let reverting = $state(false);
  let revertError = $state<string | null>(null);
  /** What the last revert did, shown under its sync. */
  let reverted = $state<{ run: number; done: number; skipped: { path: string; reason: string }[] } | null>(null);

  $effect(() => {
    void revision;
    api
      .pairHistory(id)
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
    revert: "reverting a sync",
  };

  /** Ids of the syncs a later revert put back. */
  const revertedRuns = $derived(
    new Set(
      (runs ?? [])
        .filter((run) => run.report?.reverts !== undefined && run.report.run !== undefined)
        .map((run) => run.report!.reverts!),
    ),
  );

  const canRevert = (run: RunRecord) =>
    run.report?.run !== undefined && !revertedRuns.has(run.id);

  const changedFiles = (run: RunRecord) =>
    (run.report?.written.length ?? 0) + (run.report?.trashed.length ?? 0);

  async function revert(run: RunRecord) {
    reverting = true;
    try {
      const result = await api.revertRun(id, run.id);
      reverted = {
        run: run.id,
        done: result.report.written.length + result.report.trashed.length,
        skipped: result.report.skipped,
      };
      revertError = null;
      confirming = null;
      onchange();
    } catch (failure) {
      revertError = errorText(failure);
    } finally {
      reverting = false;
    }
  }

  function summary(run: RunRecord): string {
    const report = run.report;
    if (!report) return "";
    const toRepo = report.written.filter((file) => file.side === "repo").length;
    const toKb = report.written.filter((file) => file.side === "kb").length;
    const parts = [];
    if (toRepo > 0) parts.push(t("{count} to the repo", { count: toRepo }));
    if (toKb > 0) parts.push(t("{count} to the KB", { count: toKb }));
    if (report.trashed.length > 0) parts.push(t("{count} to the trash", { count: report.trashed.length }));
    if (report.skipped.length > 0) parts.push(t("{count} skipped", { count: report.skipped.length }));
    return parts.join(" · ");
  }

  const when = (run: RunRecord) => new Date(run.started_at).toLocaleString(locale());
</script>

{#if error}
  <p class="error" role="alert">{error}</p>
{:else if runs === null}
  <p class="muted">{t("Loading…")}</p>
{:else if runs.length === 0}
  <p class="muted">{t("No sync has changed anything yet.")}</p>
{:else}
  <ul>
    {#each runs as run (run.id)}
      <li class="card">
        <details>
          <summary>
            <span class="muted">{when(run)}</span>
            <span class="muted">{t(triggers[run.trigger] ?? run.trigger)}</span>
            {#if run.error}
              <span class="error">{t("Failed")}</span>
            {:else}
              <strong>{summary(run)}</strong>
            {/if}
            {#if revertedRuns.has(run.id)}
              <span class="muted">{t("Reverted")}</span>
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
          <p class="muted hint">{t("Took {ms} ms.", { ms: run.duration_ms })}</p>
          {#if reverted?.run === run.id}
            <p class="ok" role="status">
              {t("{count} file(s) put back.", { count: reverted.done })}
            </p>
            {#each reverted.skipped as file}
              <p class="path warn">⚠ {file.path}: {file.reason}</p>
            {/each}
          {:else if confirming === run.id}
            <div class="card confirm">
              <p>
                {t("Put back the {count} file(s) this sync changed? A file changed since is left alone.", {
                  count: changedFiles(run),
                })}
              </p>
              <p class="muted hint">
                {t("The next sync carries the old versions to the other side. What is replaced goes to the backups, so this can be reverted too.")}
              </p>
              {#if revertError}
                <p class="error" role="alert">{revertError}</p>
              {/if}
              <div class="row end">
                <button type="button" onclick={() => ((confirming = null), (revertError = null))}>
                  {t("Cancel")}
                </button>
                <button class="danger" type="button" disabled={reverting} onclick={() => revert(run)}>
                  {t("Revert")}
                </button>
              </div>
            </div>
          {:else if canRevert(run)}
            <div class="row end">
              <button type="button" onclick={() => ((confirming = run.id), (revertError = null))}>
                {t("Revert this sync…")}
              </button>
            </div>
          {/if}
        </details>
      </li>
    {/each}
  </ul>
  <p class="muted hint">{t("The latest 100 syncs that changed something, or failed.")}</p>
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
  .confirm {
    margin-top: 8px;
    padding: 8px 12px;
  }
  .row.end {
    justify-content: flex-end;
    margin-top: 8px;
  }
</style>
