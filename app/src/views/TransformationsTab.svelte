<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { api, errorText, type PairView, type Settings, type TransformSample } from "../lib/api";
  import SaveWithPreview from "../lib/SaveWithPreview.svelte";

  let { pair, onchange }: { pair: PairView; onchange: () => void } = $props();

  // Edited copies: a refresh of the pair must not overwrite what is being typed.
  // svelte-ignore state_referenced_locally
  let links = $state(pair.links);
  // svelte-ignore state_referenced_locally
  let assetsDir = $state(pair.assets_dir);
  // svelte-ignore state_referenced_locally
  let frontmatter = $state(pair.frontmatter);
  // svelte-ignore state_referenced_locally
  let dropped = $state(pair.frontmatter_drop.join(", "));
  // svelte-ignore state_referenced_locally
  let callouts = $state(pair.callouts);

  let sample = $state<TransformSample | null>(null);
  let sampleLoaded = $state(false);
  let error = $state<string | null>(null);

  const isObsidian = $derived(pair.kb_type === "obsidian");
  const droppedList = $derived(
    dropped.split(",").map((key) => key.trim()).filter(Boolean),
  );

  function settings(): Settings {
    const changes: Settings = {};
    if (links !== pair.links) changes["transform.links"] = links;
    if (assetsDir !== pair.assets_dir) changes["transform.assets_dir"] = assetsDir;
    if (frontmatter !== pair.frontmatter) changes["transform.frontmatter"] = frontmatter;
    // `keep` only means something with an empty list: the list is read after it.
    const wanted = frontmatter === "keep" ? [] : droppedList;
    if (wanted.join() !== pair.frontmatter_drop.join()) {
      changes["kb_options.frontmatter_drop"] = wanted;
    }
    if (callouts !== pair.callouts) changes["kb_options.callouts"] = callouts;
    return changes;
  }

  const PAUSE_MS = 300;

  $effect(() => {
    const edited = settings();
    let current = true;
    const timer = setTimeout(async () => {
      try {
        const value = await api.previewTransform(pair.name, edited);
        if (current) {
          sample = value;
          sampleLoaded = true;
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

<h2>{t("Generic options")}</h2>
<div class="field top">
  <label for="links">{t("Links")}</label>
  <select id="links" bind:value={links}>
    <option value="markdown">{t("Convert to Markdown links")}</option>
    <option value="keep">{t("Leave as they are")}</option>
  </select>
</div>
<div class="field">
  <label for="assets">{t("Attachments folder in the repo")}</label>
  <input id="assets" type="text" bind:value={assetsDir} spellcheck="false" />
</div>
<div class="field">
  <label for="frontmatter">{t("Frontmatter in the repo")}</label>
  <select id="frontmatter" bind:value={frontmatter}>
    <option value="filter">{t("Keep, except some keys")}</option>
    <option value="keep">{t("Keep all of it")}</option>
    <option value="drop">{t("Remove all of it")}</option>
  </select>
</div>
{#if frontmatter === "filter"}
  <div class="field">
    <label for="dropped">{t("Keys left out, separated by commas")}</label>
    <input id="dropped" type="text" bind:value={dropped} spellcheck="false" />
  </div>
{/if}

{#if isObsidian}
  <h2>{t("Obsidian options")}</h2>
  <div class="field top">
    <label for="callouts">{t("Callouts")}</label>
    <select id="callouts" bind:value={callouts}>
      <option value="github">{t("Convert to GitHub alerts")}</option>
      <option value="keep">{t("Leave as they are")}</option>
    </select>
  </div>
{/if}

<h2>{t("Before and after")}</h2>
{#if error}
  <p class="error" role="alert">{error}</p>
{:else if sample}
  <p class="muted path sample">{sample.path}</p>
  <div class="columns">
    <section>
      <h3>{t("In the KB")}</h3>
      <pre>{sample.before}</pre>
    </section>
    <section>
      <h3>{t("In the repo")}</h3>
      <pre>{sample.after}</pre>
    </section>
  </div>
{:else if sampleLoaded}
  <p class="muted">{t("No note to show yet.")}</p>
{:else}
  <p class="muted">{t("Looking for a note…")}</p>
{/if}

<SaveWithPreview pair={pair.name} {settings} onsaved={onchange} />

<style>
  .top {
    margin-top: 8px;
  }
  h2 {
    margin-top: 16px;
  }
  h3 {
    font-size: 12px;
    color: var(--muted);
    margin-bottom: 4px;
  }
  .sample {
    margin: 4px 0;
  }
  .columns {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 8px;
  }
  pre {
    max-height: 260px;
  }
</style>
