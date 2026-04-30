export function numberOrZero(value) {
  const numeric = Number(value);
  return Number.isFinite(numeric) ? numeric : 0;
}

export function formatNumber(value) {
  return new Intl.NumberFormat("en-US").format(numberOrZero(value));
}

export function normalizeVersion(value) {
  return String(value || "")
    .trim()
    .replace(/^v/i, "")
    .split(/[+-]/)[0];
}

export function compareVersions(left, right) {
  const leftParts = normalizeVersion(left).split(".").map((part) => Number.parseInt(part, 10) || 0);
  const rightParts = normalizeVersion(right).split(".").map((part) => Number.parseInt(part, 10) || 0);
  const maxLength = Math.max(leftParts.length, rightParts.length);
  for (let index = 0; index < maxLength; index += 1) {
    const diff = (leftParts[index] || 0) - (rightParts[index] || 0);
    if (diff !== 0) return diff;
  }
  return 0;
}

export function humanBytesLike(value) {
  if (value == null || value === "") return "—";
  const numeric = Number(value);
  if (!Number.isFinite(numeric) || numeric <= 0) return "—";
  return String(numeric);
}

export function formatTime(value) {
  if (!Number.isFinite(value) || value < 0) return "0:00";
  const totalSeconds = Math.floor(value);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = String(totalSeconds % 60).padStart(2, "0");
  return `${minutes}:${seconds}`;
}

export function escapeHtml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}
