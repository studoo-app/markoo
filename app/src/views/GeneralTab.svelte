<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { onMount } from "svelte";
  import {
    api,
    errorText,
    isValidLabel,
    isValidProject,
    MAX_PROJECT_LEN,
    nameTaken,
    projectsOf,
    type PairView,
    type Plan,
    type Settings,
  } from "../lib/api";
  import DirectionCards from "../lib/DirectionCards.svelte";
  import Picker from "../lib/Picker.svelte";
  import PlanList from "../lib/PlanList.svelte";

  let { pair, onchange }: { pair: PairView; onchange: () => void } = $props();

  // Edited copies: a refresh of the pair must not overwrite what is being typed.
  let name = $state("");
  let project = $state("");
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
  let others = $state<PairView[]>([]);
  const projects = $derived(projectsOf(others));

  function loadPairs() {
    api.listPairs().then((pairs) => (others = pairs)).catch(() => {});
  }

  onMount(loadPairs);

  // Follows the saved name: a rename keeps the identifier, so this page stays.
  const savedName = $derived(pair.name);
  $effect(() => {
    name = savedName;
  });

  // Follows the saved project, which may take the spelling of an existing one.
  const savedProject = $derived(pair.project ?? "");
  $effect(() => {
    project = savedProject;
  });

  const projectChanged = $derived(project.trim() !== savedProject);
  // A sync made before projects may stay without one, but a project is never emptied.
  const projectOk = $derived(!projectChanged || isValidProject(project));

  const nameChanged = $derived(name.trim() !== savedName);
  // Why the name cannot be saved, if it cannot.
  const nameError = $derived(
    !nameChanged && !projectChanged
      ? null
      : !isValidLabel(name.trim())
        ? t("Use one line of at most {max} characters, without “/”.", { max: MAX_PROJECT_LEN })
        : nameTaken(others, project.trim() || pair.project, name, pair.id)
          ? t("A sync of this project already has this name.")
          : null,
  );

  function settings(): Settings {
    const changes: Settings = {};
    if (nameChanged) changes.name = name.trim();
    if (projectChanged) changes.project = project.trim();
    if (kb !== pair.kb) changes.kb = kb;
    if (repo !== pair.repo) changes.repo = repo;
    if (kbType !== pair.kb_type) changes.kb_type = kbType;
    if (direction !== pair.direction) changes.direction = direction;
    if (targetChanges !== pair.target_changes) changes.target_changes = targetChanges;
    return changes;
  }

  const dirty = $derived(
    nameChanged ||
      projectChanged ||
      kb !== pair.kb ||
      repo !== pair.repo ||
      kbType !== pair.kb_type ||
      direction !== pair.direction ||
      targetChanges !== pair.target_changes,
  );

  async function showPreview() {
    // Renaming a sync or moving it to another project changes no file:
    // nothing to preview.
    if (Object.keys(settings()).every((key) => key === "name" || key === "project")) {
      await confirm();
      return;
    }
    busy = true;
    saved = false;
    try {
      preview = await api.previewSettings(pair.id, settings());
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
      await api.updateSettings(pair.id, settings());
      preview = null;
      error = null;
      saved = true;
      onchange();
      loadPairs();
    } catch (failure) {
      error = errorText(failure);
    } finally {
      busy = false;
    }
  }
</script>

{#if preview}
  <h2>{t("Preview")}</h2>
  <p class="muted">{t("Nothing is saved yet. Saving lets Markoo sync with these settings.")}</p>
  <div class="card"><PlanList plan={preview} /></div>
  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}
  <div class="row end">
    <button type="button" onclick={() => (preview = null)}>{t("Back")}</button>
    <button class="primary" type="button" disabled={busy} onclick={confirm}>{t("Save")}</button>
  </div>
{:else}
  <div class="field">
    <label for="name">{t("Name of the sync")}</label>
    <input id="name" type="text" bind:value={name} maxlength={MAX_PROJECT_LEN} spellcheck="false" />
    {#if nameError}
      <div class="hint error">{nameError}</div>
    {:else}
      <div class="hint muted">
        {t("Identifier")}: <code>{pair.id}</code> — {t("used by the CLI and the Git hooks; it never changes.")}
      </div>
    {/if}
  </div>
  <div class="field">
    <label for="project">{t("Project")}</label>
    <input
      id="project"
      type="text"
      list="known-projects"
      bind:value={project}
      maxlength={MAX_PROJECT_LEN}
      spellcheck="false"
    />
    <datalist id="known-projects">
      {#each projects as known}
        <option value={known}></option>
      {/each}
    </datalist>
    {#if !projectOk}
      <div class="hint error">{t("Name the project of this sync.")}</div>
    {/if}
  </div>
  <div class="field">
    <label for="kb-type">{t("KB type")}</label>
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
    <label for="repo">{t("Repo")}</label>
    <Picker id="repo" bind:value={repo} />
  </div>
  <div class="field">
    <span class="heading">{t("Sync direction")}</span>
    <DirectionCards bind:direction bind:targetChanges />
  </div>
  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}
  {#if saved}
    <p class="ok" role="status">{t("Saved.")}</p>
  {/if}
  <div class="row end">
    <button class="primary" type="button" disabled={!dirty || !projectOk || nameError !== null || busy} onclick={showPreview}>{t("Save and preview")}</button>
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
