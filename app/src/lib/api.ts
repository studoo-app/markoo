import { invoke } from "@tauri-apps/api/core";

export type Direction = "both" | "kb-to-repo" | "repo-to-kb";
export type TargetChanges = "protect" | "overwrite";

export interface PairView {
  name: string;
  kb: string;
  kb_type: string;
  repo: string;
  direction: Direction;
  target_changes: TargetChanges;
  include: string[];
  exclude: string[];
  paused: boolean;
  pending: number;
  errors: string[];
  active: boolean;
}

export interface PlanEntry {
  path: string;
  kind: string;
  side?: "kb" | "repo" | null;
  to?: string | null;
  issue?: string | null;
}

export interface PlanAttachment {
  name: string;
  action: { kind: string; to?: "kb" | "repo" };
}

export interface Plan {
  pair: string;
  direction: Direction;
  paused: boolean;
  entries: PlanEntry[];
  attachments: PlanAttachment[];
  scan_issues: { path: string; message: string }[];
}

export interface Conflict {
  id: string;
  pair: string;
  path: string;
  kind: string;
  kb: string | null;
  base: string | null;
  repo: string | null;
}

export interface Conflicts {
  conflicts: Conflict[];
  excluded: { id: string; pair: string; path: string }[];
  errors: { pair: string; error: string }[];
}

export interface NewPair {
  name: string;
  kb: string;
  kb_type: string | null;
  repo: string;
  direction: Direction;
  target_changes: TargetChanges | null;
}

export interface AppInfo {
  config_file: string;
  data_dir: string;
  embedded_daemon: boolean;
  note: string | null;
}

export type Settings = Record<string, string | boolean | string[]>;

export const api = {
  appInfo: () => invoke<AppInfo>("app_info"),
  listPairs: () => invoke<PairView[]>("list_pairs"),
  pairStatus: (pair: string) => invoke<Plan>("pair_status", { pair }),
  pairDiff: (pair: string, path: string) =>
    invoke<{ diffs: { path: string; patch: string }[] }>("pair_diff", { pair, path }),
  previewNewPair: (fresh: NewPair) => invoke<Plan>("preview_new_pair", { new: fresh }),
  addPair: (fresh: NewPair) => invoke<string[]>("add_pair", { new: fresh }),
  previewSettings: (pair: string, settings: Settings) =>
    invoke<Plan>("preview_pair_settings", { pair, settings }),
  updateSettings: (pair: string, settings: Settings) =>
    invoke<{ warnings: string[] }>("update_pair_settings", { pair, settings }),
  removePair: (pair: string) => invoke<void>("remove_pair", { pair }),
  listConflicts: (pair: string) => invoke<Conflicts>("list_conflicts", { pair }),
  conflictDraft: (id: string) => invoke<string>("conflict_draft", { id }),
  resolve: (
    id: string,
    how: { keep?: string; content?: string; pushToSource?: boolean },
  ) =>
    invoke("resolve_conflict", {
      id,
      keep: how.keep ?? null,
      content: how.content ?? null,
      pushToSource: how.pushToSource ?? false,
    }),
  syncPair: (pair: string) => invoke("sync_pair", { pair }),
  inspectKb: (path: string) => invoke<{ exists: boolean; kb_type: string }>("inspect_kb", { path }),
  inspectRepo: (path: string) =>
    invoke<{ exists: boolean; git: boolean; branch: string | null; top_level: string | null }>(
      "inspect_repo",
      { path },
    ),
  knownKbs: () => invoke<string[]>("known_kbs"),
};

/** The arrow of a direction, as the documentation draws it. */
export function arrow(direction: Direction): string {
  return { both: "⇄", "kb-to-repo": "→", "repo-to-kb": "←" }[direction];
}

/** Label of a knowledge base type. */
export function kbLabel(kbType: string): string {
  return { obsidian: "Obsidian", markdown: "Markdown" }[kbType] ?? kbType;
}

/** A plain sentence for an entry of a plan. */
export function describeEntry(entry: PlanEntry): { mark: string; text: string } {
  switch (entry.kind) {
    case "copy":
      return entry.side === "repo"
        ? { mark: "→", text: "copy to the repo" }
        : { mark: "←", text: "copy to the KB" };
    case "merge":
      return { mark: "⇄", text: "merge both sides" };
    case "trash":
      return { mark: "🗑", text: `move to the trash on the ${entry.side === "repo" ? "repo" : "KB"} side` };
    case "rename":
      return { mark: "↪", text: `rename to ${entry.to ?? "?"}` };
    case "pending":
      return { mark: "⚠", text: entry.issue === "divergence" ? "divergence: waits for you" : "conflict: waits for you" };
    case "not-in-source":
      return { mark: "·", text: "only in the target: left alone" };
    default:
      return { mark: "·", text: entry.kind };
  }
}

export function errorText(error: unknown): string {
  return typeof error === "string" ? error : String(error);
}
