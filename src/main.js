import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

// Keep the About card icon inside Vite's public assets so dev mode never tries
// to import raw files out of src-tauri.
const appIcon = `${import.meta.env.BASE_URL}app-icon.png`;

const APP_VERSION = import.meta.env.VITE_APP_VERSION || "0.0.0";
const DEFAULT_MIN_BIT_DEPTH = 16;
const RELEASES_URL = "https://github.com/chuan-p/rekordport/releases";
const STORAGE_KEY = "rekordbox-lossless-scan-settings";
const SKIPPED_UPDATE_KEY = "rekordport-skipped-update-version";
const IS_MACOS = /\bMac OS X\b|\bMacintosh\b/.test(navigator.userAgent);
const IS_WINDOWS = /\bWindows\b/.test(navigator.userAgent);
const PROFILE = {
  kicker: "Info",
  name: "rekordport",
  handle: "@chuan_p",
  url: "https://www.instagram.com/chuan_p/",
  suffix: "with heavy use of Codex.",
  year: "2026",
};

const $ = (id) => document.getElementById(id);

const state = {
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
  settings: loadSettings(),
  ui: {
    profileOpen: false,
    updateOpen: false,
  },
};

const els = {
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

function loadSettings() {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) return { ...defaultSettings(), ...JSON.parse(raw) };
  } catch {
    // Ignore malformed storage.
  }
  return defaultSettings();
}

function defaultSettings() {
  return {
    dbPath: "",
    ignoreSampler: true,
    preset: "wav-auto",
    sourceHandling: "rename",
  };
}

function normalizePreset(value) {
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

function saveSettings() {
  localStorage.setItem(STORAGE_KEY, JSON.stringify({
    dbPath: els.dbPath.value,
    ignoreSampler: els.includeSampler.checked,
    preset: els.preset.value,
    sourceHandling: els.sourceHandling.value,
  }));
}

function normalizeSettings() {
  els.dbPath.value = state.settings.dbPath || "";
  els.includeSampler.checked = state.settings.ignoreSampler ?? !Boolean(state.settings.includeSampler);
  els.preset.value = normalizePreset(state.settings.preset);
  els.sourceHandling.value = state.settings.sourceHandling || "rename";
}

function setStatus(text, meta = "", kind = "ready") {
  els.statusPill.textContent = text;
  els.statusPill.dataset.kind = kind;
  els.statusMeta.textContent = meta;
  els.statusMeta.hidden = !meta;
}

function setScanProgress(progress = {}) {
  state.scanProgress = {
    ...state.scanProgress,
    ...progress,
  };
  renderScanProgress();
}

function clearScanProgress() {
  state.scanProgress = {
    active: false,
    phase: "idle",
    current: 0,
    total: 0,
    message: "",
  };
  renderScanProgress();
}

function setConvertProgress(progress = {}) {
  state.convertProgress = {
    ...state.convertProgress,
    ...progress,
  };
  renderConvertProgress();
}

function clearConvertProgress() {
  state.convertProgress = {
    active: false,
    phase: "idle",
    current: 0,
    total: 0,
    message: "",
  };
  renderConvertProgress();
}

function clearConversionMessage() {
  els.footerCopy.hidden = true;
  els.footerCopy.textContent = "";
}

function renderConvertButton() {
  const selectedCount = state.selectedIds.size;
  const convertReady = state.preflight ? state.preflight.convert_ready : true;

  if (!state.loading && selectedCount === 0 && state.conversionCompleted) {
    els.convert.textContent = "Converted";
    els.convert.disabled = true;
    return;
  }

  els.convert.textContent = "Convert Selected";
  els.convert.disabled = selectedCount === 0 || state.loading || !convertReady;
}

function renderProfileCard() {
  if (
    !els.profileCard ||
    !els.profileBackdrop ||
    !els.profileTrigger ||
    !els.profileAvatarImage ||
    !els.profileKicker ||
    !els.profileName ||
    !els.profileRole ||
    !els.profileVersion ||
    !els.profileBackup ||
    !els.profileYear
  ) {
    return;
  }

  els.profileAvatarImage.src = appIcon;
  els.profileKicker.textContent = PROFILE.kicker;
  els.profileName.textContent = PROFILE.name;
  els.profileRole.innerHTML = `made by <a href="${escapeHtml(PROFILE.url)}" data-external-link="${escapeHtml(PROFILE.url)}">${escapeHtml(PROFILE.handle)}</a> ${escapeHtml(PROFILE.suffix)}`;
  els.profileVersion.textContent = `Version v${normalizeVersion(APP_VERSION)}`;
  if (state.lastBackupDir) {
    els.profileBackup.hidden = false;
    els.profileBackup.innerHTML = `Backup <button class="footer-link" type="button" data-backup-path="${escapeHtml(state.lastBackupDir)}" title="${escapeHtml(state.lastBackupDir)}">Open</button>`;
  } else {
    els.profileBackup.hidden = true;
    els.profileBackup.textContent = "";
  }
  els.profileYear.textContent = PROFILE.year;

  els.profileBackdrop.hidden = !state.ui.profileOpen;
  els.profileCard.hidden = !state.ui.profileOpen;
  els.profileTrigger.setAttribute("aria-expanded", String(state.ui.profileOpen));
}

function closeProfileCard() {
  if (!state.ui.profileOpen) return;
  state.ui.profileOpen = false;
  renderProfileCard();
}

function toggleProfileCard() {
  state.ui.profileOpen = !state.ui.profileOpen;
  renderProfileCard();
}

function renderScanProgress() {
  if (!els.scanProgress || !els.scanProgressBar || !els.scanProgressText) return;

  const progress = state.scanProgress;
  els.scanProgress.hidden = !progress.active;

  if (!progress.active) {
    els.scanProgressText.textContent = "";
    els.scanProgressBar.removeAttribute("value");
    return;
  }

  els.scanProgressText.textContent = progress.message || "Scanning…";
  if (Number.isFinite(progress.total) && progress.total > 0) {
    els.scanProgressBar.max = String(progress.total);
    els.scanProgressBar.value = String(Math.min(progress.current || 0, progress.total));
  } else {
    els.scanProgressBar.removeAttribute("value");
  }
}

function renderConvertProgress() {
  if (!els.exportProgress || !els.exportProgressBar || !els.exportProgressText) return;

  const progress = state.convertProgress;
  els.exportProgress.hidden = !progress.active;

  if (!progress.active) {
    els.exportProgressText.textContent = "";
    els.exportProgressBar.removeAttribute("value");
    return;
  }

  els.exportProgressText.textContent = progress.message || "Converting…";
  if (Number.isFinite(progress.total) && progress.total > 0) {
    els.exportProgressBar.max = String(progress.total);
    els.exportProgressBar.value = String(Math.min(progress.current || 0, progress.total));
  } else {
    els.exportProgressBar.removeAttribute("value");
  }
}

function setError(message) {
  if (!message) {
    els.footerError.hidden = true;
    els.footerError.textContent = "";
    return;
  }
  els.footerError.hidden = false;
  els.footerError.textContent = message;
}

function existingFileConflictMessage(error) {
  const text = String(error || "");
  if (text.startsWith("source archive already exists, refusing to overwrite: ")) {
    return {
      kind: "archive",
      path: text.slice("source archive already exists, refusing to overwrite: ".length),
    };
  }
  if (text.startsWith("target file already exists: ")) {
    return {
      kind: "output",
      path: text.slice("target file already exists: ".length),
    };
  }
  return null;
}

function promptConflictResolution(conflict) {
  const label = conflict.kind === "archive" ? "archived source file" : "output file";
  const answer = window.prompt(
    `An existing ${label} was found:\n\n${conflict.path}\n\nType "redirect" to keep the old file and create a new numbered file, or type "overwrite" to replace the old file.`,
    "redirect",
  );
  if (answer == null) return null;

  const normalized = answer.trim().toLowerCase();
  if (normalized === "redirect" || normalized === "r") return "redirect";
  if (normalized === "overwrite" || normalized === "o") return "overwrite";

  window.alert('Type "redirect" or "overwrite".');
  return null;
}

function normalizeConflictResolution(value) {
  if (typeof value !== "string") return "error";
  const normalized = value.trim().toLowerCase();
  if (normalized === "redirect" || normalized === "overwrite" || normalized === "error") {
    return normalized;
  }
  return "error";
}

function numberOrZero(value) {
  const numeric = Number(value);
  return Number.isFinite(numeric) ? numeric : 0;
}

function formatNumber(value) {
  return new Intl.NumberFormat("en-US").format(numberOrZero(value));
}

function dependencySourceSummary() {
  if (!state.preflight) return "";
  const sources = [
    state.preflight.sqlcipher_source,
    state.preflight.ffmpeg_source,
  ].filter(Boolean);
  if (!sources.length) return "";
  const bundledCount = sources.filter((source) => String(source).startsWith("bundled sidecar")).length;
  if (bundledCount === sources.length) return "Bundled sidecars";
  if (bundledCount === 0) return "System dependencies";
  return "Mixed dependencies";
}

function environmentSummary() {
  if (!state.preflight) return "Checking local dependencies…";
  if (!(state.preflight.warnings || []).length) {
    const sourceSummary = dependencySourceSummary();
    return `Environment ready · ${state.preflight.os}${sourceSummary ? ` · ${sourceSummary}` : ""} · scan and conversion dependencies are available`;
  }
  return state.preflight.warnings.join(" ");
}

function normalizeVersion(value) {
  return String(value || "")
    .trim()
    .replace(/^v/i, "")
    .split(/[+-]/)[0];
}

function compareVersions(left, right) {
  const leftParts = normalizeVersion(left).split(".").map((part) => Number.parseInt(part, 10) || 0);
  const rightParts = normalizeVersion(right).split(".").map((part) => Number.parseInt(part, 10) || 0);
  const maxLength = Math.max(leftParts.length, rightParts.length);
  for (let index = 0; index < maxLength; index += 1) {
    const diff = (leftParts[index] || 0) - (rightParts[index] || 0);
    if (diff !== 0) return diff;
  }
  return 0;
}

function renderFooterNote() {
  els.footerNote.textContent = environmentSummary();
}

function skippedUpdateVersion() {
  try {
    return localStorage.getItem(SKIPPED_UPDATE_KEY) || "";
  } catch {
    return "";
  }
}

function skipUpdateVersion(version) {
  try {
    localStorage.setItem(SKIPPED_UPDATE_KEY, version);
  } catch {
    // Ignore storage failures; the user can still dismiss this run.
  }
}

function shouldPromptForUpdate() {
  return state.update.status === "available"
    && Boolean(state.update.latestVersion)
    && skippedUpdateVersion() !== state.update.latestVersion;
}

function parseChangelogSections(markdown, currentVersion, latestVersion) {
  const sections = [];
  let current = null;
  let group = null;
  const currentNormalized = normalizeVersion(currentVersion);
  const latestNormalized = normalizeVersion(latestVersion);

  for (const rawLine of String(markdown || "").split(/\r?\n/)) {
    const line = rawLine.trim();
    const versionMatch = line.match(/^##\s+v?([0-9]+(?:\.[0-9]+){1,2})\b/i);
    if (versionMatch) {
      const version = normalizeVersion(versionMatch[1]);
      const inRange = compareVersions(version, currentNormalized) > 0
        && compareVersions(version, latestNormalized) <= 0;
      current = inRange ? { version, groups: [] } : null;
      group = null;
      if (current) sections.push(current);
      continue;
    }

    if (!current) continue;

    const groupMatch = line.match(/^###\s+(.+)$/);
    if (groupMatch) {
      group = { title: groupMatch[1], items: [] };
      current.groups.push(group);
      continue;
    }

    const itemMatch = line.match(/^-\s+(.+)$/);
    if (itemMatch) {
      if (!group) {
        group = { title: "", items: [] };
        current.groups.push(group);
      }
      group.items.push(itemMatch[1].replace(/`([^`]+)`/g, "$1"));
    }
  }

  return sections
    .map((section) => ({
      ...section,
      groups: section.groups.filter((candidate) => candidate.items.length > 0),
    }))
    .filter((section) => section.groups.length > 0);
}

function parseReleaseNotesFallback(markdown, latestVersion) {
  const section = { version: normalizeVersion(latestVersion), groups: [] };
  let group = null;

  for (const rawLine of String(markdown || "").split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line || line.startsWith("#")) continue;
    if (/^rekordport\s+v?[0-9]/i.test(line)) continue;

    const headingMatch = line.match(/^(?:###\s+)?(.+):$/);
    if (headingMatch) {
      group = { title: headingMatch[1], items: [] };
      section.groups.push(group);
      continue;
    }

    const itemMatch = line.match(/^-\s+(.+)$/);
    if (!itemMatch) continue;
    if (!group) {
      group = { title: "", items: [] };
      section.groups.push(group);
    }
    group.items.push(itemMatch[1].replace(/`([^`]+)`/g, "$1"));
  }

  section.groups = section.groups.filter((candidate) => candidate.items.length > 0);
  return section.groups.length > 0 ? [section] : [];
}

function changelogSectionsForUpdate(markdown, currentVersion, latestVersion) {
  const sections = parseChangelogSections(markdown, currentVersion, latestVersion);
  if (sections.length > 0) return sections;
  return parseReleaseNotesFallback(markdown, latestVersion);
}

function renderChangelogSections(sections) {
  els.updateChangelog.textContent = "";
  els.updateChangelog.hidden = sections.length === 0;
  if (!sections.length) return;

  for (const section of sections) {
    const article = document.createElement("article");
    article.className = "update-changelog-version";

    const heading = document.createElement("h4");
    heading.textContent = `v${section.version}`;
    article.append(heading);

    for (const group of section.groups) {
      if (group.title) {
        const groupHeading = document.createElement("h5");
        groupHeading.textContent = group.title;
        article.append(groupHeading);
      }

      const list = document.createElement("ul");
      for (const item of group.items) {
        const listItem = document.createElement("li");
        listItem.textContent = item;
        list.append(listItem);
      }
      article.append(list);
    }

    els.updateChangelog.append(article);
  }
}

function renderUpdateDialog() {
  if (
    !els.updateBackdrop ||
    !els.updateDialog ||
    !els.updateTitle ||
    !els.updateChangelog ||
    !els.updateDownload ||
    !els.updateSkip
  ) {
    return;
  }

  if (!shouldPromptForUpdate()) {
    state.ui.updateOpen = false;
  }

  const open = state.ui.updateOpen && shouldPromptForUpdate();
  els.updateBackdrop.hidden = !open;
  els.updateDialog.hidden = !open;
  if (!open) return;

  els.updateTitle.textContent = `rekordport ${state.update.latestVersion} is ready`;
  renderChangelogSections(changelogSectionsForUpdate(
    state.update.changelog,
    APP_VERSION,
    state.update.latestVersion,
  ));
}

async function checkForUpdates() {
  state.update = { status: "checking", latestVersion: null, url: RELEASES_URL, changelog: "" };
  renderUpdateDialog();

  try {
    const release = await invoke("latest_release");
    const tagName = release?.tag_name || release?.name;
    const latestVersion = normalizeVersion(tagName);
    if (!latestVersion) {
      throw new Error("latest release did not include a tag");
    }
    const latestTag = `v${latestVersion}`;
    state.update = {
      status: compareVersions(latestVersion, APP_VERSION) > 0 ? "available" : "current",
      latestVersion: latestTag,
      url: release?.html_url || RELEASES_URL,
      changelog: release?.changelog || "",
    };
  } catch {
    state.update = { status: "error", latestVersion: null, url: RELEASES_URL, changelog: "" };
  }

  state.ui.updateOpen = shouldPromptForUpdate();
  renderUpdateDialog();
}

function humanBytesLike(value) {
  if (value == null || value === "") return "—";
  const numeric = Number(value);
  if (!Number.isFinite(numeric) || numeric <= 0) return "—";
  return String(numeric);
}

function analysisStateLabel(track) {
  if (track.status !== "converted") return "";
  switch (track.analysis_state) {
    case "none":
      return "Analysis not migrated";
    default:
      return "";
  }
}

function statusLabel(track) {
  return track.status === "converted" ? "Converted" : "Pending";
}

function previewButtonState(track) {
  if (previewTrack()?.id !== track.id) return "play";
  return isPreviewPlaybackActive() ? "pause" : "play";
}

function previewButtonLabel(track) {
  return previewButtonState(track) === "pause" ? "Pause preview" : "Play preview";
}

function formatAudioMeta(track) {
  const bitDepth = humanBytesLike(track.bit_depth);
  const sampleRate = humanBytesLike(track.sample_rate);
  const bitrate = humanBytesLike(track.bitrate);
  const sampleRateValue = Number(sampleRate);
  const sampleRateLabel = sampleRate === "—"
    ? "—"
    : sampleRateValue >= 1000
      ? `${(sampleRateValue / 1000).toFixed(sampleRateValue % 1000 === 0 ? 0 : 1)}k`
      : sampleRate;
  const items = [
    `<span><strong>${escapeHtml(bitDepth)}</strong>-bit</span>`,
    `<span><strong>${escapeHtml(sampleRateLabel)}</strong>Hz</span>`,
    `<span><strong>${escapeHtml(bitrate)}</strong> kbps</span>`,
  ];
  if (track.scan_issue === "wav_extensible") {
    const note = track.scan_note || "WAV header uses WAVE_FORMAT_EXTENSIBLE.";
    items.push(
      `<span class="audio-meta-issue" title="${escapeHtml(note)}">wav_extensible</span>`,
    );
  }
  return `
    <div class="audio-meta">
      ${items.join("")}
    </div>
  `;
}

async function openPathInFileManager(path) {
  await invoke("open_path_in_file_manager", { path });
}

async function openExternalUrl(url) {
  await invoke("open_external_url", { url });
}

async function resolvePreviewPath(path) {
  return invoke("prepare_preview_path", { path });
}

function normalizePreviewAssetPath(path) {
  const value = String(path || "");
  return IS_WINDOWS ? value.replace(/\\/g, "/") : value;
}

function formatTime(value) {
  if (!Number.isFinite(value) || value < 0) return "0:00";
  const totalSeconds = Math.floor(value);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = String(totalSeconds % 60).padStart(2, "0");
  return `${minutes}:${seconds}`;
}

function previewTrack() {
  return state.player.track;
}

function isPreviewPlaybackActive() {
  const audio = els.previewAudio;
  if (!previewTrack()) return false;
  return state.player.desiredPlaying || (!audio.paused && !audio.ended);
}

function nextPreviewRequest() {
  state.player.requestSeq += 1;
  return state.player.requestSeq;
}

async function attemptPreviewPlayback(requestSeq, { reportError = false } = {}) {
  try {
    await els.previewAudio.play();
    if (requestSeq !== state.player.requestSeq) return;
    state.player.loading = false;
    state.player.desiredPlaying = true;
    renderPlayer();
    renderResults();
  } catch (error) {
    if (requestSeq !== state.player.requestSeq || !state.player.desiredPlaying) return;
    state.player.loading = false;
    renderPlayer();
    renderResults();
    if (reportError) {
      setError(`This file could not be previewed: ${String(error)}`);
    }
  }
}

function renderPlayer() {
  const audio = els.previewAudio;
  const track = previewTrack();
  const hasTrack = Boolean(track);
  const isPlaying = hasTrack && isPreviewPlaybackActive();
  const displayedTime = state.player.seeking ? state.player.seekValue : state.player.currentTime;

  els.playerPlay.textContent = !hasTrack
    ? "No track selected"
    : isPlaying
        ? "Pause"
        : "Play";
  els.playerPlay.disabled = !hasTrack;
  els.playerSeek.disabled = !hasTrack || state.player.loading || !Number.isFinite(state.player.duration) || state.player.duration <= 0;
  els.playerSeek.max = String(Math.max(state.player.duration || 0, 0));
  els.playerSeek.value = String(Math.min(Math.max(displayedTime || 0, 0), state.player.duration || 0));
  els.playerCurrent.textContent = formatTime(displayedTime);
  els.playerTotal.textContent = formatTime(state.player.duration);
}

function syncPreviewRow() {
  renderResults();
  renderPlayer();
}

async function loadPreviewTrack(track, autoplay = true) {
  if (!track?.full_path) {
    setError("This track does not have a playable file path.");
    return;
  }

  const previousPlayerState = { ...state.player };
  Object.assign(state.player, {
    track,
    currentTime: 0,
    duration: 0,
    loading: autoplay,
    desiredPlaying: autoplay,
    seeking: false,
    seekValue: 0,
  });
  const requestSeq = nextPreviewRequest();
  renderPlayer();
  renderResults();

  let playablePath;
  try {
    playablePath = await resolvePreviewPath(track.full_path);
  } catch (error) {
    Object.assign(state.player, previousPlayerState);
    renderPlayer();
    renderResults();
    setStatus("Error", "", "error");
    setError(`This file could not be previewed: ${String(error)}`);
    return;
  }

  const src = convertFileSrc(normalizePreviewAssetPath(playablePath));
  els.previewAudio.src = src;
  els.previewAudio.load();
  renderPlayer();
  renderResults();

  if (!autoplay) {
    state.player.loading = false;
    state.player.desiredPlaying = false;
    renderPlayer();
    renderResults();
    return;
  }

  await attemptPreviewPlayback(requestSeq);
}

function previewErrorMessage() {
  const mediaError = els.previewAudio?.error;
  if (!mediaError) return "Unknown media error";

  const codeMap = {
    1: "Playback was aborted or cancelled",
    2: "The WebView failed to fetch the audio data",
    3: "Audio decoding failed",
    4: "The current WebView cannot load this audio source",
  };
  const detail = codeMap[mediaError.code] || "Unknown media error";
  const src = els.previewAudio?.currentSrc || els.previewAudio?.src || "";
  return src ? `${detail}：${src}` : detail;
}

function togglePreviewPlay() {
  const audio = els.previewAudio;
  if (!previewTrack()) return;

  if (isPreviewPlaybackActive()) {
    nextPreviewRequest();
    state.player.desiredPlaying = false;
    state.player.loading = false;
    audio.pause();
    renderPlayer();
    renderResults();
    return;
  }

  state.player.desiredPlaying = true;
  state.player.loading = true;
  renderPlayer();
  renderResults();
  setError("");
  const requestSeq = nextPreviewRequest();
  attemptPreviewPlayback(requestSeq).catch(() => {
    state.player.loading = false;
    renderPlayer();
    renderResults();
  });
}

function waitForPreviewAudioRelease(audio) {
  return new Promise((resolve) => {
    let settled = false;
    const settleDelay = IS_WINDOWS ? 1200 : 150;
    const finish = () => {
      if (settled) return;
      settled = true;
      window.clearTimeout(timeout);
      audio.removeEventListener("emptied", finish);
      window.setTimeout(() => {
        window.requestAnimationFrame(() => resolve());
      }, settleDelay);
    };
    const timeout = window.setTimeout(finish, settleDelay + 300);
    audio.addEventListener("emptied", finish, { once: true });
    audio.load();
  });
}

async function releasePreviewAudio() {
  const audio = els.previewAudio;
  const previous = {
    track: state.player.track,
    wasPlaying: Boolean(previewTrack() && isPreviewPlaybackActive()),
  };
  const hadSource = Boolean(audio.currentSrc || audio.src);

  nextPreviewRequest();
  audio.pause();
  audio.removeAttribute("src");
  state.player.track = null;
  state.player.currentTime = 0;
  state.player.duration = 0;
  state.player.loading = false;
  state.player.desiredPlaying = false;
  state.player.seeking = false;
  state.player.seekValue = 0;
  renderPlayer();
  renderResults();

  if (hadSource) {
    await waitForPreviewAudioRelease(audio);
  } else {
    audio.load();
  }

  return previous;
}

function currentScanRequestMatches(requestId, requestedPath, ignoreSamplerChecked) {
  return requestId === state.scanRequestId
    && requestedPath === els.dbPath.value
    && ignoreSamplerChecked === els.includeSampler.checked;
}

async function invalidateScanState({ clearPreflight = false } = {}) {
  state.scanRequestId += 1;
  state.tracks = [];
  state.scanSummary = null;
  state.selectedIds = new Set();
  state.conversionCompleted = false;
  clearConversionMessage();
  if (clearPreflight) {
    state.preflight = null;
    renderFooterNote();
  } else {
    applyPreflightState();
  }
  setStatus("Ready", "", "ready");
  await releasePreviewAudio();
  renderChips();
  renderResults();
}

function summarizeTracks(tracks) {
  const flac = tracks.filter((t) => t.file_type === "FLAC").length;
  const alac = tracks.filter((t) => t.file_type === "ALAC").length;
  const wavExtensible = tracks.filter((t) => t.scan_issue === "wav_extensible").length;
  const hiRes = tracks.filter(
    (t) => (t.file_type === "WAV" || t.file_type === "AIFF") && t.scan_issue !== "wav_extensible",
  ).length;
  return {
    library_total: tracks.length,
    candidate_total: tracks.length,
    total: tracks.length,
    flac,
    alac,
    hi_res: hiRes,
    wav_extensible: wavExtensible,
    m4a_candidates: alac,
    unreadable_m4a: 0,
    non_alac_m4a: 0,
    sampler_included: false,
    min_bit_depth: DEFAULT_MIN_BIT_DEPTH,
    db_path: "",
  };
}

function normalizeScanSummary(summary, tracks = []) {
  const fallback = summarizeTracks(tracks);
  const source = summary || {};
  return {
    library_total: numberOrZero(source.library_total ?? fallback.library_total),
    candidate_total: numberOrZero(source.candidate_total ?? fallback.candidate_total),
    total: numberOrZero(source.total ?? fallback.total),
    flac: numberOrZero(source.flac ?? fallback.flac),
    alac: numberOrZero(source.alac ?? fallback.alac),
    hi_res: numberOrZero(source.hi_res ?? source.hires ?? fallback.hi_res),
    wav_extensible: numberOrZero(source.wav_extensible ?? fallback.wav_extensible),
    m4a_candidates: numberOrZero(source.m4a_candidates ?? fallback.m4a_candidates),
    unreadable_m4a: numberOrZero(source.unreadable_m4a ?? fallback.unreadable_m4a),
    non_alac_m4a: numberOrZero(source.non_alac_m4a ?? fallback.non_alac_m4a),
    sampler_included: Boolean(source.sampler_included ?? fallback.sampler_included),
    min_bit_depth: numberOrZero(source.min_bit_depth ?? fallback.min_bit_depth),
    db_path: source.db_path || fallback.db_path,
  };
}

function selectedTracks() {
  return state.tracks.filter((track) => track.status !== "converted" && state.selectedIds.has(track.id));
}

function selectableTracks() {
  return state.tracks.filter((track) => track.status !== "converted");
}

function renderSummary() {
  const summary = state.scanSummary || summarizeTracks(state.tracks);
  const convertedCount = state.tracks.filter((track) => track.status === "converted").length;
  const selectedCount = state.selectedIds.size;
  const items = [
    `FLAC ${formatNumber(summary.flac)}`,
    `ALAC ${formatNumber(summary.alac)}`,
    `Hi-Res ${formatNumber(summary.hi_res)}`,
    summary.wav_extensible ? `WAV_EXT ${formatNumber(summary.wav_extensible)}` : null,
    summary.unreadable_m4a ? `Unreadable M4A ${formatNumber(summary.unreadable_m4a)}` : null,
  ];
  if (convertedCount > 0) items.push(`Converted ${formatNumber(convertedCount)}`);
  if (selectedCount > 0) items.push(`Selected ${formatNumber(selectedCount)}`);
  if (els.resultsMeta) {
    els.resultsMeta.textContent = items.filter(Boolean).join(" · ");
  }
}

function renderChips() {
  renderSummary();
}

function escapeHtml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

function renderResults() {
  const tracks = state.tracks;
  const previewDisabled = state.loadingTask === "convert";
  const emptyMessage = state.loading && state.loadingTask === "scan"
    ? state.scanProgress.message || "Scanning rekordbox library…"
    : state.scanSummary
      ? "No results to display."
      : "Click “Scan Library” to begin.";
  if (!tracks.length) {
    els.body.innerHTML = `
      <tr class="empty-row">
        <td colspan="8">${emptyMessage}</td>
      </tr>
    `;
    els.selectAll.checked = false;
    els.selectAll.indeterminate = false;
    renderConvertButton();
    return;
  }

  els.body.innerHTML = tracks
    .map(
      (track) => {
        const statusDetail = track.status === "converted"
          ? (track.analysis_state === "none" && track.analysis_note ? track.analysis_note : "")
          : "";
        return `
        <tr data-id="${escapeHtml(track.id)}" ${previewTrack()?.id === track.id ? 'data-previewing="true"' : ""}>
          <td class="check-cell">
            <input class="row-select" type="checkbox" data-id="${escapeHtml(track.id)}" ${track.status === "converted" ? "disabled" : ""} ${state.selectedIds.has(track.id) ? "checked" : ""} />
          </td>
          <td class="preview-cell">
            <button class="preview-pill" type="button" data-preview-id="${escapeHtml(track.id)}" data-preview-state="${escapeHtml(previewButtonState(track))}" aria-label="${escapeHtml(previewButtonLabel(track))}" title="${escapeHtml(previewButtonLabel(track))}" ${previewDisabled ? "disabled" : ""}></button>
          </td>
          <td>${escapeHtml(track.title || "—")}</td>
          <td>${escapeHtml(track.artist || "—")}</td>
          <td><span class="type-badge">${escapeHtml(track.file_type)}</span></td>
          <td class="status-cell">
            <span class="status-badge" data-status="${escapeHtml(track.status || "pending")}">${escapeHtml(statusLabel(track))}</span>
            ${statusDetail ? `<div class="status-detail">${escapeHtml(statusDetail)}</div>` : ""}
          </td>
          <td>${formatAudioMeta(track)}</td>
          <td class="path-cell">
            <button class="path-link" type="button" data-path="${escapeHtml(track.full_path)}" title="Reveal in folder">
              ${escapeHtml(track.full_path)}
            </button>
          </td>
        </tr>
      `;
      },
    )
    .join("");

  const selectedCount = state.selectedIds.size;
  const selectableCount = selectableTracks().length;
  els.selectAll.checked = selectedCount > 0 && selectedCount === selectableCount;
  els.selectAll.indeterminate = selectedCount > 0 && selectedCount < selectableCount;
  renderConvertButton();
}

function setContextControlsDisabled(disabled) {
  els.dbPath.disabled = disabled;
  els.pickDb.disabled = disabled;
  els.includeSampler.disabled = disabled;
  els.preset.disabled = disabled;
  els.sourceHandling.disabled = disabled;
}

function setLoading(loading, task = null) {
  state.loading = loading;
  state.loadingTask = loading ? (task || "scan") : null;
  const scanReady = state.preflight ? state.preflight.scan_ready : true;
  els.scan.disabled = loading || !scanReady;
  setContextControlsDisabled(loading);
  renderConvertButton();
  if (loading && state.loadingTask === "scan") {
    setStatus("Scanning", "", "busy");
  } else if (loading && state.loadingTask === "convert") {
    setStatus("Converting", "", "busy");
  } else {
    clearScanProgress();
    clearConvertProgress();
  }
}

function applyPreflightState() {
  renderFooterNote();
  const m4aOption = [...els.preset.options].find((option) => option.value === "m4a-320");
  if (!m4aOption) return;
  const m4aAvailable = state.preflight?.m4a_encoder_available ?? true;
  m4aOption.disabled = !m4aAvailable;
  m4aOption.textContent = m4aAvailable ? "M4A 320kbps" : "M4A 320kbps (Unavailable)";
  if (!m4aAvailable && els.preset.value === "m4a-320") {
    els.preset.value = "mp3-320";
    saveSettings();
  }
}

async function refreshPreflight() {
  const requestId = state.preflightRequestId + 1;
  const requestedPath = els.dbPath.value;
  state.preflightRequestId = requestId;

  try {
    const response = await invoke("preflight_check", {
      req: {
        dbPath: requestedPath,
      },
    });
    if (requestId !== state.preflightRequestId || requestedPath !== els.dbPath.value) {
      return;
    }
    state.preflight = response;
    applyPreflightState();
    renderSummary();
    renderResults();
  } catch (error) {
    if (requestId !== state.preflightRequestId || requestedPath !== els.dbPath.value) {
      return;
    }
    state.preflight = null;
    els.footerNote.textContent = `Environment check failed: ${String(error)}`;
  }
}

async function pickDatabase() {
  try {
    const path = await invoke("pick_database_path");
    if (path) {
      const pathChanged = path !== els.dbPath.value;
      els.dbPath.value = path;
      if (pathChanged) {
        await invalidateScanState({ clearPreflight: true });
      }
      saveSettings();
      await refreshPreflight();
      renderChips();
      renderResults();
    }
  } catch (error) {
    setError(String(error));
    setStatus("Error", "Failed to open the database picker", "error");
  }
}

async function scan() {
  const requestId = state.scanRequestId + 1;
  const requestedPath = els.dbPath.value;
  const ignoreSamplerChecked = els.includeSampler.checked;
  state.scanRequestId = requestId;
  setError("");
  clearConversionMessage();
  setLoading(true, "scan");
  setScanProgress({
    active: true,
    phase: "querying",
    current: 0,
    total: 0,
    message: "Reading rekordbox database…",
  });
  try {
    const response = await invoke("scan_library", {
      req: {
        dbPath: requestedPath,
        minBitDepth: DEFAULT_MIN_BIT_DEPTH,
        includeSampler: !ignoreSamplerChecked,
      },
    });
    if (!currentScanRequestMatches(requestId, requestedPath, ignoreSamplerChecked)) {
      return;
    }

    state.tracks = (response.tracks || []).map((track) => ({
      ...track,
      status: "pending",
      scan_issue: track.scan_issue || null,
      scan_note: track.scan_note || "",
      analysis_state: track.analysis_state || null,
      analysis_note: track.analysis_note || "",
    }));
    const summary = response.summary ? normalizeScanSummary(response.summary, state.tracks) : null;
    state.scanSummary = summary;
    state.selectedIds = new Set();
    state.conversionCompleted = false;
    setStatus("Scanned", "", "ready");
    if (summary?.library_total) {
      const noteParts = [
        `Scanned ${formatNumber(summary.total)} results from ${formatNumber(summary.library_total)} library entries.`,
      ];
      if (summary.unreadable_m4a) {
        noteParts.push(
          `${formatNumber(summary.unreadable_m4a)} M4A candidates could not be read at their stored paths.`
        );
      }
      if (summary.non_alac_m4a) {
        noteParts.push(
          `${formatNumber(summary.non_alac_m4a)} M4A candidates were not ALAC.`
        );
      }
      if (summary.wav_extensible) {
        noteParts.push(
          `${formatNumber(summary.wav_extensible)} WAV files use WAVE_FORMAT_EXTENSIBLE headers that some CDJ/XDJ players reject.`
        );
      }
      els.footerNote.textContent = noteParts.join(" ");
    }
    renderSummary();
    renderChips();
    renderResults();
    saveSettings();
  } catch (error) {
    if (!currentScanRequestMatches(requestId, requestedPath, ignoreSamplerChecked)) {
      return;
    }
    state.tracks = [];
    state.scanSummary = null;
    state.selectedIds = new Set();
    state.conversionCompleted = false;
    renderSummary();
    renderResults();
    setStatus("Error", "", "error");
    setError(String(error));
  } finally {
    setLoading(false);
  }
}

async function convertSelected(conflictResolution = "error") {
  conflictResolution = normalizeConflictResolution(conflictResolution);
  const tracks = selectedTracks();
  const dbPath = els.dbPath.value;
  const preset = els.preset.value;
  const sourceHandling = els.sourceHandling.value;
  if (!tracks.length) {
    setError("Select at least one track to convert.");
    return;
  }

  try {
    const rekordboxRunning = await invoke("rekordbox_process_running");
    if (rekordboxRunning) {
      window.alert("rekordbox appears to be running. Please close rekordbox before converting, then try again. No files or database rows were changed.");
      setError("Close rekordbox before converting, then try again.");
      return;
    }
  } catch (error) {
    setStatus("Error", "", "error");
    setError(`Could not check whether rekordbox is running: ${String(error)}`);
    return;
  }

  setError("");
  clearConversionMessage();
  setLoading(true, "convert");
  const previousPreview = await releasePreviewAudio();
  setConvertProgress({
    active: true,
    phase: "preparing",
    current: 0,
    total: tracks.length,
    message: tracks.length > 0 ? `Preparing conversion for 0 / ${tracks.length} tracks…` : "Preparing conversion…",
  });
  try {
    const response = await invoke("convert_tracks", {
      req: {
        dbPath,
        preset,
        sourceHandling,
        archiveConflictResolution: conflictResolution,
        outputConflictResolution: conflictResolution,
        tracks,
      },
    });
    const convertedBySourceId = new Map(
      (response.converted_tracks || []).map((track) => [track.source_id || track.id, track]),
    );
    state.tracks = state.tracks.map((track) => {
      const converted = convertedBySourceId.get(track.id);
      if (!converted) return track;
      return {
        ...track,
        ...converted,
        status: "converted",
      };
    });
    if (previousPreview.track && convertedBySourceId.has(previousPreview.track.id)) {
      await loadPreviewTrack(
        convertedBySourceId.get(previousPreview.track.id),
        previousPreview.wasPlaying,
      );
    }
    state.selectedIds = new Set();
    state.conversionCompleted = true;
    const convertedCount = numberOrZero(response.converted_count);
    const cleanupArchivedDirs = numberOrZero(response.cleanup_archived_dirs);
    const sourceCleanupFailures = numberOrZero(response.source_cleanup_failures);
    setStatus(
      "Converted",
      "",
      "ready",
    );
    const backupDir = response.backup_dir ? String(response.backup_dir) : "";
    state.lastBackupDir = backupDir;
    const cleanupText = cleanupArchivedDirs > 0
      ? ` Archived ${formatNumber(cleanupArchivedDirs)} old empty analysis folders${response.cleanup_archive_dir ? ` (${response.cleanup_archive_dir})` : ""}.`
      : "";
    const warningText = Array.isArray(response.warnings) && response.warnings.length > 0
      ? ` ${response.warnings.map((warning) => String(warning)).join(" ")}`
      : "";
    const sourceCleanupText = sourceCleanupFailures > 0
      ? ` ${formatNumber(sourceCleanupFailures)} source archive(s) could not be moved to Trash.`
      : "";
    els.footerCopy.hidden = false;
    els.footerCopy.textContent = `Converted ${formatNumber(convertedCount)} file(s).${sourceCleanupText}${cleanupText}${warningText}`;
    renderProfileCard();
    renderSummary();
    renderChips();
    renderResults();
    saveSettings();
  } catch (error) {
    const conflict = conflictResolution === "error" ? existingFileConflictMessage(error) : null;
    if (conflict) {
      const nextResolution = promptConflictResolution(conflict);
      if (nextResolution) {
        setLoading(false);
        setConvertProgress({
          active: false,
          phase: "idle",
          current: 0,
          total: 0,
          message: "",
        });
        await convertSelected(nextResolution);
        return;
      }
    }
    setStatus("Error", "", "error");
    setError(String(error));
  } finally {
    setLoading(false);
  }
}

async function wireEvents() {
  await listen("scan-progress", (event) => {
    const payload = event.payload || {};
    setScanProgress({
      active: payload.phase !== "done" && payload.phase !== "error",
      phase: payload.phase || "processing",
      current: numberOrZero(payload.current),
      total: numberOrZero(payload.total),
      message: payload.message || "Scanning…",
    });
    if (payload.phase === "querying" || payload.phase === "processing") {
      setStatus("Scanning", "", "busy");
    }
  });

  await listen("convert-progress", (event) => {
    const payload = event.payload || {};
    setConvertProgress({
      active: payload.phase !== "done" && payload.phase !== "error",
      phase: payload.phase || "processing",
      current: numberOrZero(payload.current),
      total: numberOrZero(payload.total),
      message: payload.message || "Converting…",
    });
    if (payload.phase === "preparing" || payload.phase === "processing" || payload.phase === "migrating") {
      setStatus("Converting", "", "busy");
    }
  });

  els.dbPath.addEventListener("input", async () => {
    if (state.loading) return;
    await invalidateScanState({ clearPreflight: true });
    saveSettings();
    refreshPreflight().catch((error) => setError(String(error)));
  });
  els.includeSampler.addEventListener("change", async () => {
    if (state.loading) return;
    await invalidateScanState();
    saveSettings();
  });
  els.preset.addEventListener("change", saveSettings);
  els.sourceHandling.addEventListener("change", saveSettings);

  els.pickDb.addEventListener("click", pickDatabase);
  els.scan.addEventListener("click", scan);
  els.convert.addEventListener("click", () => {
    convertSelected().catch((error) => {
      setStatus("Error", "", "error");
      setError(String(error));
    });
  });
  els.selectAll.addEventListener("change", () => {
    state.selectedIds = new Set(
      els.selectAll.checked ? selectableTracks().map((track) => track.id) : [],
    );
    if (state.selectedIds.size > 0) state.conversionCompleted = false;
    renderResults();
    renderChips();
  });
  els.body.addEventListener("click", async (event) => {
    const target = event.target;
    if (!(target instanceof HTMLElement)) return;

    const previewButton = target.closest("button[data-preview-id]");
    if (previewButton) {
      if (state.loadingTask === "convert") {
        return;
      }
      const trackId = previewButton.dataset.previewId;
      const track = state.tracks.find((candidate) => candidate.id === trackId);
      if (track) {
        setError("");
        if (previewTrack()?.id === track.id) {
          togglePreviewPlay();
        } else {
          await loadPreviewTrack(track, true);
        }
      }
      return;
    }

    const button = target.closest("button[data-path]");
    if (button) {
      const path = button.dataset.path;
      if (!path) return;

      try {
        await openPathInFileManager(path);
      } catch (error) {
        setStatus("Error", "", "error");
        setError(String(error));
      }
    }
  });
  els.profileCard.addEventListener("click", async (event) => {
    const target = event.target;
    if (!(target instanceof HTMLElement)) return;

    const button = target.closest("button[data-backup-path]");
    if (!button) return;

    const path = button.dataset.backupPath;
    if (!path) return;

    try {
      await openPathInFileManager(path);
    } catch (error) {
      setStatus("Error", "", "error");
      setError(String(error));
    }
  });
  els.body.addEventListener("change", (event) => {
    const target = event.target;
    if (!(target instanceof HTMLInputElement) || target.type !== "checkbox" || !target.dataset.id) {
      return;
    }
    if (target.checked) {
      state.selectedIds.add(target.dataset.id);
    } else {
      state.selectedIds.delete(target.dataset.id);
    }
    if (state.selectedIds.size > 0) state.conversionCompleted = false;
    renderResults();
    renderChips();
  });
  els.previewAudio.addEventListener("loadedmetadata", () => {
    state.player.duration = els.previewAudio.duration || 0;
    state.player.currentTime = els.previewAudio.currentTime || 0;
    state.player.seekValue = state.player.currentTime;
    renderPlayer();
  });
  els.previewAudio.addEventListener("canplay", () => {
    if (!previewTrack() || !state.player.desiredPlaying || !els.previewAudio.paused) return;
    attemptPreviewPlayback(state.player.requestSeq).catch(() => {
      state.player.loading = false;
      renderPlayer();
      renderResults();
    });
  });
  els.previewAudio.addEventListener("timeupdate", () => {
    state.player.currentTime = els.previewAudio.currentTime || 0;
    if (!state.player.seeking) {
      state.player.seekValue = state.player.currentTime;
    }
    renderPlayer();
  });
  els.previewAudio.addEventListener("play", () => {
    state.player.loading = false;
    state.player.desiredPlaying = true;
    renderPlayer();
    renderResults();
  });
  els.previewAudio.addEventListener("pause", () => {
    const wasLoading = state.player.loading;
    state.player.loading = false;
    if (!wasLoading) {
      state.player.desiredPlaying = false;
    }
    renderPlayer();
    renderResults();
  });
  els.previewAudio.addEventListener("ended", () => {
    state.player.loading = false;
    state.player.desiredPlaying = false;
    state.player.currentTime = 0;
    state.player.seeking = false;
    state.player.seekValue = 0;
    renderPlayer();
    renderResults();
  });
  els.previewAudio.addEventListener("error", () => {
    state.player.loading = false;
    state.player.desiredPlaying = false;
    renderPlayer();
    renderResults();
    setStatus("Error", "", "error");
    setError(`This file could not be previewed: ${previewErrorMessage()}`);
  });
  els.playerPlay.addEventListener("click", togglePreviewPlay);
  els.playerSeek.addEventListener("pointerdown", () => {
    state.player.seeking = true;
    state.player.seekValue = numberOrZero(els.playerSeek.value);
    renderPlayer();
  });
  els.playerSeek.addEventListener("input", () => {
    if (!previewTrack()) return;
    state.player.seeking = true;
    state.player.seekValue = numberOrZero(els.playerSeek.value);
    renderPlayer();
  });
  els.playerSeek.addEventListener("change", () => {
    if (!previewTrack()) return;
    const nextTime = numberOrZero(els.playerSeek.value);
    els.previewAudio.currentTime = nextTime;
    state.player.currentTime = nextTime;
    state.player.seekValue = nextTime;
    state.player.seeking = false;
    renderPlayer();
  });
  els.playerSeek.addEventListener("pointerup", () => {
    if (!previewTrack()) return;
    const nextTime = numberOrZero(els.playerSeek.value);
    els.previewAudio.currentTime = nextTime;
    state.player.currentTime = nextTime;
    state.player.seekValue = nextTime;
    state.player.seeking = false;
    renderPlayer();
  });
  els.profileTrigger.addEventListener("click", (event) => {
    event.stopPropagation();
    toggleProfileCard();
  });
  els.profileBackdrop.addEventListener("click", closeProfileCard);
  els.updateDownload.addEventListener("click", async () => {
    const url = state.update.url || RELEASES_URL;
    try {
      await openExternalUrl(url);
      state.ui.updateOpen = false;
      renderUpdateDialog();
    } catch (error) {
      setStatus("Error", "", "error");
      setError(`Could not open update link: ${String(error)}`);
    }
  });
  els.updateSkip.addEventListener("click", () => {
    if (state.update.latestVersion) {
      skipUpdateVersion(state.update.latestVersion);
    }
    state.ui.updateOpen = false;
    renderUpdateDialog();
  });
  document.addEventListener("click", async (event) => {
    const target = event.target;
    if (!(target instanceof HTMLElement)) return;

    const link = target.closest("[data-external-link]");
    if (!link) return;

    event.preventDefault();
    const url = link.dataset.externalLink;
    if (!url) return;

    try {
      await openExternalUrl(url);
    } catch (error) {
      setStatus("Error", "", "error");
      setError(`Could not open link: ${String(error)}`);
    }
  });
  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      if (state.ui.updateOpen) {
        state.ui.updateOpen = false;
        renderUpdateDialog();
        return;
      }
      closeProfileCard();
    }
  });

}

function initialize() {
  document.documentElement.classList.toggle("platform-macos", IS_MACOS);
  normalizeSettings();
  renderProfileCard();
  renderScanProgress();
  renderSummary();
  renderChips();
  renderResults();
  renderPlayer();
  els.footerCopy.hidden = true;
  wireEvents().catch((error) => {
    setError(`Failed to register the scan progress listener: ${String(error)}`);
  });
  saveSettings();
  applyPreflightState();
  checkForUpdates();
  setStatus("Ready", "", "ready");
  bootstrapDefaultPath().catch((error) => {
    setError(String(error));
  });
}

async function bootstrapDefaultPath() {
  if (!els.dbPath.value) {
    const defaultPath = await invoke("default_database_path");
    if (defaultPath) {
      els.dbPath.value = defaultPath;
      saveSettings();
    }
  }
  await refreshPreflight();
  renderSummary();
  renderChips();
}

initialize();
