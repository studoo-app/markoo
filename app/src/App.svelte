<script lang="ts">
  import { setLocale, t } from "./lib/i18n.svelte";
  import { checkForUpdate, updates } from "./lib/updates.svelte";
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import logoName from "../../assets/logo-mark-name.svg";
  import logoNameDark from "../../assets/logo-mark-name-dark.svg";
  import { api, type AppInfo } from "./lib/api";
  import Icon, { type IconName } from "./lib/Icon.svelte";
  import Activity from "./views/Activity.svelte";
  import Home from "./views/Home.svelte";
  import Pair, { type Tab } from "./views/Pair.svelte";
  import Settings from "./views/Settings.svelte";
  import Wizard from "./views/Wizard.svelte";

  type Route =
    | { name: "home" }
    | { name: "activity" }
    | { name: "wizard"; project?: string }
    | { name: "settings" }
    | { name: "pair"; pair: string; tab?: Tab };

  type Section = "syncs" | "activity" | "settings";

  let route = $state<Route>({ name: "home" });
  let info = $state<AppInfo | null>(null);
  // Goes up when something changed outside of this window.
  let revision = $state(0);

  // The sidebar highlights the section a route belongs to.
  const section = $derived<Section>(
    route.name === "activity" ? "activity" : route.name === "settings" ? "settings" : "syncs",
  );

  const top: { id: Section; label: string; icon: IconName; go: Route }[] = [
    { id: "syncs", label: "Syncs", icon: "folder", go: { name: "home" } },
  ];
  const bottom: { id: Section; label: string; icon: IconName; go: Route }[] = [
    { id: "activity", label: "Activity", icon: "activity", go: { name: "activity" } },
    { id: "settings", label: "Settings", icon: "settings", go: { name: "settings" } },
  ];

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

{#snippet item(entry: { id: Section; label: string; icon: IconName; go: Route })}
  <button
    type="button"
    class:active={section === entry.id}
    aria-current={section === entry.id ? "page" : undefined}
    onclick={() => (route = entry.go)}
  >
    <Icon name={entry.icon} />
    <span>{t(entry.label)}</span>
  </button>
{/snippet}

<div class="shell">
  <aside>
    <div class="brand">
      <button class="home" type="button" onclick={() => (route = { name: "home" })} aria-label={t("Home")}>
        <picture>
          <source srcset={logoNameDark} media="(prefers-color-scheme: dark)" />
          <img src={logoName} alt="Markoo" height="26" />
        </picture>
      </button>
    </div>

    <nav aria-label={t("Main")}>
      {#each top as entry (entry.id)}
        {@render item(entry)}
      {/each}
    </nav>

    <nav class="bottom" aria-label={t("More")}>
      {#each bottom as entry (entry.id)}
        {@render item(entry)}
      {/each}
    </nav>
  </aside>

  <div class="content">
    {#if updates.value.kind === "available"}
      <p class="note" role="status">
        {t("A new version ({version}) is available. Install it from Settings.", { version: updates.value.update.version })}
      </p>
    {/if}
    {#if info?.note}
      <p class="note warn" role="status">{info.note}</p>
    {/if}

    <main>
      {#if route.name === "home"}
        <Home
          {revision}
          onopen={(pair, tab) => (route = { name: "pair", pair, tab })}
          onnew={(project) => (route = { name: "wizard", project })}
        />
      {:else if route.name === "activity"}
        <Activity {revision} onopen={(pair) => (route = { name: "pair", pair, tab: "history" })} />
      {:else if route.name === "settings"}
        <Settings {info} onclose={() => (route = { name: "home" })} />
      {:else if route.name === "wizard"}
        <Wizard
          project={route.project}
          oncancel={() => (route = { name: "home" })}
          ondone={(pair) => (route = { name: "pair", pair })}
        />
      {:else}
        {#key route.pair}
          <Pair
            name={route.pair}
            initialTab={route.tab}
            {revision}
            onclose={() => (route = { name: "home" })}
          />
        {/key}
      {/if}
    </main>
  </div>
</div>

<style>
  .shell {
    display: grid;
    grid-template-columns: 230px 1fr;
    height: 100vh;
  }
  aside {
    display: flex;
    flex-direction: column;
    padding: 28px 20px 24px;
    background: var(--sidebar);
    border-right: 1px solid var(--line);
    overflow-y: auto;
  }
  .brand {
    padding: 0 4px 20px;
    margin: 0 8px 20px;
    border-bottom: 1px solid var(--line);
  }
  .home {
    display: flex;
    padding: 0;
    border: 0;
    background: none;
  }
  picture {
    display: flex;
  }
  nav {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  nav.bottom {
    margin-top: auto;
  }
  nav button {
    display: flex;
    align-items: center;
    gap: 14px;
    width: 100%;
    padding: 13px 16px;
    border: 0;
    border-radius: 10px;
    background: none;
    color: color-mix(in srgb, var(--ink) 82%, var(--muted));
    font-size: 16px;
    text-align: left;
  }
  nav button :global(.icon) {
    color: currentColor;
  }
  nav button:hover:not(:disabled) {
    background: var(--tile);
  }
  nav button.active {
    background: color-mix(in srgb, var(--accent) 11%, transparent);
    color: var(--accent-ink);
    font-weight: 500;
  }
  .content {
    min-width: 0;
    overflow-y: auto;
  }
  .note {
    margin: 0;
    padding: 10px 48px;
    border-bottom: 1px solid var(--line);
  }
  main {
    padding: 38px 48px 48px;
    max-width: 1040px;
  }
  @media (max-width: 1000px) {
    main {
      padding: 30px 28px 40px;
    }
  }
</style>
