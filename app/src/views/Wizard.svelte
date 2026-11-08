<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { onMount } from "svelte";
  import {
    api,
    arrow,
    errorText,
    kbLabel,
    type Direction,
    type Plan,
    type TargetChanges,
  } from "../lib/api";
  import DirectionCards from "../lib/DirectionCards.svelte";
  import FilterEditor from "../lib/FilterEditor.svelte";
  import Picker from "../lib/Picker.svelte";
  import PlanList from "../lib/PlanList.svelte";

  let { oncancel, ondone }: { oncancel: () => void; ondone: (pair: string) => void } = $props();

  const steps = ["Knowledge base", "Repo", "Direction", "Filters", "Triggers", "Summary"];
  const hookChoices = ["post-merge", "post-checkout", "post-commit", "pre-commit"];

  let step = $state(0);
  let known = $state<string[]>([]);
  let kb = $state("");
  let kbType = $state("");
  let detected = $state<{ exists: boolean; kb_type: string } | null>(null);
  let repo = $state("");
  let repoInfo = $state<{ exists: boolean; git: boolean; branch: string | null } | null>(null);
  let direction = $state<Direction>("both");
  let targetChanges = $state<TargetChanges>("protect");
  let name = $state("");
  let nameEdited = $state(false);
  let include = $state(["**/*.md"]);
  let exclude = $state<string[]>([]);
  let hooks = $state<string[]>([]);
  let branches = $state("");
  let preview = $state<Plan | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);

  onMount(() => {
    api.knownKbs().then((value) => (known = value)).catch(() => {});
  });

  $effect(() => {
    const path = kb.trim();
    if (!path) {
      detected = null;
      return;
    }
    api.inspectKb(path).then((value) => {
      if (kb.trim() === path) detected = value;
    });
    if (!nameEdited) name = suggestedName(path);
  });

  $effect(() => {
    const path = repo.trim();
    if (!path) {
      repoInfo = null;
      return;
    }
    api.inspectRepo(path).then((value) => {
      if (repo.trim() === path) repoInfo = value;
    });
  });

  function suggestedName(path: string): string {
    const last = path.replace(/[/\\]+$/, "").split(/[/\\]/).pop() ?? "";
    return last
      .normalize("NFD")
      .replace(/[̀-ͯ]/g, "")
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "");
  }

  const newPair = $derived({
    name,
    kb: kb.trim(),
    kb_type: kbType || null,
    repo: repo.trim(),
    direction,
    target_changes: direction === "both" ? null : targetChanges,
    settings: pairSettings(),
  });

  function pairSettings(): [string, string][] {
    const settings: [string, string][] = [
      ["include", include.join(",")],
      ["exclude", exclude.join(",")],
    ];
    if (hooks.length > 0) settings.push(["hooks.install", hooks.join(",")]);
    if (branches.trim()) settings.push(["hooks.branches", branches]);
    return settings;
  }

  /** What the filters would do, computed by the same code as a sync. */
  const loadFilters = () => api.previewNewPairFilters(newPair);

  const canContinue = $derived(
    step === 0
      ? detected?.exists === true && /^[a-z0-9-]+$/.test(name)
      : step === 1
        ? repoInfo?.exists === true
        : true,
  );

  const conflicts = $derived(preview?.entries.filter((e) => e.kind === "pending").length ?? 0);

  async function next() {
    error = null;
    if (step === 4) {
      busy = true;
      try {
        preview = await api.previewNewPair(newPair);
        step = 5;
      } catch (failure) {
        error = errorText(failure);
      } finally {
        busy = false;
      }
      return;
    }
    step++;
  }

  async function create() {
    busy = true;
    error = null;
    try {
      await api.addPair(newPair);
    } catch (failure) {
      error = errorText(failure);
      busy = false;
      return;
    }
    // The pair exists from here on: what fails now is shown on its page.
    if (hooks.length > 0) await api.installHooks(name).catch(() => {});
    api.syncPair(name).catch(() => {});
    ondone(name);
  }
</script>

<h1>{t("New sync")}</h1>
<ol class="steps">
  {#each steps as label, index}
    <li class:current={index === step} class:done={index < step}>{index + 1}. {t(label)}</li>
  {/each}
</ol>

<div class="card body">
  {#if step === 0}
    {#if known.length > 0}
      <div class="field">
        <span class="heading">{t("Known vaults")}</span>
        <div class="row wrap">
          {#each known as path}
            <button type="button" class="path" onclick={() => (kb = path)}>{path}</button>
          {/each}
        </div>
      </div>
    {/if}
    <div class="field">
      <label for="kb">{t("Folder of the knowledge base")}</label>
      <Picker id="kb" bind:value={kb} />
      <div class="hint">
        {#if detected && !detected.exists}
          <span class="error">{t("This folder does not exist.")}</span>
        {:else if detected}
          <span class="ok">{t("✓ {label} {kind} detected", { label: kbLabel(detected.kb_type), kind: t(detected.kb_type === "obsidian" ? "vault" : "folder") })}</span>
        {/if}
      </div>
    </div>
    <div class="field">
      <label for="kb-type">{t("KB type")}</label>
      <select id="kb-type" bind:value={kbType}>
        <option value="">{t("Detect from the folder")}</option>
        <option value="obsidian">Obsidian</option>
        <option value="markdown">Markdown</option>
      </select>
    </div>
    <div class="field">
      <label for="name">{t("Name of the sync")}</label>
      <input id="name" type="text" bind:value={name} oninput={() => (nameEdited = true)} spellcheck="false" />
      {#if name && !/^[a-z0-9-]+$/.test(name)}
        <div class="hint error">{t("Use lowercase letters, digits and hyphens.")}</div>
      {/if}
    </div>
  {:else if step === 1}
    <div class="field">
      <label for="repo">{t("Folder in the Git repository")}</label>
      <Picker id="repo" bind:value={repo} />
      <div class="hint">
        {#if repoInfo && !repoInfo.exists}
          <span class="error">{t("This folder does not exist.")}</span>
        {:else if repoInfo && !repoInfo.git}
          <span class="warn">{t("This folder is not in a Git repository.")}</span>
        {:else if repoInfo}
          <span class="ok">{repoInfo.branch ? t("✓ Git repository, branch {branch}", { branch: repoInfo.branch }) : t("✓ Git repository")}</span>
        {/if}
      </div>
    </div>
    <p class="muted">{t("Markoo never commits or pushes: what it writes here stays for you to review.")}</p>
  {:else if step === 2}
    <DirectionCards bind:direction bind:targetChanges />
  {:else if step === 3}
    <FilterEditor bind:include bind:exclude load={loadFilters} />
  {:else if step === 4}
    <div class="field">
      <span class="heading">{t("Git hooks (optional)")}</span>
      {#each hookChoices as hook}
        <label class="choice">
          <input type="checkbox" bind:group={hooks} value={hook} /> {hook}
        </label>
      {/each}
      <p class="muted hint">{t("The interval is set for every sync in Settings.")}</p>
    </div>
    <div class="field">
      <label for="branches">{t("Branches covered, separated by commas (all if empty)")}</label>
      <input id="branches" type="text" bind:value={branches} spellcheck="false" />
    </div>
  {:else if preview}
    <h2>{name}</h2>
    <p class="muted path">{kb} {arrow(direction)} {repo}</p>
    {#if conflicts > 0}
      <p class="warn">
        {t("{count} file(s) exist on both sides with different content. They will wait for your decision.", { count: conflicts })}
      </p>
    {/if}
    <PlanList plan={preview} />
  {/if}
</div>

{#if error}
  <p class="error" role="alert">{error}</p>
{/if}

<div class="row spread">
  <button type="button" onclick={oncancel}>{t("Cancel")}</button>
  <div class="row">
    {#if step > 0}
      <button type="button" onclick={() => { step--; error = null; }}>{t("Back")}</button>
    {/if}
    {#if step < 5}
      <button class="primary" type="button" disabled={!canContinue || busy} onclick={next}>
        {step === 4 ? t("Preview") : t("Continue")}
      </button>
    {:else}
      <button class="primary" type="button" disabled={busy} onclick={create}>{t("Create and sync")}</button>
    {/if}
  </div>
</div>

<style>
  .steps {
    list-style: none;
    display: flex;
    flex-wrap: wrap;
    gap: 16px;
    padding: 0;
    margin: 12px 0 16px;
    color: var(--muted);
    font-size: 13px;
  }
  .steps .current {
    color: var(--ink);
    font-weight: 600;
  }
  .steps .done {
    color: var(--ok);
  }
  .body {
    margin-bottom: 14px;
  }
  .heading {
    display: block;
    font-weight: 600;
    margin-bottom: 4px;
  }
  .wrap {
    flex-wrap: wrap;
  }
  .choice {
    font-weight: 400;
    margin-bottom: 2px;
  }
</style>
