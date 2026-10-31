<script lang="ts">
  import { errorText, type FilterPreview } from "./api";

  let {
    include = $bindable(),
    exclude = $bindable(),
    load,
  }: {
    include: string[];
    exclude: string[];
    /** Called again after each change, with the patterns already updated. */
    load: () => Promise<FilterPreview>;
  } = $props();

  let preview = $state<FilterPreview | null>(null);
  let error = $state<string | null>(null);

  // The scan walks both folders: it waits for a pause in the typing.
  const PAUSE_MS = 300;

  $effect(() => {
    void include.join();
    void exclude.join();
    let current = true;
    const timer = setTimeout(async () => {
      try {
        const value = await load();
        if (current) {
          preview = value;
          error = null;
        }
      } catch (failure) {
        if (current) error = errorText(failure);
      }
    }, PAUSE_MS);
    return () => {
      current = false;
      clearTimeout(timer);
    };
  });
</script>

{#snippet patterns(title: string, id: string, list: string[])}
  <div class="field">
    <label for={`${id}-0`}>{title}</label>
    {#each list as _, index}
      <div class="row line">
        <input id={`${id}-${index}`} type="text" bind:value={list[index]} spellcheck="false" />
        <button type="button" aria-label="Remove this pattern" onclick={() => list.splice(index, 1)}>−</button>
      </div>
    {/each}
    <button type="button" onclick={() => list.push("")}>+ Add a pattern</button>
  </div>
{/snippet}

{@render patterns("Include", "include", include)}
{@render patterns("Exclude", "exclude", exclude)}

{#if error}
  <p class="error" role="alert">{error}</p>
{:else if preview}
  <p>
    <strong>{preview.tracked_count}</strong> file(s) tracked ·
    <strong>{preview.excluded_count}</strong> excluded
  </p>
  <ul>
    {#each preview.excluded as path}
      <li class="muted path">✗ {path}</li>
    {/each}
    {#each preview.tracked as path}
      <li class="path"><span class="ok">✓</span> {path}</li>
    {/each}
  </ul>
  {#if preview.tracked_count + preview.excluded_count > preview.tracked.length + preview.excluded.length}
    <p class="muted hint">The list is cut; the numbers above are complete.</p>
  {/if}
{:else}
  <p class="muted">Looking at the files…</p>
{/if}

<style>
  .line {
    margin-bottom: 4px;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 240px;
    overflow: auto;
    font-size: 13px;
  }
</style>
