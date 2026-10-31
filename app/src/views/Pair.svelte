<script lang="ts">
  import { api, arrow, errorText, kbLabel, type PairView } from "../lib/api";
  import ChangesTab from "./ChangesTab.svelte";
  import ConflictsTab from "./ConflictsTab.svelte";
  import FiltersTab from "./FiltersTab.svelte";
  import GeneralTab from "./GeneralTab.svelte";
  import TransformationsTab from "./TransformationsTab.svelte";
  import TriggersTab from "./TriggersTab.svelte";

  let {
    name,
    revision,
    onclose,
  }: { name: string; revision: number; onclose: () => void } = $props();

  type Tab = "changes" | "conflicts" | "general" | "filters" | "transformations" | "triggers";

  let pair = $state<PairView | null>(null);
  let tab = $state<Tab>("changes");
  let error = $state<string | null>(null);
  let syncing = $state(false);
  let confirmRemove = $state(false);
  let localRevision = $state(0);

  async function load() {
    try {
      const pairs = await api.listPairs();
      pair = pairs.find((candidate) => candidate.name === name) ?? null;
      error = pair ? null : `The pair "${name}" no longer exists.`;
    } catch (failure) {
      error = errorText(failure);
    }
  }

  $effect(() => {
    void revision;
    void localRevision;
    load();
  });

  function changed() {
    localRevision++;
  }

  async function act(work: () => Promise<unknown>) {
    try {
      await work();
      error = null;
    } catch (failure) {
      error = errorText(failure);
    }
    changed();
  }

  async function sync() {
    syncing = true;
    await act(() => api.syncPair(name));
    syncing = false;
  }

  async function remove() {
    try {
      await api.removePair(name);
      onclose();
    } catch (failure) {
      error = errorText(failure);
      confirmRemove = false;
    }
  }

  const tabs: { id: Tab; label: string }[] = [
    { id: "changes", label: "Changes" },
    { id: "conflicts", label: "Conflicts" },
    { id: "general", label: "General" },
    { id: "filters", label: "Filters" },
    { id: "transformations", label: "Transformations" },
    { id: "triggers", label: "Triggers" },
  ];
</script>

<div class="row spread title">
  <div>
    <button class="back" type="button" onclick={onclose}>‹ Pairs</button>
    <h1>{name}</h1>
    {#if pair}
      <p class="muted path sub">
        {kbLabel(pair.kb_type)} · {pair.kb} <strong>{arrow(pair.direction)}</strong> {pair.repo}
      </p>
    {/if}
  </div>
  {#if pair}
    <div class="row actions">
      <button
        type="button"
        onclick={() => act(() => api.updateSettings(name, { paused: !pair!.paused }))}
      >
        {pair.paused ? "Resume" : "Pause"}
      </button>
      <button
        class="primary"
        type="button"
        disabled={syncing || pair.active || pair.paused || pair.errors.length > 0}
        onclick={sync}
      >
        {syncing || pair.active ? "Syncing…" : "Sync now"}
      </button>
    </div>
  {/if}
</div>

{#if error}
  <p class="error" role="alert">{error}</p>
{/if}
{#if pair && pair.errors.length > 0}
  <p class="error" role="alert">{pair.errors.join("; ")}</p>
{/if}

{#if pair}
  <div class="tabs" role="tablist">
    {#each tabs as item}
      <button
        type="button"
        role="tab"
        aria-selected={tab === item.id}
        class:selected={tab === item.id}
        onclick={() => (tab = item.id)}
      >
        {item.label}{#if item.id === "conflicts" && pair.pending > 0}&nbsp;<span class="count">{pair.pending}</span>{/if}
      </button>
    {/each}
  </div>

  {#if tab === "changes"}
    <ChangesTab {name} revision={revision + localRevision} />
  {:else if tab === "conflicts"}
    <ConflictsTab {name} direction={pair.direction} revision={revision + localRevision} onchange={changed} />
  {:else if tab === "filters"}
    <FiltersTab {pair} onchange={changed} />
  {:else if tab === "transformations"}
    <TransformationsTab {pair} onchange={changed} />
  {:else if tab === "triggers"}
    <TriggersTab {pair} onchange={changed} />
  {:else}
    <GeneralTab {pair} onchange={changed} />
    <section class="remove card">
      <h2>Remove this pair</h2>
      <p class="muted">Files stay where they are on both sides. Markoo forgets the pair and its last sync.</p>
      {#if confirmRemove}
        <div class="row">
          <button class="danger" type="button" onclick={remove}>Yes, remove “{name}”</button>
          <button type="button" onclick={() => (confirmRemove = false)}>Cancel</button>
        </div>
      {:else}
        <button class="danger" type="button" onclick={() => (confirmRemove = true)}>Remove…</button>
      {/if}
    </section>
  {/if}
{/if}

<style>
  .title {
    align-items: flex-start;
    margin-bottom: 14px;
    gap: 16px;
  }
  .actions {
    flex: none;
  }
  .back {
    border: 0;
    background: none;
    padding: 0;
    color: var(--muted);
    margin-bottom: 2px;
  }
  .sub {
    margin: 2px 0 0;
    font-size: 13px;
  }
  .tabs {
    display: flex;
    gap: 2px;
    border-bottom: 1px solid var(--line);
    margin-bottom: 16px;
  }
  .tabs button {
    border: 0;
    border-bottom: 2px solid transparent;
    border-radius: 0;
    background: none;
    padding: 6px 12px;
  }
  .tabs button.selected {
    border-bottom-color: var(--accent);
    font-weight: 600;
  }
  .count {
    background: var(--warn);
    color: var(--surface);
    border-radius: 9px;
    padding: 0 6px;
    font-size: 11px;
    font-weight: 700;
  }
  .remove {
    margin-top: 24px;
  }
  .remove p {
    margin: 4px 0 10px;
  }
</style>
