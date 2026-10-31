<script lang="ts">
  import { describeEntry, type Plan, type PlanEntry } from "./api";

  let {
    plan,
    onselect,
  }: { plan: Plan; onselect?: (entry: PlanEntry) => void } = $props();

  let attachmentsTo = $derived((side: "kb" | "repo") =>
    plan.attachments.filter((a) => a.action.kind === "copy" && a.action.to === side).length,
  );
</script>

{#if plan.summary}
  {@const summary = plan.summary}
  <p class="totals">
    <strong>{summary.to_kb}</strong> file(s) to the KB ·
    <strong>{summary.to_repo}</strong> to the repo
  </p>
  {#if summary.untracked_to_kb > 0}
    <div class="untracked" role="alert">
      <strong>
        {summary.untracked_to_kb} of the files going to the KB are not tracked by Git in the repo.
      </strong>
      They may have nothing to do with this repository, if an earlier sync left them there for
      example. Check them before going on.
      <ul>
        {#each summary.untracked_examples as path}
          <li class="path">{path}</li>
        {/each}
      </ul>
      {#if summary.untracked_to_kb > summary.untracked_examples.length}
        <p class="more">… and {summary.untracked_to_kb - summary.untracked_examples.length} more.</p>
      {/if}
    </div>
  {/if}
{/if}

{#if plan.entries.length === 0 && plan.attachments.length === 0}
  <p class="muted">Nothing to do: both sides agree.</p>
{:else}
  <ul>
    {#each plan.entries as entry}
      {@const what = describeEntry(entry)}
      <li class:pending={entry.kind === "pending"}>
        <span class="mark" aria-hidden="true">{what.mark}</span>
        {#if onselect}
          <button type="button" class="link" onclick={() => onselect(entry)}>{entry.path}</button>
        {:else}
          <span class="path">{entry.path}</span>
        {/if}
        <span class="muted what">{what.text}</span>
      </li>
    {/each}
  </ul>
  {#if plan.attachments.length > 0}
    <p class="muted hint">
      Attachments: {attachmentsTo("repo")} to the repo, {attachmentsTo("kb")} to the KB,
      {plan.attachments.length} in all.
    </p>
  {/if}
{/if}

{#if plan.scan_issues.length > 0}
  <details>
    <summary class="warn">{plan.scan_issues.length} warning(s)</summary>
    <ul class="issues">
      {#each plan.scan_issues as issue}
        <li><span class="path">{issue.path}</span>: {issue.message}</li>
      {/each}
    </ul>
  </details>
{/if}

<style>
  .totals {
    margin: 0 0 8px;
  }
  .untracked {
    border: 1px solid var(--warn);
    border-radius: 6px;
    padding: 8px 10px;
    margin-bottom: 10px;
  }
  .untracked ul {
    margin-top: 6px;
    font-size: 12px;
  }
  .untracked li {
    display: block;
    padding: 1px 0;
  }
  .more {
    margin: 2px 0 0;
    font-size: 12px;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  li {
    display: flex;
    gap: 8px;
    align-items: baseline;
    padding: 3px 0;
  }
  li.pending .mark {
    color: var(--warn);
  }
  .mark {
    width: 18px;
    flex: none;
    text-align: center;
  }
  .what {
    margin-left: auto;
    flex: none;
    font-size: 12px;
  }
  .link {
    background: none;
    border: 0;
    padding: 0;
    text-align: left;
    text-decoration: underline;
    text-decoration-color: var(--line);
    overflow-wrap: anywhere;
  }
  details {
    margin-top: 8px;
  }
  .issues li {
    display: block;
    font-size: 12px;
  }
</style>
