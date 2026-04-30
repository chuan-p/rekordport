import { APP_VERSION, DEFAULT_MIN_BIT_DEPTH, PROFILE, appIcon, els, state } from "./app-state.js";
import { saveSettings } from "./settings.js";
import { humanBytesLike, escapeHtml, formatNumber, normalizeVersion, numberOrZero } from "./utils.js";
import { previewButtonLabel, previewButtonState, previewTrack } from "./preview-player.js";

export function setStatus(text, meta = "", kind = "ready") {
  els.statusPill.textContent = text;
  els.statusPill.dataset.kind = kind;
  els.statusMeta.textContent = meta;
  els.statusMeta.hidden = !meta;
}

export function setScanProgress(progress = {}) {
  state.scanProgress = {
    ...state.scanProgress,
    ...progress,
  };
  renderScanProgress();
}

export function clearScanProgress() {
  state.scanProgress = {
    active: false,
    phase: "idle",
    current: 0,
    total: 0,
    message: "",
  };
  renderScanProgress();
}

export function setConvertProgress(progress = {}) {
  state.convertProgress = {
    ...state.convertProgress,
    ...progress,
  };
  renderConvertProgress();
}

export function clearConvertProgress() {
  state.convertProgress = {
    active: false,
    phase: "idle",
    current: 0,
    total: 0,
    message: "",
  };
  renderConvertProgress();
}

export function clearConversionMessage() {
  els.footerCopy.hidden = true;
  els.footerCopy.textContent = "";
}

export function renderConvertButton() {
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

export function renderProfileCard() {
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

export function closeProfileCard() {
  if (!state.ui.profileOpen) return;
  state.ui.profileOpen = false;
  renderProfileCard();
}

export function toggleProfileCard() {
  state.ui.profileOpen = !state.ui.profileOpen;
  renderProfileCard();
}

export function renderScanProgress() {
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

export function renderConvertProgress() {
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

export function setError(message) {
  if (!message) {
    els.footerError.hidden = true;
    els.footerError.textContent = "";
    return;
  }
  els.footerError.hidden = false;
  els.footerError.textContent = message;
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

export function renderFooterNote() {
  els.footerNote.textContent = environmentSummary();
}

function statusLabel(track) {
  return track.status === "converted" ? "Converted" : "Pending";
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

export function summarizeTracks(tracks) {
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

export function normalizeScanSummary(summary, tracks = []) {
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

export function selectedTracks() {
  return state.tracks.filter((track) => track.status !== "converted" && state.selectedIds.has(track.id));
}

export function selectableTracks() {
  return state.tracks.filter((track) => track.status !== "converted");
}

export function renderSummary() {
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

export function renderChips() {
  renderSummary();
}

export function renderResults() {
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

export function setContextControlsDisabled(disabled) {
  els.dbPath.disabled = disabled;
  els.pickDb.disabled = disabled;
  els.includeSampler.disabled = disabled;
  els.preset.disabled = disabled;
  els.sourceHandling.disabled = disabled;
}

export function setLoading(loading, task = null) {
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

export function applyPreflightState() {
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
