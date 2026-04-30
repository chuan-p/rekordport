import { els, makeOperationId, state } from "./app-state.js";
import { invokeCommand } from "./api.js";
import { saveSettings } from "./settings.js";
import { formatNumber, numberOrZero } from "./utils.js";
import { loadPreviewTrack, releasePreviewAudio } from "./preview-player.js";
import { clearConversionMessage, renderChips, renderProfileCard, renderResults, renderSummary, selectedTracks, setConvertProgress, setError, setLoading, setStatus } from "./render-results.js";

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

export async function convertSelected(conflictResolution = "error") {
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
    const rekordboxRunning = await invokeCommand("rekordbox_process_running");
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
  const operationId = makeOperationId("convert");
  state.convertOperationId = operationId;
  const previousPreview = await releasePreviewAudio();
  setConvertProgress({
    active: true,
    phase: "preparing",
    current: 0,
    total: tracks.length,
    message: tracks.length > 0 ? `Preparing conversion for 0 / ${tracks.length} tracks…` : "Preparing conversion…",
  });
  try {
    const response = await invokeCommand("convert_tracks", {
      req: {
        dbPath,
        preset,
        sourceHandling,
        archiveConflictResolution: conflictResolution,
        outputConflictResolution: conflictResolution,
        operationId,
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
    els.footerCopy.hidden = true;
    els.footerCopy.textContent = "";
    els.footerNote.textContent = `Converted ${formatNumber(convertedCount)} file(s).${sourceCleanupText}${cleanupText}${warningText}`;
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
        if (state.convertOperationId === operationId) {
          state.convertOperationId = null;
        }
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
    if (state.convertOperationId === operationId) {
      state.convertOperationId = null;
      setLoading(false);
    }
  }
}
