# Changelog

## 0.3.4 - 2026-05-05

### Fixed

- Fixed the Select All checkbox in scan results after the frontend controller split.
- Scan Library now filters out Rekordbox entries whose resolved audio files no longer exist on disk.
- Improved Windows Rekordbox path handling before missing-file filtering for local `file://localhost/C:/...` paths, UNC `file://server/share/...` paths, and extra-slash UNC paths such as `file:////server/share/...`.
- Fixed Windows file-path reveal for paths with spaces by passing Explorer's `/select` argument in the native quoted form.
- After conversion, the footer now shows the conversion result instead of leaving the environment-ready message in place.

### Improved

- Split the large Rust backend, frontend controller, and CSS files into focused modules while keeping existing Tauri command names and request payloads unchanged.
- Moved Rust tests out of the main backend entry file and removed unused backend/frontend helpers.

## 0.3.3 - 2026-04-29

### Added

- Added a Rekordbox review playlist for each successful conversion, containing the tracks converted in that run.
- Added a backup link to the info card so the retained recovery backup can be opened directly after conversion.
- Added per-library conversion lock files so two rekordport processes cannot convert or recover the same library at the same time.
- Added recursive `contentCue.Cues` migration for cue IDs and timestamp fields, including nested cue JSON values.

### Fixed

- Fixed review playlists being hidden in Rekordbox by creating them as normal root playlists instead of inheriting deleted or special playlist state from an existing template.
- Fixed false "Close rekordbox before converting" warnings on macOS by matching the Rekordbox executable name exactly instead of substring-matching process paths.
- Improved FLAC/ALAC bitrate display when Rekordbox stores a zero bitrate by resolving Rekordbox file paths more robustly and deriving an average bitrate when ffmpeg reports `N/A`.
- Fixed same-format interrupted-conversion recovery so an original source file is never deleted when the manifest output path is the same as the source path.
- Fixed current-track rollback so source-restore failures are reported and recovery manifests are preserved instead of being silently removed.
- Fixed migration failure rollback so `master.db` is restored from the database backup, with restore failures included in the returned error.
- Fixed stale preview and progress-event races by tagging scan and conversion progress with operation IDs and ignoring old async results.
- Fixed backend conversion safety by enforcing the Rekordbox process check in Rust, not only in the frontend.
- Fixed numeric text ID generation to ignore nonnumeric IDs that merely start with a digit.
- Fixed the Windows release upload cleanup step to delete the actual dated portable executable asset name.

### Improved

- Shortened the conversion completion message and moved backup details out of the footer.
- Clarified the info card label.
- Renamed new conversion backup folders from `rkb-lossless-backup-*` to `rekordport-backup-*`, while keeping recovery and cleanup compatible with older backup folders.
- After a successful conversion, full music backups are cleaned automatically while the latest recovery `master.db` backup is retained.
- Clarified that smart playlists are rule-based; Rekordbox refreshes them when the library opens.
- Hardened external tool resolution: invalid environment overrides now fail loudly, development sidecar lookup no longer accepts arbitrary working-directory executables, and system `PATH` remains the final fallback.
- Hardened Windows sidecar packaging by verifying pinned SHA-256 hashes during staging and embedding, blocking unverified downloads in CI, and refreshing mismatched sidecars during Windows preparation.
- Added a timeout to startup update checks.
- Added preview cache cleanup with age and size limits.
- Added SQLite JSON function preflight checks before conversion.

### Removed

- Removed the unused backend CSV/XLSX export commands and their `zip` dependency.

## 0.3.0 - 2026-04-27

### Added

- Added an in-app update check that follows the GitHub Releases latest redirect, avoiding unauthenticated GitHub API rate limits.
- Added an update prompt with Download on GitHub and Skip this version actions.
- Added the current app version to the About/info card instead of showing it permanently in the footer.
- Added interrupted-conversion recovery using a `manifest.jsonl` backup trail. On startup and before conversion, the app now attempts to recover stale backup state from earlier interrupted runs.

### Fixed

- Fixed Rekordbox cue migration so `contentCue.Cues` is updated in a JSON-aware way instead of using blind string replacement. This avoids corrupting cue timestamps or other numeric substrings during conversion.
- Fixed FLAC/ALAC scan rows that showed `0kbps` by falling back to audio probing when Rekordbox stores a zero bitrate.
- Improved conversion rollback so restore failures are reported instead of being silently swallowed.
- Removed the extra writable-database probe before conversion migration, reducing the time window where Rekordbox could reopen the database and block the final write.
- Fixed local dev styling disappearing by constraining Vite dependency discovery and prebundling the Tauri API imports.

### Improved

- Reduced `sqlcipher` process fan-out during track migration by batching source-data reads instead of spawning multiple queries per track.
- Improved conversion speed by reusing audio probe data and using best-effort fast file duplication for source backups and analysis resources.
- Added regression coverage for cue JSON migration, rollback error reporting, stale-backup recovery, and bitrate fallback behavior.

### Removed

- Removed the Python CLI scanner from the repository and kept the Tauri app as the single supported entry point.
