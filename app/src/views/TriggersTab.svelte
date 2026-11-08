<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import {
    api,
    errorText,
    type GlobalSettings,
    type HookChange,
    type HooksState,
    type PairView,
    type Settings,
  } from "../lib/api";

  let { pair, onchange }: { pair: PairView; onchange: () => void } = $props();

  const choices = ["post-merge", "post-checkout", "post-commit", "pre-commit"];

  // Edited copies: a refresh of the pair must not overwrite what is being typed.
  // svelte-ignore state_referenced_locally
  let wanted = $state([...pair.hooks.install]);
  // svelte-ignore state_referenced_locally
  let branches = $state(pair.hooks.branches.join(", "));

  let state_ = $state<HooksState | null>(null);
  let globals = $state<GlobalSettings | null>(null);
  let changes = $state<HookChange[]>([]);
  let error = $state<string | null>(null);
  let busy = $state(false);

  const branchList = $derived(branches.split(",").map((b) => b.trim()).filter(Boolean));

  $effect(() => {
    void pair.id;
    refresh();
    api.globalSettings().then((value) => (globals = value)).catch(() => {});
  });

  async function refresh() {
    try {
      state_ = await api.hooksState(pair.id);
    } catch (failure) {
      error = errorText(failure);
    }
  }

  async function run(work: () => Promise<HookChange[]>) {
    busy = true;
    try {
      // The hooks written follow the settings, so these are saved first.
      const settings: Settings = {};
      if (wanted.join() !== pair.hooks.install.join()) settings["hooks.install"] = wanted;
      if (branchList.join() !== pair.hooks.branches.join()) settings["hooks.branches"] = branchList;
      if (Object.keys(settings).length > 0) {
        await api.updateSettings(pair.id, settings);
        onchange();
      }
      changes = await work();
      error = null;
      await refresh();
    } catch (failure) {
      error = errorText(failure);
    } finally {
      busy = false;
    }
  }

  const mcpCommand = "claude mcp add -s user markoo -- markoo mcp";
</script>

<section class="field">
  <h2>{t("Interval")}</h2>
  {#if globals}
    <p class="muted">{t("Every sync runs on the same schedule: {interval}. It is set in Settings.", { interval: globals.interval })}</p>
  {/if}
</section>

<section class="field">
  <h2>{t("Git hooks")}</h2>
  {#if state_ && !state_.git}
    <p class="warn">{t("This folder is not in a Git repository.")}</p>
  {:else}
    {#if state_?.manager}
      <p class="warn">
        {t("{manager} manages the hooks of this repository: Markoo gives the lines to add instead of writing them.", { manager: state_.manager })}
      </p>
    {/if}
    {#each choices as hook}
      <label class="choice">
        <input type="checkbox" bind:group={wanted} value={hook} />
        {hook}
        {#if state_?.installed.includes(hook)}<span class="ok">{t("installed")}</span>{/if}
      </label>
    {/each}
    <div class="branches">
      <label for="branches">{t("Branches covered, separated by commas (all if empty)")}</label>
      <input id="branches" type="text" bind:value={branches} spellcheck="false" />
    </div>
    <div class="row">
      <button class="primary" type="button" disabled={busy} onclick={() => run(() => api.installHooks(pair.id))}>{t("Install")}</button>
      <button type="button" disabled={busy || !state_?.installed.length} onclick={() => run(() => api.uninstallHooks(pair.id))}>{t("Uninstall")}</button>
    </div>
    <p class="muted hint">{t("Installing syncs nothing: hooks act the next time Git runs them.")}</p>
  {/if}
  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}
  {#each changes as change}
    {#if change.status === "manual"}
      <p class="warn">{change.hook}: {change.reason}</p>
      <pre>{change.snippet}</pre>
    {/if}
  {/each}
</section>

<section class="field">
  <h2>{t("API and MCP")}</h2>
  {#if globals}
    <p class="muted">
      {t("API: {api} · MCP over HTTP: {mcp}", { api: globals.api_enabled ? `http://127.0.0.1:${globals.api_port}` : t("off"), mcp: globals.api_enabled && globals.mcp_http ? `http://127.0.0.1:${globals.api_port}/mcp` : t("off") })}
    </p>
  {/if}
  <pre>{mcpCommand}</pre>
</section>

<style>
  h2 {
    margin-bottom: 6px;
  }
  p {
    margin: 4px 0;
  }
  .choice {
    font-weight: 400;
    margin-bottom: 2px;
  }
  .branches {
    margin: 10px 0;
  }
</style>
