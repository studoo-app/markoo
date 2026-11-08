<script lang="ts">
  import { locale, t } from "../lib/i18n.svelte";
  import { api, arrow, errorText, kbLabel, type PairView } from "../lib/api";
  import Icon from "../lib/Icon.svelte";
  import type { Tab } from "./Pair.svelte";

  let {
    revision,
    onopen,
    onnew,
  }: {
    revision: number;
    onopen: (pair: string, tab?: Tab) => void;
    onnew: (project?: string) => void;
  } = $props();

  let pairs = $state<PairView[] | null>(null);
  let error = $state<string | null>(null);
  let syncing = $state<string | null>(null);
  // Key of the project being synced one sync after the other.
  let syncingProject = $state<string | null>(null);
  let confirming = $state<string | null>(null);
  let query = $state("");
  // Start time of the latest run of each pair, in milliseconds.
  let lastRuns = $state<Record<string, number>>({});

  const visible = $derived(
    (pairs ?? []).filter((pair) => {
      const needle = query.trim().toLowerCase();
      if (needle === "") return true;
      return [pair.name, pair.project ?? "", pair.kb, pair.repo].some((text) =>
        text.toLowerCase().includes(needle),
      );
    }),
  );
  const toSettle = $derived((pairs ?? []).reduce((total, pair) => total + pair.pending, 0));

  // Syncs grouped by project, in the order projects first appear; syncs made
  // before projects existed come last.
  const groups = $derived.by(() => {
    const byProject = new Map<string | null, PairView[]>();
    for (const pair of visible.filter((pair) => pair.project !== null)) {
      byProject.set(pair.project, [...(byProject.get(pair.project) ?? []), pair]);
    }
    const legacy = visible.filter((pair) => pair.project === null);
    if (legacy.length > 0) byProject.set(null, legacy);
    return [...byProject.entries()].map(([project, members]) => ({ project, members }));
  });

  const kindHint: Record<string, string> = {
    obsidian: "wikilinks, embeds and callouts converted both ways",
    markdown: "plain Markdown, copied as it is",
  };

  /** The knowledge base of a sync, as its tooltip shows it. */
  function kbTooltip(pair: PairView): string {
    const hint = kindHint[pair.kb_type];
    const kind = hint ? `${kbLabel(pair.kb_type)} (${t(hint)})` : kbLabel(pair.kb_type);
    return `${kind}: ${pair.kb}\nGit: ${pair.repo}`;
  }

  /** Whether a sync can be run now from this screen. */
  function canSync(pair: PairView): boolean {
    return !pair.paused && !pair.active && pair.errors.length === 0;
  }

  /** All the syncs of a project, whatever the search shows. */
  function membersOf(project: string | null): PairView[] {
    return (pairs ?? []).filter((pair) => pair.project === project);
  }

  async function load() {
    try {
      const listed = await api.listPairs();
      pairs = listed;
      error = null;
      const entries = await Promise.all(
        listed.map(async (pair): Promise<[string, number] | null> => {
          try {
            const runs = await api.pairHistory(pair.name);
            return runs.length > 0 ? [pair.name, runs[0].started_at] : null;
          } catch {
            return null;
          }
        }),
      );
      lastRuns = Object.fromEntries(entries.filter((entry) => entry !== null));
    } catch (failure) {
      error = errorText(failure);
    }
  }

  $effect(() => {
    void revision;
    load();
  });

  async function sync(pair: string) {
    syncing = pair;
    try {
      await api.syncPair(pair);
      error = null;
    } catch (failure) {
      error = errorText(failure);
    } finally {
      syncing = null;
      await load();
    }
  }

  /** Runs the syncs of a project one after the other. */
  async function syncProject(project: string | null) {
    syncingProject = project ?? "";
    const failures: string[] = [];
    for (const pair of membersOf(project).filter(canSync)) {
      syncing = pair.name;
      try {
        await api.syncPair(pair.name);
      } catch (failure) {
        failures.push(errorText(failure));
      }
    }
    syncing = null;
    syncingProject = null;
    error = failures.length > 0 ? failures.join(" · ") : null;
    await load();
  }

  async function remove(pair: string) {
    try {
      await api.removePair(pair);
      error = null;
    } catch (failure) {
      error = errorText(failure);
    } finally {
      confirming = null;
      await load();
    }
  }

  /** Last folder of a path. */
  function folderName(path: string): string {
    return path.split(/[\\/]/).filter((part) => part !== "").at(-1) ?? path;
  }

  /** "3 min ago", "yesterday"… */
  function ago(milliseconds: number): string {
    const format = new Intl.RelativeTimeFormat(locale(), { numeric: "auto", style: "short" });
    const seconds = Math.round((milliseconds - Date.now()) / 1000);
    const steps: [Intl.RelativeTimeFormatUnit, number][] = [
      ["second", 60],
      ["minute", 60],
      ["hour", 24],
      ["day", 7],
      ["week", 4.35],
      ["month", 12],
    ];
    let value = seconds;
    for (const [unit, size] of steps) {
      if (Math.abs(value) < size) return format.format(Math.round(value), unit);
      value /= size;
    }
    return format.format(Math.round(value), "year");
  }

  function status(pair: PairView): { text: string; tone: string } {
    if (pair.errors.length > 0) return { text: t("Error"), tone: "error" };
    if (pair.active || syncing === pair.name) return { text: t("Syncing"), tone: "muted" };
    if (pair.pending > 0) return { text: t("To settle"), tone: "warn" };
    if (pair.paused) return { text: t("Paused"), tone: "muted" };
    return { text: t("Up to date"), tone: "ok" };
  }

  function detail(pair: PairView): string {
    if (pair.errors.length > 0) return pair.errors[0];
    if (pair.pending > 0) return t("{count} to settle", { count: pair.pending });
    if (pair.paused) return t("paused");
    return t("in step");
  }
</script>

<header class="intro">
  <div class="lead">
    <h1>{t("Your syncs.")}</h1>
    <p>
      {t("Folders Markoo keeps in step between your knowledge base and your Git repositories. It only asks you when both sides changed.")}
    </p>
  </div>
  {#if pairs?.length}
    <dl class="stats">
      <div>
        <dt>{pairs.length}</dt>
        <dd>{pairs.length === 1 ? t("sync") : t("syncs")}</dd>
      </div>
      <div>
        <dt>{toSettle}</dt>
        <dd>{t("to settle")}</dd>
      </div>
    </dl>
  {/if}
</header>

{#if error}
  <p class="error banner" role="alert">{error}</p>
{/if}

{#if pairs === null}
  <p class="muted">{t("Loading…")}</p>
{:else if pairs.length === 0}
  <div class="empty">
    <span class="tile big" aria-hidden="true"><Icon name="folder" size={28} stroke={1.25} /></span>
    <h2>{t("No sync yet")}</h2>
    <p class="muted">{t("A sync keeps a folder of your knowledge base in sync with a folder of a Git repository.")}</p>
    <button class="add" type="button" onclick={() => onnew()}><Icon name="plus" size={14} stroke={2.5} />{t("add a sync")}</button>
  </div>
{:else}
  <label class="search">
    <Icon name="search" />
    <input type="search" placeholder={t("Search your syncs…")} aria-label={t("Search your syncs…")} bind:value={query} />
  </label>

  {#if visible.length === 0}
    <p class="muted">{t("No sync matches “{query}”.", { query })}</p>
  {/if}

  {#each groups as { project, members } (project)}
    {@const all = membersOf(project)}
    {@const title = project ?? t("No project")}
    <section>
      <header class="group-head">
        <h2 class="group">
          <span class="group-title">{title}</span>
          <span class="group-hint">{all.length === 1 ? t("1 sync") : t("{count} syncs", { count: all.length })}</span>
          <span class="rule" aria-hidden="true"></span>
        </h2>
        <span class="group-actions">
          <button
            class="small"
            type="button"
            title={t("Sync every sync of this project that is not paused")}
            aria-label={t("Sync the project {project}", { project: title })}
            disabled={syncing !== null || !all.some(canSync)}
            onclick={() => syncProject(project)}
          >
            <span class:spin={syncingProject === (project ?? "")}><Icon name="refresh" size={14} /></span>
            {t("Sync project")}
          </button>
          {#if project !== null}
            <button
              class="small"
              type="button"
              aria-label={t("Add a sync to {project}", { project })}
              onclick={() => onnew(project)}
            >
              <Icon name="plus" size={14} stroke={2.5} />
              {t("Add a sync")}
            </button>
          {/if}
        </span>
      </header>

      <ul>
        {#each members as pair (pair.name)}
          {@const state = status(pair)}
          <li class:paused={pair.paused}>
            <button class="open" type="button" onclick={() => onopen(pair.name)}>
              <span class="tile" aria-hidden="true"><Icon name="folder" size={24} stroke={1.5} /></span>
              <span class="text">
                <span class="name">{pair.name}</span>
                <span class="sub" title={kbTooltip(pair)}>
                  <span class="kind">{kbLabel(pair.kb_type)}</span>
                  {folderName(pair.kb)} {arrow(pair.direction)} {folderName(pair.repo)}
                  <span class="detail" class:error={pair.errors.length > 0}>– {detail(pair)}</span>
                </span>
              </span>
            </button>

            <span class="meta">
              {#if lastRuns[pair.name] !== undefined}
                <strong>{ago(lastRuns[pair.name])}</strong>
                <em>{t("last sync")}</em>
              {:else}
                <strong>—</strong>
                <em>{t("never synced")}</em>
              {/if}
            </span>

            <span class="pill {state.tone}"><span class="dot" aria-hidden="true"></span>{state.text}</span>

            {#if confirming === pair.name}
              <span class="confirm" role="group" aria-label={t("Remove {name}", { name: pair.name })}>
                <button class="danger small" type="button" onclick={() => remove(pair.name)}>{t("Remove")}</button>
                <button class="small" type="button" onclick={() => (confirming = null)}>{t("Cancel")}</button>
              </span>
            {:else}
              <span class="actions">
                <button
                  class="icon-button"
                  type="button"
                  title={t("Sync now")}
                  aria-label={t("Sync {name} now", { name: pair.name })}
                  disabled={pair.paused || pair.active || syncing !== null || pair.errors.length > 0}
                  onclick={() => sync(pair.name)}
                >
                  <span class:spin={syncing === pair.name || pair.active}><Icon name="refresh" /></span>
                </button>
                <button
                  class="icon-button"
                  type="button"
                  title={t("Settings")}
                  aria-label={t("Settings of {name}", { name: pair.name })}
                  onclick={() => onopen(pair.name, "general")}
                >
                  <Icon name="settings" />
                </button>
                <button
                  class="icon-button"
                  type="button"
                  title={t("Remove this sync (files stay where they are)")}
                  aria-label={t("Remove {name}", { name: pair.name })}
                  onclick={() => (confirming = pair.name)}
                >
                  <Icon name="trash" />
                </button>
              </span>
            {/if}
          </li>
        {/each}
      </ul>
    </section>
  {/each}

  <button class="add" type="button" onclick={() => onnew()}><Icon name="plus" size={14} stroke={2.5} />{t("add a sync")}</button>
{/if}

<style>
  /* Header: serif title, lead text, figures on the right. */
  .intro {
    display: flex;
    justify-content: space-between;
    align-items: flex-end;
    gap: 32px;
    padding-bottom: 24px;
    margin-bottom: 26px;
    border-bottom: 1px solid var(--line);
  }
  .lead h1 {
    font-size: 38px;
    line-height: 1.1;
    margin-bottom: 14px;
  }
  .lead p {
    margin: 0;
    max-width: 600px;
    font-size: 15.5px;
    line-height: 1.6;
    color: var(--muted);
  }
  .stats {
    display: flex;
    margin: 0;
    flex: none;
  }
  .stats div {
    padding: 0 0 0 28px;
    text-align: right;
  }
  .stats div + div {
    margin-left: 28px;
    border-left: 1px solid var(--line);
  }
  .stats dt {
    font-family: var(--serif);
    font-size: 32px;
    font-weight: 400;
    line-height: 1.1;
    letter-spacing: 0.03em;
  }
  .stats dd {
    margin: 4px 0 0;
    font-size: 13.5px;
    letter-spacing: 0.06em;
    color: var(--muted);
  }
  .banner {
    margin: 0 0 16px;
  }

  /* Search field. */
  .search {
    position: relative;
    display: block;
    margin: 0 0 30px;
    font-weight: 400;
    color: var(--muted);
  }
  .search :global(.icon) {
    position: absolute;
    left: 22px;
    top: 50%;
    transform: translateY(-50%);
    pointer-events: none;
  }
  .search input {
    width: 100%;
    font: inherit;
    color: var(--ink);
    height: 60px;
    padding: 0 22px 0 60px;
    border: 1px solid color-mix(in srgb, var(--warn) 22%, var(--line));
    border-radius: 14px;
    background: var(--field);
    font-size: 16.5px;
    letter-spacing: 0.01em;
  }
  .search input::placeholder {
    color: var(--muted);
  }
  .search input:focus-visible {
    outline: none;
    border-color: color-mix(in srgb, var(--accent) 55%, var(--line));
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 16%, transparent);
  }

  /* Group heading: serif title, italic hint, a thin rule. */
  section + section {
    margin-top: 44px;
  }
  .group {
    display: flex;
    align-items: baseline;
    gap: 14px;
    margin: 0 6px 10px;
    line-height: 1.2;
  }
  .group-title {
    flex: none;
    font-family: var(--serif);
    font-size: 22px;
    font-weight: 500;
    white-space: nowrap;
  }
  .group-hint {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 14.5px;
    font-style: italic;
    font-weight: 400;
    color: var(--muted);
  }
  .group-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 14px;
    margin: 0 6px 10px;
  }
  .group-head .group {
    flex: 1;
    min-width: 0;
    margin: 0;
  }
  .group-actions {
    display: flex;
    gap: 8px;
    flex: none;
  }
  .group-actions button {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .rule {
    flex: 1;
    min-width: 24px;
    max-width: 120px;
    height: 1px;
    background: var(--line);
    align-self: center;
  }

  /* Sync cards. */
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 12px;
    grid-template-columns: minmax(0, 1fr);
    container-type: inline-size;
  }
  li {
    display: flex;
    align-items: center;
    gap: 22px;
    padding: 21px 26px;
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: var(--radius);
    box-shadow: var(--shadow);
  }
  li.paused .name,
  li.paused .tile {
    color: var(--muted);
  }
  .open {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 26px;
    padding: 0;
    border: 0;
    background: none;
    text-align: left;
  }
  .open:focus-visible {
    outline: none;
  }
  li:has(.open:focus-visible) {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .open:hover:not(:disabled) .name {
    color: var(--accent);
  }
  .tile {
    display: grid;
    place-items: center;
    width: 62px;
    height: 62px;
    flex: none;
    border-radius: 12px;
    background: var(--tile);
    border: 1px solid var(--line);
    color: var(--accent);
  }
  .text {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .name {
    font-family: var(--serif);
    font-size: 22px;
    font-weight: 500;
    line-height: 1.2;
    letter-spacing: 0.005em;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    transition: color 120ms ease;
  }
  .sub {
    font-size: 15px;
    color: var(--muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .kind {
    margin-right: 6px;
    font-size: 12px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  .detail.error {
    color: var(--danger);
  }
  .meta {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 3px;
    flex: none;
    width: 104px;
    text-align: right;
  }
  .meta strong {
    font-size: 15.5px;
    font-weight: 500;
  }
  .meta em {
    font-size: 14px;
    color: var(--muted);
  }

  /* Status pill: uppercase, spaced letters, a tinted background. */
  .pill {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 9px;
    flex: none;
    min-width: 138px;
    height: 34px;
    padding: 0 16px;
    border-radius: 999px;
    font-size: 13px;
    font-weight: 600;
    letter-spacing: 0.09em;
    text-transform: uppercase;
    background: color-mix(in srgb, currentColor 13%, transparent);
  }
  .pill.ok {
    color: var(--ok);
  }
  .pill.warn {
    color: var(--warn);
  }
  .pill.error {
    color: var(--danger);
  }
  .pill.muted {
    color: var(--muted);
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: currentColor;
  }

  /* Icon buttons. */
  .actions,
  .confirm {
    display: flex;
    align-items: center;
    gap: 12px;
    flex: none;
  }
  .icon-button {
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    padding: 0;
    border: 0;
    border-radius: 10px;
    background: none;
    color: var(--ink);
  }
  .icon-button:hover:not(:disabled) {
    background: var(--tile);
  }
  .icon-button:last-child:hover:not(:disabled) {
    color: var(--danger);
  }
  .spin {
    display: grid;
    animation: spin 1s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .small {
    padding: 6px 12px;
    border-radius: 9px;
    font-size: 13.5px;
  }

  /* Main action. */
  .add {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-top: 22px;
    padding: 16px 26px;
    border: 0;
    border-radius: 12px;
    background: var(--accent);
    color: var(--on-accent);
    font-size: 16px;
    font-weight: 600;
    box-shadow: 0 4px 12px color-mix(in srgb, var(--accent) 20%, transparent);
  }
  @media (prefers-color-scheme: dark) {
    .add {
      box-shadow: 0 4px 12px color-mix(in srgb, var(--accent) 14%, transparent);
    }
  }
  .add:hover:not(:disabled) {
    filter: brightness(1.06);
  }

  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    padding: 48px 32px;
    text-align: center;
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: var(--radius);
    box-shadow: var(--shadow);
  }
  .empty h2 {
    font-family: var(--serif);
    font-size: 22px;
    font-weight: 500;
  }
  .empty p {
    margin: 0;
  }
  .tile.big {
    width: 64px;
    height: 64px;
  }

  /* Narrow window: these come last so they win over the rules above. */
  @container (max-width: 760px) {
    .meta {
      display: none;
    }
  }
  @container (max-width: 560px) {
    .open .tile {
      display: none;
    }
    li {
      padding: 16px 18px;
      gap: 12px;
    }
    .actions {
      gap: 2px;
    }
    .icon-button {
      width: 36px;
      height: 36px;
    }
    .pill {
      min-width: 0;
      padding: 0 12px;
    }
  }
</style>
