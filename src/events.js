import { IS_MACOS, els, state } from "./app-state.js";
import { invokeCommand, listenEvent, openExternalUrl, openPathInFileManager } from "./api.js";
import { checkForUpdates, renderUpdateDialog, skipUpdateVersion } from "./updates.js";
import { loadSettings, normalizeSettings, saveSettings } from "./settings.js";
import { numberOrZero } from "./utils.js";
import { convertSelected } from "./convert-flow.js";
import { invalidateScanState, pickDatabase, refreshPreflight, scan } from "./scan-flow.js";
import { attemptPreviewPlayback, loadPreviewTrack, previewErrorMessage, previewTrack, renderPlayer, togglePreviewPlay } from "./preview-player.js";
import { applyPreflightState, closeProfileCard, renderChips, renderProfileCard, renderResults, renderScanProgress, renderSummary, setConvertProgress, setError, setScanProgress, setStatus, toggleProfileCard } from "./render-results.js";

async function wireEvents() {
  await listenEvent("scan-progress", (event) => {
    const payload = event.payload || {};
    if (!payload.operationId || payload.operationId !== state.scanOperationId) {
      return;
    }
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

  await listenEvent("convert-progress", (event) => {
    const payload = event.payload || {};
    if (!payload.operationId || payload.operationId !== state.convertOperationId) {
      return;
    }
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

export function initialize() {
  state.settings = loadSettings();
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
    const defaultPath = await invokeCommand("default_database_path");
    if (defaultPath) {
      els.dbPath.value = defaultPath;
      saveSettings();
    }
  }
  await refreshPreflight();
  renderSummary();
  renderChips();
}
