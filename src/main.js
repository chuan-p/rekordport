import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./style.css";

const DEFAULT_MIN_BIT_DEPTH = 16;
const STORAGE_KEY = "rekordbox-lossless-scan-settings";
const IS_MACOS = /\bMac OS X\b|\bMacintosh\b/.test(navigator.userAgent);
const IS_WINDOWS = /\bWindows\b/.test(navigator.userAgent);

const $ = (id) => document.getElementById(id);

const state = {
  tracks: [],
  scanSummary: null,
  selectedIds: new Set(),
  conversionCompleted: false,
  loading: false,
  loadingTask: null,
  preflight: null,
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
    seeking: false,
    seekValue: 0,
  },
  settings: loadSettings(),
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
  resultsCaption: $("results-caption"),
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

function renderConvertButton() {
  const selectedCount = state.selectedIds.size;
  const convertReady = state.preflight ? state.preflight.convert_ready : true;

  if (!state.loading && selectedCount === 0 && state.conversionCompleted) {
    els.convert.textContent = "Conversion Completed";
    els.convert.disabled = true;
    return;
  }

  els.convert.textContent = "Convert Selected";
  els.convert.disabled = selectedCount === 0 || state.loading || !convertReady;
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

function formatNumber(value) {
  return new Intl.NumberFormat("en-US").format(value);
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

function humanBytesLike(value) {
  if (value == null || value === "") return "—";
  return String(value);
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

function formatAudioMeta(track) {
  const bitDepth = humanBytesLike(track.bit_depth);
  const sampleRate = humanBytesLike(track.sample_rate);
  const bitrate = humanBytesLike(track.bitrate);
  const sampleRateLabel = sampleRate === "—"
    ? "—"
    : Number(sampleRate) >= 1000
      ? `${(Number(sampleRate) / 1000).toFixed(Number(sampleRate) % 1000 === 0 ? 0 : 1)}k`
      : sampleRate;
  return `
    <div class="audio-meta">
      <span><strong>${escapeHtml(bitDepth)}</strong>-bit</span>
      <span><strong>${escapeHtml(sampleRateLabel)}</strong>Hz</span>
      <span><strong>${escapeHtml(bitrate)}</strong> kbps</span>
    </div>
  `;
}

async function openPathInFileManager(path) {
  await invoke("open_path_in_file_manager", { path });
}

async function resolvePreviewPath(path) {
  return invoke("prepare_preview_path", { path });
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

function renderPlayer() {
  const audio = els.previewAudio;
  const track = previewTrack();
  const hasTrack = Boolean(track);
  const isPlaying = hasTrack && !audio.paused && !audio.ended;
  const displayedTime = state.player.seeking ? state.player.seekValue : state.player.currentTime;

  els.playerPlay.textContent = !hasTrack
    ? "No track selected"
    : state.player.loading
      ? "Loading"
      : isPlaying
        ? "Pause"
        : "Play";
  els.playerPlay.disabled = !hasTrack || state.player.loading;
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

  state.player.track = track;
  state.player.currentTime = 0;
  state.player.duration = 0;
  state.player.loading = true;
  state.player.seeking = false;
  state.player.seekValue = 0;
  const playablePath = await resolvePreviewPath(track.full_path);
  const src = convertFileSrc(playablePath);
  els.previewAudio.src = src;
  els.previewAudio.load();
  renderPlayer();
  renderResults();

  if (!autoplay) {
    state.player.loading = false;
    renderPlayer();
    return;
  }

  try {
    await els.previewAudio.play();
  } catch {
    // Some webviews need a metadata/canplay cycle before playback can start.
  }
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

  if (audio.paused || audio.ended) {
    state.player.loading = true;
    renderPlayer();
    audio.play().catch(() => {
      state.player.loading = false;
      renderPlayer();
    });
  } else {
    audio.pause();
  }
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
    wasPlaying: Boolean(state.player.track && !audio.paused && !audio.ended),
  };
  const hadSource = Boolean(audio.currentSrc || audio.src);

  audio.pause();
  audio.removeAttribute("src");
  state.player.track = null;
  state.player.currentTime = 0;
  state.player.duration = 0;
  state.player.loading = false;
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

function summarizeTracks(tracks) {
  const flac = tracks.filter((t) => t.file_type === "FLAC").length;
  const alac = tracks.filter((t) => t.file_type === "ALAC").length;
  const hires = tracks.filter((t) => t.file_type === "WAV" || t.file_type === "AIFF").length;
  return { total: tracks.length, flac, alac, hires };
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
    summary.candidate_total ? `Candidates ${formatNumber(summary.candidate_total)}` : null,
    `FLAC ${formatNumber(summary.flac)}`,
    `ALAC ${formatNumber(summary.alac)}`,
    `Hi-Res ${formatNumber(summary.hires)}`,
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
  els.resultsCaption.textContent = state.loading && state.loadingTask === "scan"
    ? state.scanProgress.message || "Scanning rekordbox library…"
    : state.scanSummary?.candidate_total
      ? `${formatNumber(tracks.length)} results from ${formatNumber(state.scanSummary.candidate_total)} candidate tracks`
      : `${formatNumber(tracks.length)} results`;

  if (!tracks.length) {
  els.body.innerHTML = `
    <tr class="empty-row">
        <td colspan="8">${state.tracks.length ? "No results to display." : "Click “Scan Library” to begin."}</td>
      </tr>
    `;
    els.selectAll.checked = false;
    els.selectAll.indeterminate = false;
    renderConvertButton();
    return;
  }

  els.body.innerHTML = tracks
    .map(
      (track) => `
        <tr data-id="${escapeHtml(track.id)}" ${previewTrack()?.id === track.id ? 'data-previewing="true"' : ""}>
          <td class="check-cell">
            <input class="row-select" type="checkbox" data-id="${escapeHtml(track.id)}" ${track.status === "converted" ? "disabled" : ""} ${state.selectedIds.has(track.id) ? "checked" : ""} />
          </td>
          <td class="preview-cell">
            <button class="preview-pill" type="button" data-preview-id="${escapeHtml(track.id)}" aria-label="Play preview" title="Play preview"></button>
          </td>
          <td>${escapeHtml(track.title || "—")}</td>
          <td>${escapeHtml(track.artist || "—")}</td>
          <td><span class="type-badge">${escapeHtml(track.file_type)}</span></td>
          <td class="status-cell">
            <span class="status-badge" data-status="${escapeHtml(track.status || "pending")}">${escapeHtml(statusLabel(track))}</span>
            ${track.status === "converted" && track.analysis_state === "none" && track.analysis_note ? `<div class="status-detail">${escapeHtml(track.analysis_note || analysisStateLabel(track))}</div>` : ""}
          </td>
          <td>${formatAudioMeta(track)}</td>
          <td class="path-cell">
            <button class="path-link" type="button" data-path="${escapeHtml(track.full_path)}" title="Reveal in folder">
              ${escapeHtml(track.full_path)}
            </button>
          </td>
        </tr>
      `,
    )
    .join("");

  const selectedCount = state.selectedIds.size;
  const selectableCount = selectableTracks().length;
  els.selectAll.checked = selectedCount > 0 && selectedCount === selectableCount;
  els.selectAll.indeterminate = selectedCount > 0 && selectedCount < selectableCount;
  renderConvertButton();
}

function setLoading(loading, task = null) {
  state.loading = loading;
  state.loadingTask = loading ? (task || "scan") : null;
  const scanReady = state.preflight ? state.preflight.scan_ready : true;
  els.scan.disabled = loading || !scanReady;
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
  els.footerNote.textContent = environmentSummary();
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
  try {
    const response = await invoke("preflight_check", {
      req: {
        dbPath: els.dbPath.value,
      },
    });
    state.preflight = response;
    applyPreflightState();
    renderSummary();
    renderResults();
  } catch (error) {
    state.preflight = null;
    els.footerNote.textContent = `Environment check failed: ${String(error)}`;
  }
}

async function pickDatabase() {
  try {
    const path = await invoke("pick_database_path");
    if (path) {
      els.dbPath.value = path;
      state.scanSummary = null;
      saveSettings();
      await refreshPreflight();
      renderSummary();
      renderChips();
      renderResults();
    }
  } catch (error) {
    setError(String(error));
    setStatus("Error", "Failed to open the database picker", "error");
  }
}

async function scan() {
  setError("");
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
        dbPath: els.dbPath.value,
        minBitDepth: DEFAULT_MIN_BIT_DEPTH,
        includeSampler: !els.includeSampler.checked,
      },
    });

    state.tracks = (response.tracks || []).map((track) => ({
      ...track,
      status: "pending",
      analysis_state: track.analysis_state || null,
      analysis_note: track.analysis_note || "",
    }));
    state.scanSummary = response.summary || null;
    state.selectedIds = new Set();
    state.conversionCompleted = false;
    setStatus(
      "Scanned",
      response.summary?.candidate_total
        ? `${formatNumber(response.summary.candidate_total)} candidates inspected`
        : "",
      "ready",
    );
    if (response.summary?.library_total) {
      const noteParts = [
        `Scanned ${formatNumber(response.summary.candidate_total || 0)} candidate tracks from ${formatNumber(response.summary.library_total)} library entries.`,
      ];
      if (response.summary.unreadable_m4a) {
        noteParts.push(
          `${formatNumber(response.summary.unreadable_m4a)} M4A candidates could not be read at their stored paths.`
        );
      }
      if (response.summary.non_alac_m4a) {
        noteParts.push(
          `${formatNumber(response.summary.non_alac_m4a)} M4A candidates were not ALAC.`
        );
      }
      els.footerNote.textContent = noteParts.join(" ");
    }
    renderSummary();
    renderChips();
    renderResults();
    saveSettings();
  } catch (error) {
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
        dbPath: els.dbPath.value,
        preset: els.preset.value,
        sourceHandling: els.sourceHandling.value,
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
    setStatus(
      "Converted",
      `${response.converted_count} converted · ${response.analysis_migrated_count} analysis sets migrated`,
      "ready",
    );
    const cleanupText = response.cleanup_archived_dirs > 0
      ? ` Archived ${response.cleanup_archived_dirs} old empty analysis folders${response.cleanup_archive_dir ? ` (${response.cleanup_archive_dir})` : ""}.`
      : "";
    const sourceHandlingText = response.source_cleanup_mode === "trash"
      ? response.source_cleanup_failures > 0
        ? `Source files were moved to Trash where possible; ${response.source_cleanup_failures} file(s) could not be removed and were kept in place.`
        : "Source files were moved to Trash."
      : "Source files were renamed and kept in place.";
    els.footerCopy.hidden = false;
    els.footerCopy.textContent = `Conversion complete: ${response.converted_count} new file(s) were written. ${sourceHandlingText} Old entries were removed and standard playlists were rebound.${cleanupText}`;
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
      current: Number(payload.current || 0),
      total: Number(payload.total || 0),
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
      current: Number(payload.current || 0),
      total: Number(payload.total || 0),
      message: payload.message || "Converting…",
    });
    if (payload.phase === "preparing" || payload.phase === "processing" || payload.phase === "migrating") {
      setStatus("Converting", "", "busy");
    }
  });

  els.dbPath.addEventListener("input", () => {
    state.scanSummary = null;
    saveSettings();
    renderSummary();
    renderChips();
    renderResults();
    refreshPreflight().catch((error) => setError(String(error)));
  });
  els.includeSampler.addEventListener("change", () => {
    state.scanSummary = null;
    saveSettings();
    renderSummary();
    renderChips();
    renderResults();
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
      const trackId = previewButton.dataset.previewId;
      const track = state.tracks.find((candidate) => candidate.id === trackId);
      if (track) {
        setError("");
        await loadPreviewTrack(track, true);
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
    state.player.loading = false;
    renderPlayer();
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
    renderPlayer();
  });
  els.previewAudio.addEventListener("pause", () => {
    state.player.loading = false;
    renderPlayer();
  });
  els.previewAudio.addEventListener("ended", () => {
    state.player.loading = false;
    state.player.currentTime = 0;
    state.player.seeking = false;
    state.player.seekValue = 0;
    renderPlayer();
  });
  els.previewAudio.addEventListener("error", () => {
    state.player.loading = false;
    renderPlayer();
    setStatus("Error", "", "error");
    setError(`This file could not be previewed: ${previewErrorMessage()}`);
  });
  els.playerPlay.addEventListener("click", togglePreviewPlay);
  els.playerSeek.addEventListener("pointerdown", () => {
    state.player.seeking = true;
    state.player.seekValue = Number(els.playerSeek.value);
    renderPlayer();
  });
  els.playerSeek.addEventListener("input", () => {
    if (!previewTrack()) return;
    state.player.seeking = true;
    state.player.seekValue = Number(els.playerSeek.value);
    renderPlayer();
  });
  els.playerSeek.addEventListener("change", () => {
    if (!previewTrack()) return;
    const nextTime = Number(els.playerSeek.value);
    els.previewAudio.currentTime = nextTime;
    state.player.currentTime = nextTime;
    state.player.seekValue = nextTime;
    state.player.seeking = false;
    renderPlayer();
  });
  els.playerSeek.addEventListener("pointerup", () => {
    if (!previewTrack()) return;
    const nextTime = Number(els.playerSeek.value);
    els.previewAudio.currentTime = nextTime;
    state.player.currentTime = nextTime;
    state.player.seekValue = nextTime;
    state.player.seeking = false;
    renderPlayer();
  });

}

function initialize() {
  document.documentElement.classList.toggle("platform-macos", IS_MACOS);
  normalizeSettings();
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
