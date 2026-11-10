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

/** A sample sync; its label is made from its project and name. */
function sample(fields: Omit<PairView, "label" | keyof typeof options>): PairView {
  const label = fields.project === null ? fields.name : `${fields.project}/${fields.name}`;
  return { ...fields, label, ...options };
}

const pairs: PairView[] = [
  sample({
    id: "clara", name: "Architecture", project: "Clara",
    kb: "~/Obsidian/Knowledge/Projets/Clara/Architecture", kb_type: "obsidian",
    repo: "~/Projets/lerebours/clara/docs/architecture", direction: "both", target_changes: "protect",
    include: ["**/*.md"], exclude: [], paused: false, pending: 1, errors: [], active: false,
  }),
  sample({
    id: "clara-documentation", name: "Documentation", project: "Clara",
    kb: "~/Obsidian/Knowledge/Projets/Clara/Documentation", kb_type: "obsidian",
    repo: "~/Projets/lerebours/clara/documentation", direction: "kb-to-repo",
    target_changes: "protect", include: ["**/*.md"], exclude: [], paused: false, pending: 0,
    errors: [], active: false,
  }),
  sample({
    id: "markoo-documentation", name: "Documentation", project: "Markoo",
    kb: "~/Obsidian/Knowledge/Projets/Markoo/Documentation", kb_type: "obsidian",
    repo: "~/Projets/opensource/markoo/documentation", direction: "repo-to-kb",
    target_changes: "protect", include: ["**/*.md"], exclude: [], paused: false, pending: 0,
    errors: [], active: true,
  }),
  sample({
    id: "notes-api", name: "notes-api", project: null, kb: "~/notes/api", kb_type: "markdown",
    repo: "~/api/docs", direction: "kb-to-repo", target_changes: "protect", include: ["**/*.md"],
    exclude: [], paused: true, pending: 0, errors: [], active: false,
  }),
  sample({
    id: "markoo-skill", name: "Skill Claude (prod)", project: "Markoo",
    kb: "~/Obsidian/Knowledge/Projets/Markoo/Skill", kb_type: "obsidian",
    repo: "~/Projets/opensource/markoo/skills/markoo", direction: "kb-to-repo",
    target_changes: "protect", include: ["**/*.md"], exclude: [], paused: false, pending: 0,
    errors: [], active: false,
  }),
];

/** The sync a command names by its identifier. */
function find(id: string | undefined): PairView | undefined {
  return pairs.find((pair) => pair.id === id);
}

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
    const params = args as { pair?: string; settings?: Record<string, unknown>; new?: { name: string; project: string } };
    switch (command) {
      case "app_info":
        return { config_file: "~/markoo.toml", data_dir: "~/data", embedded_daemon: true, note: null };
      case "list_pairs":
        return pairs;
      case "pair_status":
        return { ...plan, pair: find(params.pair)?.id ?? "new" };
      case "add_pair": {
        const fresh = params.new!;
        const slug = (text: string) =>
          text.normalize("NFD").replace(/[\u0300-\u036f]/g, "").toLowerCase()
            .replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "");
        let id = `${slug(fresh.project)}-${slug(fresh.name)}`;
        for (let n = 2; find(id); n++) id = `${slug(fresh.project)}-${slug(fresh.name)}-${n}`;
        pairs.push(sample({
          ...pairs[0], id, name: fresh.name, project: fresh.project, pending: 0, active: false,
        }));
        return { id, warnings: [] };
      }
      case "update_pair_settings": {
        // Renames and moves are kept, so that the screens follow them.
        const pair = find(params.pair);
        const settings = params.settings ?? {};
        if (pair && typeof settings.name === "string") pair.name = settings.name;
        if (pair && typeof settings.project === "string") pair.project = settings.project;
        if (pair) pair.label = pair.project === null ? pair.name : `${pair.project}/${pair.name}`;
        if (pair && typeof settings.paused === "boolean") pair.paused = settings.paused;
        return { warnings: [] };
      }
      case "preview_new_pair":
      case "preview_pair_settings":
        return {
          ...plan,
          pair: find(params.pair)?.id ?? "new",
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
        return { conflicts: find(params.pair)?.id === "clara" ? [conflict] : [], excluded: [], errors: [] };
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
              run: "1794158173153",
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
      case "revert_run":
        return {
          pair: "clara", run: 2,
          report: {
            reverts: 2, run: "1794158179999",
            written: [{ path: "adr/001-stack.md", side: "repo" }],
            trashed: [],
            skipped: [{ path: "guide/install.md", reason: "changed since this sync" }],
            pending: [],
          },
        };
      case "activity":
        return [
          {
            pair: "clara",
            run: {
              id: 2, trigger: "interval", started_at: Date.now() - 120000, duration_ms: 84, error: null,
              report: {
                written: [{ path: "adr/001-stack.md", side: "repo" }, { path: "guide/install.md", side: "kb" }],
                trashed: [{ path: "old.md", side: "kb" }],
                skipped: [],
                pending: [],
              },
            },
          },
          {
            pair: "markoo-documentation",
            run: {
              id: 1, trigger: "hook", started_at: Date.now() - 900000, duration_ms: 1240, error: null,
              report: {
                written: [{ path: "README.md", side: "repo" }],
                trashed: [],
                skipped: [{ path: "draft.md", reason: "changed during the sync" }],
                pending: [{}],
              },
            },
          },
          {
            pair: "notes-api",
            run: {
              id: 1, trigger: "interval", started_at: Date.now() - 3600000, duration_ms: 12, report: null,
              error: 'pair "notes-api": the kb folder is unavailable',
            },
          },
        ];
      case "global_settings":
        return {
          interval: "300s", on_startup: true, api_enabled: true, api_port: 4848,
          mcp_http: true, mcp_allow_config_write: false, language: "en",
        };
      case "hooks_state":
        return { git: true, installed: ["post-merge"], manager: null };
      case "known_kbs":
        return ["~/Obsidian/Knowledge", "~/Obsidian/Work"];
      case "inspect_kb":
        return { exists: true, kb_type: "obsidian", overlap: null };
      case "inspect_repo":
        return { exists: true, git: true, branch: "main", top_level: "~/Projets/clara", overlap: null };
      default:
        return null;
    }
  }, { shouldMockEvents: true });
}
