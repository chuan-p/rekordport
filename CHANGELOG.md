# Changelog

## 0.3.1 - 2026-04-28

### Added

- Added a Rekordbox review playlist for each successful conversion, containing the tracks converted in that run.
- Added a backup link to the info card so the retained recovery backup can be opened directly after conversion.

### Fixed

- Fixed review playlists being hidden in Rekordbox by creating them as normal root playlists instead of inheriting deleted or special playlist state from an existing template.
- Fixed false "Close rekordbox before converting" warnings on macOS by matching the Rekordbox executable name exactly instead of substring-matching process paths.
- Improved FLAC/ALAC bitrate display when Rekordbox stores a zero bitrate by resolving Rekordbox file paths more robustly and deriving an average bitrate when ffmpeg reports `N/A`.

### Improved

- Shortened the conversion completion message and moved backup details out of the footer.
- Clarified the info card label.
- Renamed new conversion backup folders from `rkb-lossless-backup-*` to `rekordport-backup-*`, while keeping recovery and cleanup compatible with older backup folders.
- After a successful conversion, full music backups are cleaned automatically while the latest recovery `master.db` backup is retained.

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
