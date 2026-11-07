// Checks GitHub for a newer signed version of Markoo, and installs it.
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { errorText } from "./api";

export type UpdateStatus =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "current" }
  | { kind: "available"; update: Update }
  | { kind: "installing" }
  | { kind: "failed"; message: string };

/** Read by the screens: a change shows up everywhere at once. */
export const updates = $state<{ value: UpdateStatus }>({ value: { kind: "idle" } });

/**
 * Looks for a newer version. `quiet` is for the check at launch: a failure
 * (no network, no release yet) is then not shown.
 */
export async function checkForUpdate(quiet = false): Promise<void> {
  updates.value = { kind: "checking" };
  try {
    const update = await check();
    updates.value = update ? { kind: "available", update } : { kind: "current" };
  } catch (failure) {
    updates.value = quiet ? { kind: "idle" } : { kind: "failed", message: errorText(failure) };
  }
}

/** Downloads, installs, then restarts Markoo. */
export async function installUpdate(update: Update): Promise<void> {
  updates.value = { kind: "installing" };
  try {
    await update.downloadAndInstall();
    await relaunch();
  } catch (failure) {
    updates.value = { kind: "failed", message: errorText(failure) };
  }
}
