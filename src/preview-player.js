import { IS_WINDOWS, els, state } from "./app-state.js";
import { assetSrc, normalizePreviewAssetPath, resolvePreviewPath } from "./api.js";
import { formatTime } from "./utils.js";
import { renderResults, setError, setStatus } from "./render-results.js";

export function previewButtonState(track) {
  if (previewTrack()?.id !== track.id) return "play";
  return isPreviewPlaybackActive() ? "pause" : "play";
}

export function previewButtonLabel(track) {
  return previewButtonState(track) === "pause" ? "Pause preview" : "Play preview";
}

export function previewTrack() {
  return state.player.track;
}

export function isPreviewPlaybackActive() {
  const audio = els.previewAudio;
  if (!previewTrack()) return false;
  return state.player.desiredPlaying || (!audio.paused && !audio.ended);
}

export function nextPreviewRequest() {
  state.player.requestSeq += 1;
  return state.player.requestSeq;
}

export function previewRequestMatches(requestSeq, track) {
  return requestSeq === state.player.requestSeq
    && state.player.track?.id === track?.id;
}

export async function attemptPreviewPlayback(requestSeq, { reportError = false } = {}) {
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

export function renderPlayer() {
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

export async function loadPreviewTrack(track, autoplay = true) {
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
    if (!previewRequestMatches(requestSeq, track)) {
      return;
    }
  } catch (error) {
    if (!previewRequestMatches(requestSeq, track)) {
      return;
    }
    Object.assign(state.player, previousPlayerState);
    renderPlayer();
    renderResults();
    setStatus("Error", "", "error");
    setError(`This file could not be previewed: ${String(error)}`);
    return;
  }

  if (!previewRequestMatches(requestSeq, track)) {
    return;
  }
  const src = assetSrc(normalizePreviewAssetPath(playablePath));
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

export function previewErrorMessage() {
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

export function togglePreviewPlay() {
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

export async function releasePreviewAudio() {
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
