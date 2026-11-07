<script lang="ts">
  import { setLocale, t } from "./lib/i18n.svelte";
  import { checkForUpdate, updates } from "./lib/updates.svelte";
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import logoName from "../../assets/logo-mark-name.svg";
  import logoNameDark from "../../assets/logo-mark-name-dark.svg";
  import { api, type AppInfo } from "./lib/api";
  import Home from "./views/Home.svelte";
  import Pair from "./views/Pair.svelte";
  import Settings from "./views/Settings.svelte";
  import Wizard from "./views/Wizard.svelte";

  type Route =
    | { name: "home" }
    | { name: "wizard" }
    | { name: "settings" }
    | { name: "pair"; pair: string };

  let route = $state<Route>({ name: "home" });
  let info = $state<AppInfo | null>(null);
  // Goes up when something changed outside of this window.
  let revision = $state(0);

  onMount(() => {
    api.appInfo().then((value) => (info = value));
    api.globalSettings().then((value) => setLocale(value.language)).catch(() => {});
    // A few seconds after the start, so that the window opens first.
    const updateTimer = setTimeout(() => checkForUpdate(true), 5000);
    const unlisten = listen("markoo-changed", () => revision++);
    const onFocus = () => revision++;
    window.addEventListener("focus", onFocus);
    return () => {
      clearTimeout(updateTimer);
      unlisten.then((stop) => stop());
      window.removeEventListener("focus", onFocus);
    };
  });
</script>

<header class="row spread">
  <button class="home" type="button" onclick={() => (route = { name: "home" })} aria-label={t("Home")}>
    <picture>
      <source srcset={logoNameDark} media="(prefers-color-scheme: dark)" />
      <img src={logoName} alt="Markoo" height="30" />
    </picture>
    <span class="version">v{__APP_VERSION__}</span>
  </button>
  <button type="button" onclick={() => (route = { name: "settings" })}>{t("Settings")}</button>
</header>

{#if updates.value.kind === "available"}
  <p class="note" role="status">{t("A new version ({version}) is available. Install it from Settings.", { version: updates.value.update.version })}</p>
{/if}

{#if info?.note}
  <p class="note warn" role="status">{info.note}</p>
{/if}

<main>
  {#if route.name === "home"}
    <Home
      {revision}
      onopen={(pair) => (route = { name: "pair", pair })}
      onnew={() => (route = { name: "wizard" })}
    />
  {:else if route.name === "settings"}
    <Settings {info} onclose={() => (route = { name: "home" })} />
  {:else if route.name === "wizard"}
    <Wizard
      oncancel={() => (route = { name: "home" })}
      ondone={(pair) => (route = { name: "pair", pair })}
    />
  {:else}
    {#key route.pair}
      <Pair name={route.pair} {revision} onclose={() => (route = { name: "home" })} />
    {/key}
  {/if}
</main>

<style>
  header {
    padding: 10px 20px;
    border-bottom: 1px solid var(--line);
    background: var(--surface);
  }
  .home {
    display: flex;
    align-items: center;
    gap: 8px;
    border: 0;
    padding: 0;
    background: none;
    font-weight: 700;
    font-size: 15px;
  }
  picture {
    display: flex;
  }
  .version {
    font-weight: 400;
    font-size: 12px;
    color: var(--muted, #6b6b6b);
  }
  .note {
    margin: 0;
    padding: 8px 20px;
    border-bottom: 1px solid var(--line);
  }
  main {
    padding: 20px;
    max-width: 1100px;
    margin: 0 auto;
  }
</style>
