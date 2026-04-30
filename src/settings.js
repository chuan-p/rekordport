import { els, state } from "./app-state.js";
import { STORAGE_KEY } from "./app-state.js";

export function loadSettings() {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) return { ...defaultSettings(), ...JSON.parse(raw) };
  } catch {
    // Ignore malformed storage.
  }
  return defaultSettings();
}

export function defaultSettings() {
  return {
    dbPath: "",
    ignoreSampler: true,
    preset: "wav-auto",
    sourceHandling: "rename",
  };
}

export function normalizePreset(value) {
  switch (value) {
    case "wav-44100":
    case "wav-48000":
    case "wav-auto":
      return "wav-auto";
    case "aiff-44100":
    case "aiff-48000":
    case "aiff-auto":
      return "aiff-auto";
    case "mp3-320":
      return "mp3-320";
    case "m4a-320":
      return "m4a-320";
    default:
      return "wav-auto";
  }
}

export function saveSettings() {
  localStorage.setItem(STORAGE_KEY, JSON.stringify({
    dbPath: els.dbPath.value,
    ignoreSampler: els.includeSampler.checked,
    preset: els.preset.value,
    sourceHandling: els.sourceHandling.value,
  }));
}

export function normalizeSettings() {
  els.dbPath.value = state.settings.dbPath || "";
  els.includeSampler.checked = state.settings.ignoreSampler ?? !Boolean(state.settings.includeSampler);
  els.preset.value = normalizePreset(state.settings.preset);
  els.sourceHandling.value = state.settings.sourceHandling || "rename";
}
