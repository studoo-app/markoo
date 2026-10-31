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
