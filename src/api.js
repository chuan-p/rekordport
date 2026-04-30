import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { IS_WINDOWS } from "./app-state.js";

export const invokeCommand = invoke;
export const listenEvent = listen;
export const assetSrc = convertFileSrc;

export async function openPathInFileManager(path) {
  await invoke("open_path_in_file_manager", { path });
}

export async function openExternalUrl(url) {
  await invoke("open_external_url", { url });
}

export async function resolvePreviewPath(path) {
  return invoke("prepare_preview_path", { path });
}

export function normalizePreviewAssetPath(path) {
  const value = String(path || "");
  return IS_WINDOWS ? value.replace(/\\/g, "/") : value;
}
