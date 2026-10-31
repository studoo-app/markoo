<script lang="ts">
  import { api, errorText, type Plan, type PlanEntry } from "../lib/api";
  import PlanList from "../lib/PlanList.svelte";

  let { name, revision }: { name: string; revision: number } = $props();

  let plan = $state<Plan | null>(null);
  let error = $state<string | null>(null);
  let selected = $state<string | null>(null);
  let patch = $state<string | null>(null);

  $effect(() => {
    void revision;
    api
      .pairStatus(name)
      .then((value) => {
        plan = value;
        error = null;
      })
      .catch((failure) => (error = errorText(failure)));
  });

  async function show(entry: PlanEntry) {
    selected = entry.path;
    patch = null;
    try {
      const answer = await api.pairDiff(name, entry.path);
      patch = answer.diffs[0]?.patch ?? "";
    } catch (failure) {
      patch = errorText(failure);
    }
  }
</script>

{#if error}
  <p class="error" role="alert">{error}</p>
{:else if plan === null}
  <p class="muted">Comparing both sides…</p>
{:else}
  <PlanList {plan} onselect={show} />
  {#if selected}
    <section>
      <h2 class="path">{selected}</h2>
      {#if patch === null}
        <p class="muted">Loading…</p>
      {:else if patch === ""}
        <p class="muted">No difference in the text.</p>
      {:else}
        <pre>{patch}</pre>
      {/if}
    </section>
  {/if}
{/if}

<style>
  section {
    margin-top: 16px;
  }
  h2 {
    margin-bottom: 6px;
  }
</style>
