<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import logo from "../../assets/logo-mark.svg";
  import logoDark from "../../assets/logo-mark-dark.svg";
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
    const unlisten = listen("markoo-changed", () => revision++);
    const onFocus = () => revision++;
    window.addEventListener("focus", onFocus);
    return () => {
      unlisten.then((stop) => stop());
      window.removeEventListener("focus", onFocus);
    };
  });
</script>

<header class="row spread">
  <button class="home" type="button" onclick={() => (route = { name: "home" })} aria-label="Home">
    <picture>
      <source srcset={logoDark} media="(prefers-color-scheme: dark)" />
      <img src={logo} alt="" width="26" height="24" />
    </picture>
    <span>Markoo</span>
  </button>
  <button type="button" onclick={() => (route = { name: "settings" })}>Settings</button>
</header>

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
