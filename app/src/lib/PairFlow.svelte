<script lang="ts">
  import { arrow, kbLabel, type Direction } from "./api";
  import obsidianLogo from "./brand/obsidian-black.svg";
  import gitLogo from "./brand/git-black.svg";

  let {
    kbType,
    direction,
    kb,
    repo,
  }: { kbType: string; direction: Direction; kb: string; repo: string } = $props();

  /** Last folder of a path: the name shown next to each logo. */
  function folderName(path: string): string {
    const parts = path.split(/[\\/]/).filter((part) => part !== "");
    return parts.at(-1) ?? path;
  }
</script>

<span class="flow" title={`${kbLabel(kbType)}: ${kb}\n${arrow(direction)}\nGit: ${repo}`}>
  <span class="side">
    <img class="mark kb" src={obsidianLogo} alt="" aria-hidden="true" />
    <span class="dir">{folderName(kb)}</span>
  </span>
  <span class="arrow" aria-hidden="true">{arrow(direction)}</span>
  <span class="side">
    <img class="mark" src={gitLogo} alt="" aria-hidden="true" />
    <span class="dir">{folderName(repo)}</span>
  </span>
</span>

<style>
  .flow {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    color: var(--ink);
    vertical-align: middle;
    min-width: 0;
  }
  .side {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .dir {
    font-size: 17px;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .mark {
    width: 34px;
    height: 34px;
    flex: none;
  }
  /* The gem starts about 4px inside its 34px box: pull the logo back so the gem lines up with the status dot. */
  .kb {
    margin-left: -4px;
  }
  .arrow {
    color: var(--accent);
    font-size: 24px;
    font-weight: 700;
    line-height: 1;
  }
</style>
