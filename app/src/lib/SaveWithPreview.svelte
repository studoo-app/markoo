<script lang="ts">
  import { api, errorText, type Plan, type Settings } from "./api";
  import PlanList from "./PlanList.svelte";

  let {
    pair,
    settings,
    onsaved,
  }: { pair: string; settings: () => Settings; onsaved: () => void } = $props();

  let preview = $state<Plan | null>(null);
  let error = $state<string | null>(null);
  let saved = $state(false);
  let busy = $state(false);

  const dirty = $derived(Object.keys(settings()).length > 0);

  async function run(work: () => Promise<void>) {
    busy = true;
    try {
      await work();
      error = null;
    } catch (failure) {
      error = errorText(failure);
    } finally {
      busy = false;
    }
  }

  const showPreview = () =>
    run(async () => {
      saved = false;
      preview = await api.previewSettings(pair, settings());
    });

  const save = () =>
    run(async () => {
      await api.updateSettings(pair, settings());
      preview = null;
      saved = true;
      onsaved();
    });
</script>

{#if error}
  <p class="error" role="alert">{error}</p>
{/if}
{#if saved && !dirty}
  <p class="ok" role="status">Saved.</p>
{/if}

{#if preview}
  <section>
    <h2>Preview</h2>
    <p class="muted">Nothing is saved yet. Saving lets Markoo sync with these settings.</p>
    <div class="card"><PlanList plan={preview} /></div>
    <div class="row end">
      <button type="button" onclick={() => (preview = null)}>Back</button>
      <button class="primary" type="button" disabled={busy} onclick={save}>Save</button>
    </div>
  </section>
{:else}
  <div class="row end">
    <button class="primary" type="button" disabled={!dirty || busy} onclick={showPreview}>
      Save and preview
    </button>
  </div>
{/if}

<style>
  section {
    margin-top: 16px;
  }
  section p {
    margin: 4px 0 8px;
  }
  .end {
    justify-content: flex-end;
    margin-top: 12px;
  }
</style>
