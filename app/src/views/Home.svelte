<script lang="ts">
  import { api, arrow, errorText, kbLabel, type PairView } from "../lib/api";

  let {
    revision,
    onopen,
    onnew,
  }: { revision: number; onopen: (pair: string) => void; onnew: () => void } = $props();

  let pairs = $state<PairView[] | null>(null);
  let error = $state<string | null>(null);
  let syncing = $state<string | null>(null);

  async function load() {
    try {
      pairs = await api.listPairs();
      error = null;
    } catch (failure) {
      error = errorText(failure);
    }
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
    if (pair.active) return { text: "🔄 Syncing", tone: "muted" };
    if (pair.pending > 0) return { text: `⚠️ ${pair.pending} to settle`, tone: "warn" };
    if (pair.paused) return { text: "Paused", tone: "muted" };
    return { text: "✅ No pending conflict", tone: "ok" };
  }
</script>

<div class="row spread title">
  <h1>Pairs</h1>
  <button class="primary" type="button" onclick={onnew}>+ New pair</button>
</div>

{#if error}
  <p class="error" role="alert">{error}</p>
{/if}

{#if pairs === null}
  <p class="muted">Loading…</p>
{:else if pairs.length === 0}
  <div class="card empty">
    <h2>No pair yet</h2>
    <p class="muted">A pair keeps a folder of your knowledge base in sync with a folder of a Git repository.</p>
    <button class="primary" type="button" onclick={onnew}>Create the first one</button>
  </div>
{:else}
  <ul>
    {#each pairs as pair (pair.name)}
      {@const state = status(pair)}
      <li class="card" class:paused={pair.paused}>
        <button class="open" type="button" onclick={() => onopen(pair.name)}>
          <span class="row name">
            <strong>{pair.name}</strong>
            <span class="muted">{kbLabel(pair.kb_type)}</span>
          </span>
          <span class="path folders">{pair.kb} <strong>{arrow(pair.direction)}</strong> {pair.repo}</span>
          <span class={state.tone}>{state.text}</span>
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
  .empty {
    text-align: center;
    padding: 32px;
  }
</style>
