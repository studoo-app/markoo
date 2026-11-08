<script lang="ts">
  import { locale, t } from "../lib/i18n.svelte";
  import {
    api,
    errorText,
    labelOf,
    type ActivityEntry,
    type PairView,
    type RunRecord,
  } from "../lib/api";
  import Icon, { type IconName } from "../lib/Icon.svelte";

  let {
    revision,
    onopen,
  }: { revision: number; onopen: (pair: string) => void } = $props();

  let entries = $state<ActivityEntry[] | null>(null);
  // The syncs, to show runs by their label rather than their identifier.
  let pairs = $state<PairView[]>([]);
  let error = $state<string | null>(null);
  // Identifier of the sync shown; "" means every sync.
  let filter = $state("");

  const pairIds = $derived(
    [...new Set((entries ?? []).map((entry) => entry.pair))].sort((a, b) =>
      labelOf(pairs, a).localeCompare(labelOf(pairs, b), locale()),
    ),
  );
  const visible = $derived(
    (entries ?? []).filter((entry) => filter === "" || entry.pair === filter),
  );
  const failures = $derived(visible.filter((entry) => entry.run.error).length);

  // Runs grouped by day, the latest day first (entries already come sorted).
  const days = $derived.by(() => {
    const byDay = new Map<string, ActivityEntry[]>();
    for (const entry of visible) {
      const key = new Date(entry.run.started_at).toDateString();
      byDay.set(key, [...(byDay.get(key) ?? []), entry]);
    }
    return [...byDay.entries()].map(([key, runs]) => ({ key, label: dayLabel(key), runs }));
  });

  async function load() {
    try {
      const [runs, listed] = await Promise.all([api.activity(), api.listPairs().catch(() => pairs)]);
      entries = runs;
      pairs = listed;
      error = null;
    } catch (failure) {
      error = errorText(failure);
    }
  }

  $effect(() => {
    void revision;
    load();
  });

  function dayLabel(key: string): string {
    const day = new Date(key);
    const today = new Date();
    const yesterday = new Date(today);
    yesterday.setDate(today.getDate() - 1);
    if (day.toDateString() === today.toDateString()) return t("Today");
    if (day.toDateString() === yesterday.toDateString()) return t("Yesterday");
    return day.toLocaleDateString(locale(), { weekday: "long", day: "numeric", month: "long" });
  }

  function time(milliseconds: number): string {
    return new Date(milliseconds).toLocaleTimeString(locale(), { timeStyle: "short" });
  }

  function duration(milliseconds: number): string {
    return milliseconds < 1000 ? `${milliseconds} ms` : `${(milliseconds / 1000).toFixed(1)} s`;
  }

  // What the run did, and whether it went well.
  function outcome(run: RunRecord): { label: string; text: string; tone: string; icon: IconName } {
    if (run.error) return { label: t("Failed"), text: run.error, tone: "error", icon: "alert" };
    const report = run.report;
    const parts = report
      ? [
          report.written.length > 0 && t("{count} written", { count: report.written.length }),
          report.trashed.length > 0 && t("{count} to the trash", { count: report.trashed.length }),
          report.pending.length > 0 && t("{count} to settle", { count: report.pending.length }),
          report.skipped.length > 0 && t("{count} skipped", { count: report.skipped.length }),
        ].filter((part): part is string => typeof part === "string")
      : [];
    if (parts.length === 0) return { label: t("No change"), text: t("nothing to do"), tone: "muted", icon: "minus" };
    if (report && report.pending.length > 0) {
      return { label: t("To settle"), text: parts.join(" · "), tone: "warn", icon: "clock" };
    }
    return { label: t("Synced"), text: parts.join(" · "), tone: "ok", icon: "check" };
  }
</script>

<header class="intro">
  <div class="lead">
    <h1>{t("Activity.")}</h1>
    <p>{t("Every sync that ran, across all your syncs, the latest first. Open one to see its history.")}</p>
  </div>
  {#if entries?.length}
    <dl class="stats">
      <div>
        <dt>{visible.length}</dt>
        <dd>{visible.length === 1 ? t("run") : t("runs")}</dd>
      </div>
      <div>
        <dt>{failures}</dt>
        <dd>{failures === 1 ? t("failure") : t("failures")}</dd>
      </div>
    </dl>
  {/if}
</header>

{#if error}
  <p class="error banner" role="alert">{error}</p>
{/if}

{#if pairIds.length > 1}
  <div class="filters" role="group" aria-label={t("Filter by sync")}>
    <button type="button" class:active={filter === ""} onclick={() => (filter = "")}>{t("All syncs")}</button>
    {#each pairIds as id (id)}
      <button type="button" class:active={filter === id} onclick={() => (filter = id)}>{labelOf(pairs, id)}</button>
    {/each}
  </div>
{/if}

{#if entries === null}
  <p class="muted">{t("Loading…")}</p>
{:else if visible.length === 0}
  <div class="empty">
    <span class="tile big" aria-hidden="true"><Icon name="activity" size={28} stroke={1.25} /></span>
    <h2>{t("Nothing yet")}</h2>
    <p class="muted">{t("Syncs appear here once they have written, moved or flagged a file.")}</p>
  </div>
{:else}
  {#each days as day (day.key)}
    <section>
      <h2 class="group">
        <span class="group-title">{day.label}</span>
        <span class="group-hint">{day.runs.length} {day.runs.length === 1 ? t("run") : t("runs")}</span>
        <span class="rule" aria-hidden="true"></span>
      </h2>
      <ul>
        {#each day.runs as entry (entry.pair + ":" + entry.run.id)}
          {@const result = outcome(entry.run)}
          <li>
            <button class="open" type="button" onclick={() => onopen(entry.pair)}>
              <span class="tile {result.tone}" aria-hidden="true"><Icon name={result.icon} size={22} stroke={1.6} /></span>
              <span class="text">
                <span class="name">{labelOf(pairs, entry.pair)}</span>
                <span class="sub" class:error={result.tone === "error"} title={result.text}>{result.text}</span>
              </span>
            </button>
            <span class="meta">
              <strong>{time(entry.run.started_at)}</strong>
              <em>{duration(entry.run.duration_ms)} · {t("via {trigger}", { trigger: entry.run.trigger })}</em>
            </span>
            <span class="pill {result.tone}"><span class="dot" aria-hidden="true"></span>{result.label}</span>
          </li>
        {/each}
      </ul>
    </section>
  {/each}
{/if}

<style>
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

  /* Filter chips. */
  .filters {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin-bottom: 30px;
  }
  .filters button {
    padding: 7px 16px;
    border: 1px solid var(--line);
    border-radius: 999px;
    background: var(--field);
    font-size: 14px;
    color: var(--muted);
  }
  .filters button:hover:not(:disabled) {
    border-color: var(--line);
    color: var(--ink);
  }
  .filters button.active {
    background: color-mix(in srgb, var(--accent) 11%, transparent);
    border-color: transparent;
    color: var(--accent-ink);
    font-weight: 500;
  }

  section + section {
    margin-top: 40px;
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
  .rule {
    flex: 1;
    min-width: 24px;
    max-width: 120px;
    height: 1px;
    background: var(--line);
    align-self: center;
  }

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
    color: var(--muted);
  }
  .tile.ok {
    color: var(--ok);
  }
  .tile.warn {
    color: var(--warn);
  }
  .tile.error {
    color: var(--danger);
  }
  .tile.big {
    width: 64px;
    height: 64px;
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
  .sub.error {
    color: var(--danger);
  }
  .meta {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 3px;
    flex: none;
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

  /* Narrow window: these come last so they win over the rules above. */
  @container (max-width: 620px) {
    .meta em {
      display: none;
    }
    .pill {
      min-width: 0;
    }
  }
</style>
