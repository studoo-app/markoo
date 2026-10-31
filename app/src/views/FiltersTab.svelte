<script lang="ts">
  import { api, type PairView, type Settings } from "../lib/api";
  import FilterEditor from "../lib/FilterEditor.svelte";
  import SaveWithPreview from "../lib/SaveWithPreview.svelte";

  let { pair, onchange }: { pair: PairView; onchange: () => void } = $props();

  // Edited copies: a refresh of the pair must not overwrite what is being typed.
  // svelte-ignore state_referenced_locally
  let include = $state([...pair.include]);
  // svelte-ignore state_referenced_locally
  let exclude = $state([...pair.exclude]);

  const cleaned = (list: string[]) => list.map((item) => item.trim()).filter(Boolean);
  const same = (a: string[], b: string[]) => a.join("\n") === b.join("\n");

  function settings(): Settings {
    const changes: Settings = {};
    if (!same(cleaned(include), pair.include)) changes.include = cleaned(include);
    if (!same(cleaned(exclude), pair.exclude)) changes.exclude = cleaned(exclude);
    return changes;
  }
</script>

<FilterEditor bind:include bind:exclude load={() => api.previewFilters(pair.name, settings())} />
<SaveWithPreview pair={pair.name} {settings} onsaved={onchange} />
