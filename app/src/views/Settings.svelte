<script lang="ts">
  import { onMount } from "svelte";
  import { api, errorText, type AppInfo, type GlobalSettings, type Settings } from "../lib/api";

  let { info, onclose }: { info: AppInfo | null; onclose: () => void } = $props();

  let saved = $state<GlobalSettings | null>(null);
  let interval = $state("");
  let onStartup = $state(true);
  let apiEnabled = $state(true);
  let apiPort = $state("");
  let mcpHttp = $state(true);
  let allowWrite = $state(false);
  let error = $state<string | null>(null);
  let done = $state(false);
  let busy = $state(false);

  function apply(value: GlobalSettings) {
    saved = value;
    interval = value.interval;
    onStartup = value.on_startup;
    apiEnabled = value.api_enabled;
    apiPort = String(value.api_port);
    mcpHttp = value.mcp_http;
    allowWrite = value.mcp_allow_config_write;
  }

  onMount(() => {
    api.globalSettings().then(apply).catch((failure) => (error = errorText(failure)));
  });

  function changes(): Settings {
    if (!saved) return {};
    const edited: Settings = {};
    if (interval.trim() !== saved.interval) edited["schedule.interval"] = interval.trim();
    if (onStartup !== saved.on_startup) edited["schedule.on_startup"] = String(onStartup);
    if (apiEnabled !== saved.api_enabled) edited["api.enabled"] = String(apiEnabled);
    if (apiPort.trim() !== String(saved.api_port)) edited["api.port"] = apiPort.trim();
    if (mcpHttp !== saved.mcp_http) edited["mcp.http"] = String(mcpHttp);
    if (allowWrite !== saved.mcp_allow_config_write) {
      edited["mcp.allow_config_write"] = String(allowWrite);
    }
    return edited;
  }

  const dirty = $derived(Object.keys(changes()).length > 0);
  const needsRestart = $derived(
    saved !== null &&
      (apiEnabled !== saved.api_enabled ||
        apiPort.trim() !== String(saved.api_port) ||
        mcpHttp !== saved.mcp_http),
  );

  async function save() {
    busy = true;
    try {
      const restart = needsRestart;
      await api.updateGlobalSettings(changes());
      apply(await api.globalSettings());
      error = null;
      done = true;
      restarted = restart;
    } catch (failure) {
      error = errorText(failure);
      done = false;
    } finally {
      busy = false;
    }
  }

  let restarted = $state(false);
</script>

<button class="back" type="button" onclick={onclose}>‹ Pairs</button>
<h1>Settings</h1>

{#if saved}
  <section class="card">
    <h2>Schedule</h2>
    <div class="field">
      <label for="interval">Interval between syncs</label>
      <input id="interval" type="text" bind:value={interval} spellcheck="false" />
      <p class="muted hint">30s, 5m, 1h… or off.</p>
    </div>
    <label class="choice"><input type="checkbox" bind:checked={onStartup} /> Sync when Markoo starts</label>
  </section>

  <section class="card">
    <h2>Local API</h2>
    <label class="choice"><input type="checkbox" bind:checked={apiEnabled} /> Serve the API on 127.0.0.1</label>
    <div class="field port">
      <label for="port">Port</label>
      <input id="port" type="text" inputmode="numeric" bind:value={apiPort} disabled={!apiEnabled} />
    </div>
  </section>

  <section class="card">
    <h2>MCP</h2>
    <label class="choice"><input type="checkbox" bind:checked={mcpHttp} /> Also serve MCP on the API</label>
    <label class="choice"><input type="checkbox" bind:checked={allowWrite} /> Let an AI agent change the configuration</label>
    {#if allowWrite}
      <p class="warn hint">Changing a direction or a folder can cause many writes at the next sync.</p>
    {/if}
  </section>

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}
  {#if done && !dirty}
    <p class="ok" role="status">Saved.{restarted ? " Restart Markoo to apply the API settings." : ""}</p>
  {/if}
  <div class="row end">
    <button class="primary" type="button" disabled={!dirty || busy} onclick={save}>Save</button>
  </div>
{:else if error}
  <p class="error" role="alert">{error}</p>
{:else}
  <p class="muted">Loading…</p>
{/if}

{#if info}
  <section class="files muted">
    <p class="path">Configuration: {info.config_file}</p>
    <p class="path">Data: {info.data_dir}</p>
  </section>
{/if}

<style>
  .back {
    border: 0;
    background: none;
    padding: 0;
    color: var(--muted);
    margin-bottom: 2px;
  }
  h1 {
    margin-bottom: 14px;
  }
  section.card {
    margin-bottom: 10px;
  }
  h2 {
    margin-bottom: 8px;
  }
  .choice {
    font-weight: 400;
    margin-bottom: 4px;
  }
  .field {
    margin-bottom: 8px;
  }
  .port {
    margin-top: 8px;
    max-width: 160px;
  }
  .hint {
    margin: 4px 0 0;
  }
  .end {
    justify-content: flex-end;
  }
  .files {
    margin-top: 16px;
    font-size: 12px;
  }
  .files p {
    margin: 2px 0;
  }
</style>
