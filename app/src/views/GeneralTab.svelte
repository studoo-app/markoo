<script lang="ts">
  import { api, errorText, type PairView, type Plan, type Settings } from "../lib/api";
  import DirectionCards from "../lib/DirectionCards.svelte";
  import Picker from "../lib/Picker.svelte";
  import PlanList from "../lib/PlanList.svelte";

  let { pair, onchange }: { pair: PairView; onchange: () => void } = $props();

  // Edited copies: a refresh of the pair must not overwrite what is being typed.
  // svelte-ignore state_referenced_locally
  let kb = $state(pair.kb);
  // svelte-ignore state_referenced_locally
  let repo = $state(pair.repo);
  // svelte-ignore state_referenced_locally
  let kbType = $state(pair.kb_type);
  // svelte-ignore state_referenced_locally
  let direction = $state(pair.direction);
  // svelte-ignore state_referenced_locally
  let targetChanges = $state(pair.target_changes);

  let preview = $state<Plan | null>(null);
  let error = $state<string | null>(null);
  let saved = $state(false);
  let busy = $state(false);

  function settings(): Settings {
    const changes: Settings = {};
    if (kb !== pair.kb) changes.kb = kb;
    if (repo !== pair.repo) changes.repo = repo;
    if (kbType !== pair.kb_type) changes.kb_type = kbType;
    if (direction !== pair.direction) changes.direction = direction;
    if (targetChanges !== pair.target_changes) changes.target_changes = targetChanges;
    return changes;
  }

  const dirty = $derived(
    kb !== pair.kb ||
      repo !== pair.repo ||
      kbType !== pair.kb_type ||
      direction !== pair.direction ||
      targetChanges !== pair.target_changes,
  );

  async function showPreview() {
    busy = true;
    saved = false;
    try {
      preview = await api.previewSettings(pair.name, settings());
      error = null;
    } catch (failure) {
      error = errorText(failure);
    } finally {
      busy = false;
    }
  }

  async function confirm() {
    busy = true;
    try {
      await api.updateSettings(pair.name, settings());
      preview = null;
      error = null;
      saved = true;
      onchange();
    } catch (failure) {
      error = errorText(failure);
    } finally {
      busy = false;
    }
  }
</script>

{#if preview}
  <h2>Preview</h2>
  <p class="muted">Nothing is saved yet. Saving lets Markoo sync with these settings.</p>
  <div class="card"><PlanList plan={preview} /></div>
  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}
  <div class="row end">
    <button type="button" onclick={() => (preview = null)}>Back</button>
    <button class="primary" type="button" disabled={busy} onclick={confirm}>Save</button>
  </div>
{:else}
  <div class="field">
    <label for="kb-type">KB type</label>
    <select id="kb-type" bind:value={kbType}>
      <option value="obsidian">Obsidian</option>
      <option value="markdown">Markdown</option>
    </select>
  </div>
  <div class="field">
    <label for="kb">KB</label>
    <Picker id="kb" bind:value={kb} />
  </div>
  <div class="field">
    <label for="repo">Repo</label>
    <Picker id="repo" bind:value={repo} />
  </div>
  <div class="field">
    <span class="heading">Sync direction</span>
    <DirectionCards bind:direction bind:targetChanges />
  </div>
  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}
  {#if saved}
    <p class="ok" role="status">Saved.</p>
  {/if}
  <div class="row end">
    <button class="primary" type="button" disabled={!dirty || busy} onclick={showPreview}>
      Save and preview
    </button>
  </div>
{/if}

<style>
  .heading {
    display: block;
    font-weight: 600;
    margin-bottom: 4px;
  }
  .end {
    justify-content: flex-end;
    margin-top: 12px;
  }
</style>
