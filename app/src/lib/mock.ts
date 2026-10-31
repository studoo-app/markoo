// Sample data to look at the screens in a plain browser (`?mock`). Never loaded
// by the app itself.
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import type { Conflict, PairView, Plan } from "./api";

const pairs: PairView[] = [
  {
    name: "clara", kb: "~/Obsidian/Knowledge/Projets/Clara", kb_type: "obsidian",
    repo: "~/Projets/lerebours/clara/docs", direction: "both", target_changes: "protect",
    include: ["**/*.md"], exclude: [], paused: false, pending: 1, errors: [], active: false,
  },
  {
    name: "markoo", kb: "~/Obsidian/Knowledge/Projets/Markoo", kb_type: "obsidian",
    repo: "~/Projets/opensource/markoo/documentation", direction: "repo-to-kb",
    target_changes: "protect", include: ["**/*.md"], exclude: [], paused: false, pending: 0,
    errors: [], active: true,
  },
  {
    name: "notes-api", kb: "~/notes/api", kb_type: "markdown", repo: "~/api/docs",
    direction: "kb-to-repo", target_changes: "protect", include: ["**/*.md"], exclude: [],
    paused: true, pending: 0, errors: [], active: false,
  },
];

const plan: Plan = {
  pair: "clara", direction: "both", paused: false,
  entries: [
    { path: "architecture.md", kind: "pending", issue: "conflict" },
    { path: "adr/001-stack.md", kind: "copy", side: "repo" },
    { path: "guide/install.md", kind: "copy", side: "kb" },
    { path: "glossary.md", kind: "merge" },
    { path: "old.md", kind: "trash", side: "kb" },
  ],
  attachments: [{ name: "schema.png", action: { kind: "copy", to: "repo" } }],
  scan_issues: [{ path: "guide/install.md", message: "link target not found: [[setup]]" }],
};

const conflict: Conflict = {
  id: "c_42", pair: "clara", path: "architecture.md", kind: "conflict",
  kb: "# Architecture\n\nThe engine uses a queue per pair.\n",
  base: "# Architecture\n\nThe engine uses a queue.\n",
  repo: "# Architecture\n\nThe engine uses one global queue.\n",
};

export function installMock(): void {
  mockWindows("main");
  mockIPC((command, args) => {
    const params = args as { pair?: string };
    switch (command) {
      case "app_info":
        return { config_file: "~/markoo.toml", data_dir: "~/data", embedded_daemon: true, note: null };
      case "list_pairs":
        return pairs;
      case "pair_status":
      case "preview_new_pair":
      case "preview_pair_settings":
        return { ...plan, pair: params.pair ?? "new" };
      case "pair_diff":
        return { diffs: [{ path: "glossary.md", patch: "@@ -1 +1 @@\n-old line\n+new line\n" }] };
      case "list_conflicts":
        return { conflicts: params.pair === "clara" ? [conflict] : [], excluded: [], errors: [] };
      case "conflict_draft":
        return "# Architecture\n\n<<<<<<< ours\nThe engine uses a queue per pair.\n=======\nThe engine uses one global queue.\n>>>>>>> theirs\n";
      case "known_kbs":
        return ["~/Obsidian/Knowledge", "~/Obsidian/Work"];
      case "inspect_kb":
        return { exists: true, kb_type: "obsidian" };
      case "inspect_repo":
        return { exists: true, git: true, branch: "main", top_level: "~/Projets/clara" };
      default:
        return null;
    }
  }, { shouldMockEvents: true });
}
