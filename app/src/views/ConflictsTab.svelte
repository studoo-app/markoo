<script lang="ts">
  import { api, errorText, type Conflict, type Conflicts, type Direction } from "../lib/api";

  let {
    name,
    direction,
    revision,
    onchange,
  }: { name: string; direction: Direction; revision: number; onchange: () => void } = $props();

  let all = $state<Conflicts | null>(null);
  let error = $state<string | null>(null);
  let current = $state<Conflict | null>(null);
  let result = $state("");
  let busy = $state(false);

  const oneWay = $derived(direction !== "both");

  $effect(() => {
    void revision;
    api
      .listConflicts(name)
      .then((value) => {
        all = value;
        // A refresh must not throw away the text being edited.
        if (current && !value.conflicts.some((issue) => issue.id === current!.id)) current = null;
      })
      .catch((failure) => (error = errorText(failure)));
  });

  async function pick(issue: Conflict) {
    current = issue;
    error = null;
    try {
      result = await api.conflictDraft(issue.id);
    } catch (failure) {
      result = issue.kb ?? issue.repo ?? "";
      error = errorText(failure);
    }
  }

  /** Keeping the source of a one-way pair is what brings an excluded file back. */
  const sourceSide = $derived(direction === "kb-to-repo" ? "kb" : "repo");

  async function settle(
    how: { keep?: string; content?: string; pushToSource?: boolean },
    id = current?.id,
  ) {
    if (!id) return;
    busy = true;
    try {
      await api.resolve(id, how);
      current = null;
      error = null;
      onchange();
    } catch (failure) {
      error = errorText(failure);
    } finally {
      busy = false;
    }
  }
</script>

{#if error}
  <p class="error" role="alert">{error}</p>
{/if}

{#if all === null}
  <p class="muted">Comparing both sides…</p>
{:else if current}
  <button class="back" type="button" onclick={() => (current = null)}>‹ All conflicts</button>
  <h2 class="path">{current.path}</h2>
  <div class="columns">
    {#each [["KB", current.kb], ["Base", current.base], ["Repo", current.repo]] as [title, text]}
      <section>
        <h3>{title}</h3>
        {#if text === null || text === undefined}
          <p class="muted">{title === "Base" ? "No common version" : "Deleted on this side"}</p>
        {:else}
          <pre>{text}</pre>
        {/if}
      </section>
    {/each}
  </div>

  {#if !oneWay}
    <label for="result">Result</label>
    <textarea id="result" class="mono" rows="10" bind:value={result} spellcheck="false"></textarea>
  {/if}

  <div class="row buttons">
    {#if oneWay}
      <button type="button" disabled={busy} onclick={() => settle({ pushToSource: true })}>
        Copy the target to the source
      </button>
      <button type="button" disabled={busy} onclick={() => settle({ keep: sourceSide })}>
        Keep the source
      </button>
      <button type="button" disabled={busy} onclick={() => settle({ keep: sourceSide === "kb" ? "repo" : "kb" })}>
        Keep the target, stop syncing this file
      </button>
    {:else}
      <button class="primary" type="button" disabled={busy} onclick={() => settle({ content: result })}>
        Use the result
      </button>
      <button type="button" disabled={busy} onclick={() => settle({ keep: "kb" })}>Keep the KB</button>
      <button type="button" disabled={busy} onclick={() => settle({ keep: "repo" })}>Keep the repo</button>
      <button type="button" disabled={busy} onclick={() => settle({ keep: "both" })}>Keep both</button>
    {/if}
  </div>
  <p class="muted hint">What gets overwritten is kept in Markoo's backups.</p>
{:else}
  {#if all.conflicts.length === 0}
    <p class="muted">Nothing to settle.</p>
  {:else}
    <ul>
      {#each all.conflicts as issue (issue.id)}
        <li>
          <button type="button" class="link path" onclick={() => pick(issue)}>{issue.path}</button>
          <span class="muted">{issue.kind.replace("-", " ")}</span>
        </li>
      {/each}
    </ul>
  {/if}
  {#if all.excluded.length > 0}
    <h2 class="excluded">Excluded from sync</h2>
    <ul>
      {#each all.excluded as item (item.id)}
        <li>
          <span class="muted path">{item.path}</span>
          <button type="button" disabled={busy} onclick={() => settle({ keep: sourceSide }, item.id)}>
            Sync it again
          </button>
        </li>
      {/each}
    </ul>
  {/if}
{/if}

<style>
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  li {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    padding: 4px 0;
  }
  .link {
    background: none;
    border: 0;
    padding: 0;
    text-align: left;
    text-decoration: underline;
    text-decoration-color: var(--line);
  }
  .back {
    border: 0;
    background: none;
    padding: 0;
    color: var(--muted);
    margin-bottom: 4px;
  }
  .columns {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 8px;
    margin: 10px 0 14px;
  }
  .columns pre {
    max-height: 240px;
    margin-top: 4px;
  }
  h3 {
    font-size: 12px;
    color: var(--muted);
  }
  .buttons {
    flex-wrap: wrap;
    margin-top: 10px;
  }
  .excluded {
    margin-top: 18px;
  }
</style>
