<script lang="ts">
  import type { Direction, TargetChanges } from "./api";

  let {
    direction = $bindable(),
    targetChanges = $bindable(),
  }: { direction: Direction; targetChanges: TargetChanges } = $props();

  const choices: { value: Direction; title: string; text: string }[] = [
    { value: "kb-to-repo", title: "KB → Repo", text: "The KB is the reference" },
    { value: "both", title: "KB ⇄ Repo", text: "Writing on both sides" },
    { value: "repo-to-kb", title: "KB ← Repo", text: "The repo is the reference" },
  ];
</script>

<div class="cards" role="radiogroup" aria-label="Sync direction">
  {#each choices as choice}
    <button
      type="button"
      role="radio"
      aria-checked={direction === choice.value}
      class:selected={direction === choice.value}
      onclick={() => (direction = choice.value)}
    >
      <strong>{choice.title}</strong>
      <span class="muted">{choice.text}</span>
    </button>
  {/each}
</div>

{#if direction !== "both"}
  <fieldset>
    <legend>If the target is changed</legend>
    <label class="choice">
      <input type="radio" bind:group={targetChanges} value="protect" />
      Protect: report a divergence, overwrite nothing
    </label>
    <label class="choice">
      <input type="radio" bind:group={targetChanges} value="overwrite" />
      Overwrite: strict mirror of the source (with backup)
    </label>
    {#if targetChanges === "overwrite"}
      <p class="warn hint">
        Files that differ in the target will be replaced or moved to the trash. Check the preview.
      </p>
    {/if}
  </fieldset>
{/if}

<style>
  .cards {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 8px;
  }
  .cards button {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 12px;
    text-align: left;
  }
  .cards button.selected {
    border: 2px solid var(--accent);
    padding: 11px;
  }
  fieldset {
    border: 0;
    padding: 0;
    margin: 14px 0 0;
  }
  legend {
    font-weight: 600;
    padding: 0;
    margin-bottom: 4px;
  }
  .choice {
    font-weight: 400;
    margin-bottom: 2px;
  }
</style>
