import { DEFAULT_MIN_BIT_DEPTH, els, makeOperationId, state } from "./app-state.js";
import { invokeCommand } from "./api.js";
import { saveSettings } from "./settings.js";
import { formatNumber, numberOrZero } from "./utils.js";
import { releasePreviewAudio } from "./preview-player.js";
import { applyPreflightState, clearConversionMessage, normalizeScanSummary, renderChips, renderFooterNote, renderResults, renderSummary, setError, setLoading, setScanProgress, setStatus } from "./render-results.js";

function currentScanRequestMatches(requestId, requestedPath, ignoreSamplerChecked) {
  return requestId === state.scanRequestId
    && requestedPath === els.dbPath.value
    && ignoreSamplerChecked === els.includeSampler.checked;
}

export async function invalidateScanState({ clearPreflight = false } = {}) {
  state.scanRequestId += 1;
  state.scanOperationId = null;
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

export async function refreshPreflight() {
  const requestId = state.preflightRequestId + 1;
  const requestedPath = els.dbPath.value;
  state.preflightRequestId = requestId;

  try {
    const response = await invokeCommand("preflight_check", {
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

export async function pickDatabase() {
  try {
    const path = await invokeCommand("pick_database_path");
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

export async function scan() {
  const requestId = state.scanRequestId + 1;
  const operationId = makeOperationId("scan");
  const requestedPath = els.dbPath.value;
  const ignoreSamplerChecked = els.includeSampler.checked;
  state.scanRequestId = requestId;
  state.scanOperationId = operationId;
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
    const response = await invokeCommand("scan_library", {
      req: {
        dbPath: requestedPath,
        minBitDepth: DEFAULT_MIN_BIT_DEPTH,
        includeSampler: !ignoreSamplerChecked,
        operationId,
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
    if (state.scanOperationId === operationId) {
      state.scanOperationId = null;
      setLoading(false);
    }
  }
}
