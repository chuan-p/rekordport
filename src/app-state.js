// to import raw files out of src-tauri.
export const appIcon = `${import.meta.env.BASE_URL}app-icon.png`;

export const APP_VERSION = import.meta.env.VITE_APP_VERSION || "0.0.0";
export const DEFAULT_MIN_BIT_DEPTH = 16;
export const RELEASES_URL = "https://github.com/chuan-p/rekordport/releases";
export const STORAGE_KEY = "rekordbox-lossless-scan-settings";
export const SKIPPED_UPDATE_KEY = "rekordport-skipped-update-version";
export const IS_MACOS = /\bMac OS X\b|\bMacintosh\b/.test(navigator.userAgent);
export const IS_WINDOWS = /\bWindows\b/.test(navigator.userAgent);
export const PROFILE = {
  kicker: "Info",
  name: "rekordport",
  handle: "@chuan_p",
  url: "https://www.instagram.com/chuan_p/",
  suffix: "with heavy use of Codex.",
  year: "2026",
};

const $ = (id) => document.getElementById(id);

export function makeOperationId(prefix) {
  if (crypto?.randomUUID) {
    return `${prefix}-${crypto.randomUUID()}`;
  }
  return `${prefix}-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

export const state = {
  tracks: [],
  scanSummary: null,
  selectedIds: new Set(),
  conversionCompleted: false,
  lastBackupDir: "",
  loading: false,
  loadingTask: null,
  preflight: null,
  preflightRequestId: 0,
  scanRequestId: 0,
  scanOperationId: null,
  convertOperationId: null,
  update: {
    status: "checking",
    latestVersion: null,
    url: RELEASES_URL,
    changelog: "",
  },
  scanProgress: {
    active: false,
    phase: "idle",
    current: 0,
    total: 0,
    message: "",
  },
  convertProgress: {
    active: false,
    phase: "idle",
    current: 0,
    total: 0,
    message: "",
  },
  player: {
    track: null,
    currentTime: 0,
    duration: 0,
    loading: false,
    desiredPlaying: false,
    seeking: false,
    seekValue: 0,
    requestSeq: 0,
  },
  settings: null,
  ui: {
    profileOpen: false,
    updateOpen: false,
  },
};

export const els = {
  statusPill: $("status-pill"),
  statusMeta: $("status-meta"),
  scanProgress: $("scan-progress"),
  scanProgressBar: $("scan-progress-bar"),
  scanProgressText: $("scan-progress-text"),
  exportProgress: $("export-progress"),
  exportProgressBar: $("export-progress-bar"),
  exportProgressText: $("export-progress-text"),
  resultsMeta: $("results-meta"),
  body: $("results-body"),
  footerCopy: $("footer-copy"),
  footerNote: $("footer-note"),
  footerError: $("footer-error"),
  dbPath: $("db-path"),
  includeSampler: $("include-sampler"),
  preset: $("preset"),
  sourceHandling: $("source-handling"),
  scan: $("scan"),
  convert: $("convert"),
  selectAll: $("select-all"),
  pickDb: $("pick-db"),
  previewAudio: $("preview-audio"),
  playerPlay: $("player-play"),
  playerSeek: $("player-seek"),
  playerCurrent: $("player-current"),
  playerTotal: $("player-total"),
  profileTrigger: $("profile-trigger"),
  profileBackdrop: $("profile-backdrop"),
  profileCard: $("profile-card"),
  profileAvatarImage: $("profile-avatar-image"),
  profileKicker: $("profile-kicker"),
  profileName: $("profile-name"),
  profileRole: $("profile-role"),
  profileVersion: $("profile-version"),
  profileBackup: $("profile-backup"),
  profileYear: $("profile-year"),
  updateBackdrop: $("update-backdrop"),
  updateDialog: $("update-dialog"),
  updateTitle: $("update-title"),
  updateChangelog: $("update-changelog"),
  updateDownload: $("update-download"),
  updateSkip: $("update-skip"),
};
