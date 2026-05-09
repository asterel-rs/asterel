import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { relaunch } from "@tauri-apps/plugin-process";
import { check } from "@tauri-apps/plugin-updater";

export async function sendNotification(title: string, body: string): Promise<void> {
  try {
    await invoke("send_notification", { params: { title, body } });
  } catch {
    /* permission may be absent */
  }
}

export interface UpdateInfo {
  available: boolean;
  version?: string;
  body?: string;
}

export async function checkForUpdates(): Promise<UpdateInfo> {
  try {
    const update = await check();
    if (update) {
      return {
        available: true,
        version: update.version,
        ...(update.body != null ? { body: update.body } : {}),
      };
    }
    return { available: false };
  } catch {
    return { available: false };
  }
}

export async function installUpdateAndRelaunch(): Promise<void> {
  try {
    const update = await check();
    if (!update) return;
    await update.downloadAndInstall();
    await relaunch();
  } catch (error) {
    console.warn("Updater install failed or is disabled", error);
    throw error;
  }
}

export function onDeepLink(callback: (urls: string[]) => void): Promise<() => void> {
  return listen<string[]>("deep-link-received", (event) => {
    callback(event.payload);
  }).then((unlisten) => unlisten);
}
