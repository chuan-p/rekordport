import { APP_VERSION, RELEASES_URL, SKIPPED_UPDATE_KEY, els, state } from "./app-state.js";
import { invokeCommand } from "./api.js";
import { compareVersions, normalizeVersion } from "./utils.js";

function skippedUpdateVersion() {
  try {
    return localStorage.getItem(SKIPPED_UPDATE_KEY) || "";
  } catch {
    return "";
  }
}

export function skipUpdateVersion(version) {
  try {
    localStorage.setItem(SKIPPED_UPDATE_KEY, version);
  } catch {
    // Ignore storage failures; the user can still dismiss this run.
  }
}

export function shouldPromptForUpdate() {
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

export function renderUpdateDialog() {
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

export async function checkForUpdates() {
  state.update = { status: "checking", latestVersion: null, url: RELEASES_URL, changelog: "" };
  renderUpdateDialog();

  try {
    const release = await invokeCommand("latest_release");
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
