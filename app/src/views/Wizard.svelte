<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { onMount } from "svelte";
  import {
    api,
    arrow,
    errorText,
    isValidLabel,
    isValidProject,
    kbLabel,
    MAX_PROJECT_LEN,
    nameTaken,
    projectsOf,
    type Direction,
    type PairView,
    type Plan,
    type TargetChanges,
  } from "../lib/api";
  import DirectionCards from "../lib/DirectionCards.svelte";
  import FilterEditor from "../lib/FilterEditor.svelte";
  import Picker from "../lib/Picker.svelte";
  import PlanList from "../lib/PlanList.svelte";

  let {
    project: initialProject,
    oncancel,
    ondone,
  }: {
    /** Project the new sync goes in, chosen beforehand. */
    project?: string;
    oncancel: () => void;
    /** Called with the identifier of the new sync. */
    ondone: (pair: string) => void;
  } = $props();

  // Picked in the list of projects: an existing one, or NEW_PROJECT to name one.
  const NEW_PROJECT = "\u0000new";

  const steps = ["Knowledge base", "Repo", "Direction", "Filters", "Triggers", "Summary"];
  const hookChoices = ["post-merge", "post-checkout", "post-commit", "pre-commit"];

  let step = $state(0);
  let known = $state<string[]>([]);
  // Projects and syncs that exist, null until they are loaded.
  let projects = $state<string[] | null>(null);
  let existing = $state<PairView[]>([]);
  let projectChoice = $state("");
  // svelte-ignore state_referenced_locally
  let newProject = $state(initialProject ?? "");
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
    api
      .listPairs()
      .then((pairs) => {
        projects = projectsOf(pairs);
        existing = pairs;
        const preset = initialProject && existingProject(initialProject);
        projectChoice = preset || (projects.length > 0 && !initialProject ? "" : NEW_PROJECT);
      })
      .catch(() => {
        projects = [];
        projectChoice = NEW_PROJECT;
      });
  });

  /** The existing project spelled like `name`, ignoring case. */
  function existingProject(name: string): string | undefined {
    const wanted = name.trim().toLowerCase();
    return projects?.find((existing) => existing.toLowerCase() === wanted);
  }

  const project = $derived(projectChoice === NEW_PROJECT ? newProject.trim() : projectChoice);
  // A new name that matches an existing project joins it.
  const joins = $derived(projectChoice === NEW_PROJECT ? existingProject(newProject) : undefined);

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

  /** The last folder of a path, as it is written: a readable default name. */
  function suggestedName(path: string): string {
    const last = path.replace(/[/\\]+$/, "").split(/[/\\]/).pop() ?? "";
    return [...last.trim()].slice(0, MAX_PROJECT_LEN).join("").trim();
  }

  // The name as it is saved: free text, without surrounding spaces.
  const cleanName = $derived(name.trim());
  // Why the name cannot be used, if it cannot; nothing while it is empty.
  const nameError = $derived(
    cleanName === ""
      ? null
      : !isValidLabel(cleanName)
        ? t("Use one line of at most {max} characters, without “/”.", { max: MAX_PROJECT_LEN })
        : nameTaken(existing, joins ?? project, cleanName)
          ? t("A sync of this project already has this name.")
          : null,
  );

  const newPair = $derived({
    name: cleanName,
    project,
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
      ? isValidProject(project) &&
        detected?.exists === true &&
        cleanName !== "" &&
        nameError === null
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
    let id: string;
    try {
      id = await api.addPair(newPair);
    } catch (failure) {
      error = errorText(failure);
      busy = false;
      return;
    }
    // The pair exists from here on: what fails now is shown on its page.
    if (hooks.length > 0) await api.installHooks(id).catch(() => {});
    api.syncPair(id).catch(() => {});
    ondone(id);
  }
</script>

{#snippet projectName()}
  <div class="field">
    <label for="new-project">{t("Project name")}</label>
    <input
      id="new-project"
      type="text"
      bind:value={newProject}
      maxlength={MAX_PROJECT_LEN}
      placeholder="Markoo"
      spellcheck="false"
    />
    <div class="hint">
      {#if newProject.trim() !== "" && !isValidProject(newProject)}
        <span class="error">{t("Use one line of at most {max} characters, without “/”.", { max: MAX_PROJECT_LEN })}</span>
      {:else if joins}
        <span class="muted">{t("This sync joins the project {project}.", { project: joins })}</span>
      {:else if projects?.length === 0}
        <span class="muted">{t("A project groups this sync and the next ones you add to it.")}</span>
      {/if}
    </div>
  </div>
{/snippet}

<h1>{t("New sync")}</h1>
<ol class="steps">
  {#each steps as label, index}
    <li class:current={index === step} class:done={index < step}>{index + 1}. {t(label)}</li>
  {/each}
</ol>

<div class="card body">
  {#if step === 0}
    {#if projects !== null && projects.length > 0}
      <div class="field">
        <label for="project">{t("Project")}</label>
        <select id="project" bind:value={projectChoice}>
          <option value="" disabled>{t("Choose a project")}</option>
          {#each projects as choice}
            <option value={choice}>{choice}</option>
          {/each}
          <option value={NEW_PROJECT}>{t("New project…")}</option>
        </select>
      </div>
      {#if projectChoice === NEW_PROJECT}
        {@render projectName()}
      {/if}
    {:else if projects !== null}
      {@render projectName()}
    {/if}
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
      <input
        id="name"
        type="text"
        bind:value={name}
        oninput={() => (nameEdited = true)}
        maxlength={MAX_PROJECT_LEN}
        placeholder={t("Technical documentation")}
        spellcheck="false"
      />
      {#if nameError}
        <div class="hint error">{nameError}</div>
      {:else}
        <div class="hint muted">{t("Free text, shown in the list of syncs. You can change it later.")}</div>
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
    <h2>{joins ?? project} / {cleanName}</h2>
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
