# Changelog

## Unreleased

### Fixed

- Fixed Rekordbox cue migration so `contentCue.Cues` is updated in a JSON-aware way instead of using blind string replacement. This avoids corrupting cue timestamps or other numeric substrings during conversion.
- Improved conversion rollback so restore failures are reported instead of being silently swallowed.
- Removed the extra writable-database probe before conversion migration, reducing the time window where Rekordbox could reopen the database and block the final write.
- Added interrupted-conversion recovery using a `manifest.jsonl` backup trail. On startup and before conversion, the app now attempts to recover stale backup state from earlier interrupted runs.

### Improved

- Reduced `sqlcipher` process fan-out during track migration by batching source-data reads instead of spawning multiple queries per track.
- Added regression coverage for cue JSON migration, rollback error reporting, and stale-backup recovery.

### Removed

- Removed the Python CLI scanner from the repository and kept the Tauri app as the single supported entry point.

