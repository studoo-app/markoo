<script lang="ts">
  import { api, errorText, type PairView } from "../lib/api";
  import PairFlow from "../lib/PairFlow.svelte";

  let {
    revision,
    onopen,
    onnew,
  }: { revision: number; onopen: (pair: string) => void; onnew: () => void } = $props();

  let pairs = $state<PairView[] | null>(null);
  let error = $state<string | null>(null);
  let syncing = $state<string | null>(null);
  // Start time of the latest run of each pair, in milliseconds.
  let lastRuns = $state<Record<string, number>>({});

  async function load() {
    try {
      const listed = await api.listPairs();
      pairs = listed;
      error = null;
      const entries = await Promise.all(
        listed.map(async (pair): Promise<[string, number] | null> => {
          try {
            const runs = await api.pairHistory(pair.name);
            return runs.length > 0 ? [pair.name, runs[0].started_at] : null;
          } catch {
            return null;
          }
        }),
      );
      lastRuns = Object.fromEntries(entries.filter((entry) => entry !== null));
    } catch (failure) {
      error = errorText(failure);
    }
  }

  function when(milliseconds: number): string {
    return new Date(milliseconds).toLocaleString(undefined, {
      dateStyle: "short",
      timeStyle: "short",
    });
  }

  $effect(() => {
    void revision;
    load();
  });

  async function sync(pair: string) {
    syncing = pair;
    try {
      await api.syncPair(pair);
      error = null;
    } catch (failure) {
      error = errorText(failure);
    } finally {
      syncing = null;
      await load();
    }
  }

  function status(pair: PairView): { text: string; tone: string } {
    if (pair.errors.length > 0) return { text: pair.errors.join("; "), tone: "error" };
    if (pair.active) return { text: "Syncing", tone: "muted" };
    if (pair.pending > 0) return { text: `${pair.pending} to settle`, tone: "warn" };
    if (pair.paused) return { text: "Paused", tone: "muted" };
    return { text: "No pending conflict", tone: "ok" };
  }
</script>

<div class="row spread title">
  <h1>Sync</h1>
  {#if pairs?.length}
    <button class="primary" type="button" onclick={onnew}>+ New sync</button>
  {/if}
</div>

{#if error}
  <p class="error" role="alert">{error}</p>
{/if}

{#if pairs === null}
  <p class="muted">Loading…</p>
{:else if pairs.length === 0}
  <div class="card empty">
    <h2>No sync yet</h2>
    <p class="muted">A sync keeps a folder of your knowledge base in sync with a folder of a Git repository.</p>
    <button class="primary" type="button" onclick={onnew}>Create the first one</button>
  </div>
{:else}
  <ul>
    {#each pairs as pair (pair.name)}
      {@const state = status(pair)}
      <li class="card" class:paused={pair.paused}>
        <button class="open" type="button" onclick={() => onopen(pair.name)}>
          <span class="flow-line"><PairFlow kbType={pair.kb_type} direction={pair.direction} kb={pair.kb} repo={pair.repo} /></span>
          <span class="status-line">
            <span class={state.tone}><span class="dot" aria-hidden="true"></span>{state.text}</span>
            {#if lastRuns[pair.name] !== undefined}
              <span class="muted">· Last sync {when(lastRuns[pair.name])}</span>
            {/if}
          </span>
        </button>
        <button
          type="button"
          disabled={pair.paused || pair.active || syncing !== null || pair.errors.length > 0}
          onclick={() => sync(pair.name)}
        >
          {syncing === pair.name ? "Syncing…" : "Sync now"}
        </button>
      </li>
    {/each}
  </ul>
{/if}

<style>
  .title {
    margin-bottom: 16px;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 8px;
  }
  li {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  li.paused .open {
    opacity: 0.65;
  }
  .open {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    border: 0;
    padding: 0;
    background: none;
    text-align: left;
  }
  .folders {
    font-size: 13px;
  }
  .status-line {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: baseline;
  }
  /* The state dot takes the colour of its tone. */
  .dot {
    display: inline-block;
    width: 9px;
    height: 9px;
    margin-right: 6px;
    border-radius: 50%;
    background: currentColor;
    vertical-align: middle;
  }
  .empty {
    text-align: center;
    padding: 32px;
  }
</style>
