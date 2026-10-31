// Sample data to look at the screens in a plain browser (`?mock`). Never loaded
// by the app itself.
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import type { Conflict, PairView, Plan } from "./api";

const options = {
  assets_dir: "assets",
  links: "markdown",
  frontmatter: "filter",
  frontmatter_drop: ["cssclasses", "aliases", "publish"],
  callouts: "github",
  hooks: { install: ["post-merge"], branches: [] },
} as const satisfies Partial<PairView>;

const pairs: PairView[] = [
  {
    name: "clara", kb: "~/Obsidian/Knowledge/Projets/Clara", kb_type: "obsidian",
    repo: "~/Projets/lerebours/clara/docs", direction: "both", target_changes: "protect",
    include: ["**/*.md"], exclude: [], paused: false, pending: 1, errors: [], active: false, ...options,
  },
  {
    name: "markoo", kb: "~/Obsidian/Knowledge/Projets/Markoo", kb_type: "obsidian",
    repo: "~/Projets/opensource/markoo/documentation", direction: "repo-to-kb",
    target_changes: "protect", include: ["**/*.md"], exclude: [], paused: false, pending: 0,
    errors: [], active: true, ...options,
  },
  {
    name: "notes-api", kb: "~/notes/api", kb_type: "markdown", repo: "~/api/docs",
    direction: "kb-to-repo", target_changes: "protect", include: ["**/*.md"], exclude: [],
    paused: true, pending: 0, errors: [], active: false, ...options,
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
        return { ...plan, pair: params.pair ?? "new" };
      case "preview_new_pair":
      case "preview_pair_settings":
        return {
          ...plan,
          pair: params.pair ?? "new",
          summary: {
            to_kb: 98,
            to_repo: 1,
            git: true,
            untracked_to_kb: 75,
            untracked_examples: [
              "00-administration/charte-nommage.md",
              "01-infrastructure/INDEX.md",
              "02-services/INDEX.md",
              "Obsidian/INDEX.md",
              "README.md",
            ],
          },
        };
      case "pair_diff":
        return { diffs: [{ path: "glossary.md", patch: "@@ -1 +1 @@\n-old line\n+new line\n" }] };
      case "list_conflicts":
        return { conflicts: params.pair === "clara" ? [conflict] : [], excluded: [], errors: [] };
      case "conflict_draft":
        return "# Architecture\n\n<<<<<<< ours\nThe engine uses a queue per pair.\n=======\nThe engine uses one global queue.\n>>>>>>> theirs\n";
      case "preview_filters":
      case "preview_new_pair_filters":
        return {
          tracked_count: 42, excluded_count: 1,
          tracked: ["architecture.md", "adr/001-stack.md"], excluded: ["drafts/ideas.md"],
        };
      case "preview_transform":
        return {
          path: "architecture.md",
          before: "---\naliases: [arch]\ntitle: Architecture\n---\nSee [[glossary]].\n",
          after: "---\ntitle: Architecture\n---\nSee [glossary](./glossary.md).\n",
        };
      case "pair_history":
        return [
          {
            id: 2, trigger: "interval", started_at: Date.now() - 120000, duration_ms: 84, error: null,
            report: {
              written: [
                { path: "adr/001-stack.md", side: "repo" },
                { path: "guide/install.md", side: "kb" },
              ],
              trashed: [{ path: "old.md", side: "kb" }],
              skipped: [],
              pending: [],
            },
          },
          {
            id: 1, trigger: "hook", started_at: Date.now() - 3600000, duration_ms: 12, report: null,
            error: 'pair "clara": the kb folder is unavailable',
          },
        ];
      case "global_settings":
        return {
          interval: "300s", on_startup: true, api_enabled: true, api_port: 4848,
          mcp_http: true, mcp_allow_config_write: false,
        };
      case "hooks_state":
        return { git: true, installed: ["post-merge"], manager: null };
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
