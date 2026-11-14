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
    return [...byDay.entries()].map(([key, runs]) => ({
      key,
      label: dayLabel(key),
      count: runs.length,
      rows: collapse(runs),
    }));
  });

  // One row of the list: a run, or a streak of consecutive runs of the same
  // sync that ended the same way (the latest first).
  interface Row {
    key: string;
    pair: string;
    runs: RunRecord[];
    result: Outcome;
  }

  // Consecutive runs of the same sync with the same outcome make one row, so
  // a sync that runs every few minutes without news does not fill the screen.
  function collapse(runs: ActivityEntry[]): Row[] {
    const rows: Row[] = [];
    for (const entry of runs) {
      const result = outcome(entry.run);
      const last = rows.at(-1);
      if (last && last.pair === entry.pair && sameOutcome(last.result, result)) {
        last.runs.push(entry.run);
      } else {
        rows.push({ key: entry.pair + ":" + entry.run.id, pair: entry.pair, runs: [entry.run], result });
      }
    }
    return rows;
  }

  function sameOutcome(a: Outcome, b: Outcome): boolean {
    return a.tone === b.tone && a.text === b.text;
  }

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

  // "1:04 – 1:54 PM" for a streak, from its first run to its latest.
  function timeRange(runs: RunRecord[]): string {
    const latest = runs[0].started_at;
    if (runs.length === 1) return time(latest);
    const format = new Intl.DateTimeFormat(locale(), { timeStyle: "short" });
    return format.formatRange(runs.at(-1)!.started_at, latest);
  }

  // The duration of a run, or the average one of a streak.
  function rowDuration(runs: RunRecord[]): string {
    const total = runs.reduce((sum, run) => sum + run.duration_ms, 0);
    const text = duration(Math.round(total / runs.length));
    return runs.length > 1 ? `~${text}` : text;
  }

  function triggers(runs: RunRecord[]): string {
    return [...new Set(runs.map((run) => run.trigger))].join(", ");
  }

  function duration(milliseconds: number): string {
    return milliseconds < 1000 ? `${milliseconds} ms` : `${(milliseconds / 1000).toFixed(1)} s`;
  }

  interface Outcome {
    label: string;
    text: string;
    /** The parts of the text, the skipped ones flagged to stand out. */
    parts: { text: string; flagged: boolean }[];
    tone: string;
  }

  // What the run did, and whether it went well.
  function outcome(run: RunRecord): Outcome {
    if (run.error) {
      return { label: t("Failed"), text: run.error, parts: [{ text: run.error, flagged: false }], tone: "error" };
    }
    const report = run.report;
    const parts = report
      ? [
          report.written.length > 0 && { text: t("{count} written", { count: report.written.length }), flagged: false },
          report.trashed.length > 0 && { text: t("{count} to the trash", { count: report.trashed.length }), flagged: false },
          report.pending.length > 0 && { text: t("{count} to settle", { count: report.pending.length }), flagged: true },
          report.skipped.length > 0 && { text: t("{count} skipped", { count: report.skipped.length }), flagged: true },
        ].filter((part) => part !== false)
      : [];
    const text = parts.map((part) => part.text).join(" · ");
    if (parts.length === 0) {
      return { label: t("No change"), text: t("nothing to do"), parts: [{ text: t("nothing to do"), flagged: false }], tone: "muted" };
    }
    if (report && report.pending.length > 0) return { label: t("To settle"), text, parts, tone: "warn" };
    return { label: t("Synced"), text, parts, tone: "ok" };
  }

  const icons: Record<string, IconName> = { ok: "check", warn: "clock", error: "alert", muted: "minus" };
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
    <span class="tile" aria-hidden="true"><Icon name="activity" size={28} stroke={1.25} /></span>
    <h2>{t("Nothing yet")}</h2>
    <p class="muted">{t("Syncs appear here once they have written, moved or flagged a file.")}</p>
  </div>
{:else}
  {#each days as day (day.key)}
    <section>
      <h2 class="group">
        <span class="group-title">{day.label}</span>
        <span class="group-hint">{day.count} {day.count === 1 ? t("run") : t("runs")}</span>
        <span class="rule" aria-hidden="true"></span>
      </h2>
      <ul>
        {#each day.rows as row (row.key)}
          <li class={row.result.tone}>
            <button
              class="row"
              type="button"
              onclick={() => onopen(row.pair)}
              aria-label="{labelOf(pairs, row.pair)} · {row.result.label} · {row.result.text} · {timeRange(row.runs)}"
            >
              <span class="status {row.result.tone}" title={row.result.label} aria-hidden="true">
                <Icon name={icons[row.result.tone]} size={16} stroke={1.8} />
              </span>
              <span class="when">
                {timeRange(row.runs)}
              </span>
              <span class="name">{labelOf(pairs, row.pair)}</span>
              <span class="summary" title={row.result.text}>
                {#if row.runs.length > 1}
                  <span class="count" title={t("{count} identical runs", { count: row.runs.length })}>×{row.runs.length}</span>
                {/if}
                {#each row.result.parts as part, index (index)}
                  {#if index > 0}<span class="sep" aria-hidden="true">·</span>{/if}
                  <span class:flagged={part.flagged}>{part.text}</span>
                {/each}
              </span>
              <span class="meta">{rowDuration(row.runs)} · {t("via {trigger}", { trigger: triggers(row.runs) })}</span>
              <span class="chevron" aria-hidden="true">›</span>
            </button>
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
    padding-bottom: 18px;
    margin-bottom: 22px;
    border-bottom: 1px solid var(--line);
  }
  .lead h1 {
    font-size: 34px;
    line-height: 1.1;
    margin-bottom: 8px;
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
    margin-top: 28px;
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

  /* One card per day, one line per run (or streak of identical runs). */
  ul {
    list-style: none;
    margin: 0;
    padding: 4px 0;
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: var(--radius);
    box-shadow: var(--shadow);
    container-type: inline-size;
  }
  li + li {
    border-top: 1px solid var(--line);
  }
  .row {
    width: 100%;
    display: grid;
    grid-template-columns: 20px 9.5em minmax(8em, 15em) minmax(0, 1fr) auto 10px;
    align-items: center;
    column-gap: 16px;
    padding: 9px 18px;
    border: 0;
    border-radius: 0;
    background: none;
    text-align: left;
    font-size: 14px;
    line-height: 1.35;
  }
  li:first-child .row {
    border-radius: calc(var(--radius) - 4px) calc(var(--radius) - 4px) 0 0;
  }
  li:last-child .row {
    border-radius: 0 0 calc(var(--radius) - 4px) calc(var(--radius) - 4px);
  }
  .row:hover:not(:disabled) {
    background: color-mix(in srgb, var(--tile) 70%, transparent);
  }
  .row:hover .name,
  .row:hover .chevron {
    color: var(--accent);
  }
  .row:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
  /* Failures are tinted: they are what the eye should find first. */
  li.error .row {
    background: color-mix(in srgb, var(--danger) 7%, transparent);
  }
  li.error .row:hover {
    background: color-mix(in srgb, var(--danger) 11%, transparent);
  }
  .status {
    display: grid;
    place-items: center;
    color: var(--muted);
  }
  .status.ok {
    color: var(--ok);
  }
  .status.warn {
    color: var(--warn);
  }
  .status.error {
    color: var(--danger);
  }
  .status.muted {
    opacity: 0.55;
  }
  .when {
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    color: var(--ink);
  }
  .name {
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    transition: color 120ms ease;
  }
  .summary {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--muted);
  }
  .sep {
    margin: 0 0.4em;
  }
  .flagged {
    color: var(--warn);
    font-weight: 500;
  }
  li.error .summary {
    color: var(--danger);
  }
  .count {
    display: inline-block;
    margin-right: 8px;
    padding: 0 7px;
    border-radius: 999px;
    background: var(--tile);
    border: 1px solid var(--line);
    font-size: 12.5px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    color: var(--ink);
  }
  .meta {
    font-size: 13px;
    color: var(--muted);
    white-space: nowrap;
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .chevron {
    font-size: 18px;
    line-height: 1;
    color: var(--muted);
    opacity: 0.6;
  }
  /* Runs with nothing to tell stay quiet. */
  li.muted .when,
  li.muted .name {
    color: var(--muted);
    font-weight: 400;
  }
  .tile {
    display: grid;
    place-items: center;
    width: 64px;
    height: 64px;
    border-radius: 12px;
    background: var(--tile);
    border: 1px solid var(--line);
    color: var(--accent);
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
  @container (max-width: 760px) {
    .row {
      grid-template-columns: 20px 9.5em minmax(6em, 12em) minmax(0, 1fr) 10px;
    }
    .meta {
      display: none;
    }
  }
  @container (max-width: 520px) {
    .row {
      grid-template-columns: 20px minmax(0, 1fr) auto 10px;
      grid-template-areas: "status name when chevron" "status summary summary chevron";
      row-gap: 2px;
    }
    .status {
      grid-area: status;
    }
    .name {
      grid-area: name;
    }
    .summary {
      grid-area: summary;
    }
    .chevron {
      grid-area: chevron;
    }
    .when {
      grid-area: when;
      font-size: 13px;
      color: var(--muted);
    }
  }
</style>
