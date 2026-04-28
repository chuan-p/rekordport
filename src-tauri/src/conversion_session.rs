use super::*;
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::Write;

const COMPLETED_MARKER_NAME: &str = "manifest.completed";
const DATABASE_BACKUP_NAME: &str = "master.db";
pub(super) const BACKUP_DIR_PREFIX: &str = "rekordport-backup-";
const LEGACY_BACKUP_DIR_PREFIX: &str = "rkb-lossless-backup-";

#[derive(Debug)]
pub(super) struct ConvertedArtifact {
    source_path: PathBuf,
    output_path: PathBuf,
    archive_path: PathBuf,
    converted_track: Track,
}

#[derive(Debug, Default)]
pub(super) struct ConversionSession {
    artifacts: Vec<ConvertedArtifact>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ConversionManifestEntry {
    pub(super) track_id: String,
    pub(super) source_path: String,
    pub(super) archive_path: String,
    pub(super) output_path: String,
}

#[derive(Debug, Default)]
pub(super) struct ConversionRecoveryReport {
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

struct ConversionSummaryTrack {
    title: String,
    artist: String,
    source_path: String,
    output_path: String,
}

impl ConversionSession {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn push(
        &mut self,
        source_track: &Track,
        converted_track: Track,
        output_path: PathBuf,
        archive_path: PathBuf,
    ) {
        self.artifacts.push(ConvertedArtifact {
            source_path: PathBuf::from(&source_track.full_path),
            output_path,
            archive_path,
            converted_track,
        });
    }

    pub(super) fn converted_tracks(&self) -> Vec<Track> {
        self.artifacts
            .iter()
            .map(|artifact| artifact.converted_track.clone())
            .collect()
    }

    pub(super) fn archive_paths(&self) -> impl Iterator<Item = &PathBuf> {
        self.artifacts.iter().map(|artifact| &artifact.archive_path)
    }

    fn summary_tracks(&self) -> Vec<ConversionSummaryTrack> {
        self.artifacts
            .iter()
            .map(|artifact| ConversionSummaryTrack {
                title: artifact.converted_track.title.clone(),
                artist: artifact.converted_track.artist.clone(),
                source_path: artifact.source_path.to_string_lossy().to_string(),
                output_path: artifact.output_path.to_string_lossy().to_string(),
            })
            .collect()
    }

    pub(super) fn remove_outputs(&self) -> Vec<String> {
        let mut errors = Vec::new();
        for artifact in &self.artifacts {
            if let Err(error) = remove_file_path(&artifact.output_path) {
                errors.push(format!(
                    "failed to remove converted output {}: {}",
                    artifact.output_path.display(),
                    error
                ));
            }
        }
        errors
    }

    pub(super) fn restore_archives(&self) -> Vec<String> {
        let mut errors = Vec::new();
        for artifact in &self.artifacts {
            if let Err(error) = rename_path(&artifact.archive_path, &artifact.source_path) {
                errors.push(format!(
                    "failed to restore archived source {} -> {}: {}",
                    artifact.archive_path.display(),
                    artifact.source_path.display(),
                    error
                ));
            }
        }
        errors
    }

    pub(super) fn rollback_all(&self) -> Vec<String> {
        let mut errors = self.remove_outputs();
        errors.extend(self.restore_archives());
        errors
    }
}

fn receipt_created_at() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M").to_string()
}

fn display_track_name(track: &ConversionSummaryTrack) -> String {
    match (
        track.artist.trim().is_empty(),
        track.title.trim().is_empty(),
    ) {
        (false, false) => format!("{} - {}", track.artist.trim(), track.title.trim()),
        (true, false) => track.title.trim().to_string(),
        (false, true) => track.artist.trim().to_string(),
        (true, true) => track.source_path.clone(),
    }
}

pub(super) fn write_conversion_receipts(
    backup_root: &Path,
    session: &ConversionSession,
    playlist_name: Option<&str>,
) -> Result<(), String> {
    let tracks = session.summary_tracks();
    let created_at = receipt_created_at();
    let mut summary = String::new();
    summary.push_str("rekordport conversion summary\n");
    summary.push_str(&format!("Created: {created_at}\n"));
    summary.push_str(&format!("Converted: {} track(s)\n", tracks.len()));
    if let Some(name) = playlist_name {
        summary.push_str(&format!("Review playlist: {name}\n"));
    }
    summary.push_str(&format!("Backup: {}\n", backup_root.display()));
    summary.push('\n');
    summary.push_str("Tracks:\n");
    for track in &tracks {
        summary.push_str(&format!("- {}\n", display_track_name(track)));
        summary.push_str(&format!("  From: {}\n", track.source_path));
        summary.push_str(&format!("  To:   {}\n", track.output_path));
    }
    write_path(&backup_root.join("conversion-summary.txt"), summary)?;
    Ok(())
}

fn manifest_path(backup_root: &Path) -> PathBuf {
    backup_root.join("manifest.jsonl")
}

fn completed_marker_path(backup_root: &Path) -> PathBuf {
    backup_root.join(COMPLETED_MARKER_NAME)
}

fn database_backup_path(backup_root: &Path) -> PathBuf {
    backup_root.join(DATABASE_BACKUP_NAME)
}

pub(super) fn music_backup_path(backup_root: &Path) -> PathBuf {
    backup_root.join("music")
}

fn is_conversion_backup_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name.starts_with(BACKUP_DIR_PREFIX) || name.starts_with(LEGACY_BACKUP_DIR_PREFIX)
        })
}

pub(super) fn append_manifest_entry(
    backup_root: &Path,
    entry: &ConversionManifestEntry,
) -> Result<(), String> {
    let manifest = manifest_path(backup_root);
    if let Some(parent) = manifest.parent() {
        create_dir_all_path(parent)?;
    }
    let line = serde_json::to_string(entry)
        .map_err(|error| format!("failed to serialize conversion manifest entry: {}", error))?;

    retry_io_operation(
        format!("failed to append manifest {}", manifest.display()),
        || {
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&manifest)?;
            file.write_all(line.as_bytes())?;
            file.write_all(b"\n")?;
            file.flush()?;
            Ok(())
        },
    )
}

pub(super) fn remove_manifest(backup_root: &Path) -> Result<(), String> {
    let manifest = manifest_path(backup_root);
    if !path_exists(&manifest)? {
        return Ok(());
    }
    remove_file_path(&manifest)
}

pub(super) fn mark_manifest_completed(backup_root: &Path) -> Result<(), String> {
    let marker = completed_marker_path(backup_root);
    if let Some(parent) = marker.parent() {
        create_dir_all_path(parent)?;
    }
    retry_io_operation(
        format!("failed to write completion marker {}", marker.display()),
        || {
            let mut file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&marker)?;
            file.write_all(b"completed\n")?;
            file.flush()?;
            Ok(())
        },
    )
}

pub(super) fn stale_conversion_backup_manifests(
    backup_parent: &Path,
) -> Result<Vec<PathBuf>, String> {
    if !path_exists(backup_parent)? {
        return Ok(Vec::new());
    }

    let mut manifests = Vec::new();
    for entry in read_dir_path(backup_parent)? {
        let entry = entry.map_err(|error| {
            io_error_message(
                &format!(
                    "failed to read backup directory entry in {}",
                    backup_parent.display()
                ),
                &error,
            )
        })?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if !is_conversion_backup_dir(&path) {
            continue;
        }
        if path_exists(&completed_marker_path(&path))? {
            continue;
        }
        let manifest = manifest_path(&path);
        if path_exists(&manifest)? {
            manifests.push(manifest);
        }
    }

    Ok(manifests)
}

fn recover_manifest_entry(entry: &ConversionManifestEntry) -> ConversionRecoveryReport {
    let mut report = ConversionRecoveryReport::default();
    let source = PathBuf::from(&entry.source_path);
    let archive = PathBuf::from(&entry.archive_path);
    let output = PathBuf::from(&entry.output_path);
    let output_is_source = output == source;

    let archive_exists = path_exists(&archive).unwrap_or(false);
    let output_exists = path_exists(&output).unwrap_or(false);

    if output_exists && (!output_is_source || archive_exists) {
        if let Err(error) = remove_file_path(&output) {
            report.errors.push(format!(
                "failed to remove interrupted output {} for track {}: {}",
                output.display(),
                entry.track_id,
                error
            ));
            return report;
        }
    }

    let archive_exists = path_exists(&archive).unwrap_or(false);
    let source_exists = path_exists(&source).unwrap_or(false);

    if archive_exists {
        if source_exists {
            if let Err(error) = remove_file_path(&archive) {
                report.errors.push(format!(
                    "failed to remove duplicate archived source {} for track {}: {}",
                    archive.display(),
                    entry.track_id,
                    error
                ));
            } else {
                report.warnings.push(format!(
                    "interrupted conversion left both source and archive on disk for track {}; kept source {} and removed archive {}",
                    entry.track_id,
                    source.display(),
                    archive.display()
                ));
            }
        } else if let Err(error) = rename_path(&archive, &source) {
            report.errors.push(format!(
                "failed to restore archived source {} -> {} for track {}: {}",
                archive.display(),
                source.display(),
                entry.track_id,
                error
            ));
        }
    } else if !source_exists {
        report.errors.push(format!(
            "missing both source and archive while recovering interrupted conversion for track {}",
            entry.track_id
        ));
    } else if output_is_source {
        report.warnings.push(format!(
            "interrupted same-format conversion for track {} had not archived the source yet; kept original source {}",
            entry.track_id,
            source.display()
        ));
    }

    report
}

pub(super) fn recover_stale_conversion_backups(
    backup_parent: &Path,
    db_path: &Path,
) -> Result<ConversionRecoveryReport, String> {
    let mut report = ConversionRecoveryReport::default();
    for manifest in stale_conversion_backup_manifests(backup_parent)? {
        let manifest_dir = manifest.parent().unwrap_or(backup_parent);
        let db_backup = database_backup_path(manifest_dir);
        if path_exists(&db_backup).unwrap_or(false) {
            if let Err(error) = copy_path(&db_backup, db_path) {
                report.errors.push(format!(
                    "failed to restore database backup {} -> {} before recovering interrupted conversion {}: {}",
                    db_backup.display(),
                    db_path.display(),
                    manifest.display(),
                    error
                ));
                continue;
            }
        } else {
            report.errors.push(format!(
                "missing database backup {} while recovering interrupted conversion {}",
                db_backup.display(),
                manifest.display()
            ));
            continue;
        }

        let manifest_text = String::from_utf8_lossy(&read_path(&manifest)?).to_string();
        let mut had_parse_error = false;
        let mut manifest_report = ConversionRecoveryReport::default();

        for (line_index, line) in manifest_text.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let entry: ConversionManifestEntry = match serde_json::from_str(trimmed) {
                Ok(entry) => entry,
                Err(error) => {
                    manifest_report.errors.push(format!(
                        "failed to parse conversion manifest {} line {}: {}",
                        manifest.display(),
                        line_index + 1,
                        error
                    ));
                    had_parse_error = true;
                    continue;
                }
            };
            let entry_report = recover_manifest_entry(&entry);
            manifest_report.warnings.extend(entry_report.warnings);
            manifest_report.errors.extend(entry_report.errors);
        }

        if !had_parse_error && manifest_report.errors.is_empty() {
            if let Err(error) = remove_manifest(manifest_dir) {
                manifest_report.warnings.push(format!(
                    "failed to remove recovered manifest {}: {}",
                    manifest.display(),
                    error
                ));
            }
        }

        report.warnings.extend(manifest_report.warnings);
        report.errors.extend(manifest_report.errors);
    }

    Ok(report)
}

pub(super) fn cleanup_completed_conversion_backups(
    backup_parent: &Path,
) -> Result<ConversionRecoveryReport, String> {
    let mut report = ConversionRecoveryReport::default();
    if !path_exists(backup_parent)? {
        return Ok(report);
    }

    for entry in read_dir_path(backup_parent)? {
        let entry = entry.map_err(|error| {
            io_error_message(
                &format!(
                    "failed to read backup directory entry in {}",
                    backup_parent.display()
                ),
                &error,
            )
        })?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if !is_conversion_backup_dir(&path) {
            continue;
        }

        let marker = completed_marker_path(&path);
        if !path_exists(&marker)? {
            continue;
        }

        let manifest = manifest_path(&path);
        let mut manifest_removed = true;
        if path_exists(&manifest)? {
            if let Err(error) = remove_file_path(&manifest) {
                report.warnings.push(format!(
                    "failed to remove completed manifest {}: {}",
                    manifest.display(),
                    error
                ));
                manifest_removed = false;
            }
        }
        if manifest_removed {
            if let Err(error) = remove_file_path(&marker) {
                report.warnings.push(format!(
                    "failed to remove completion marker {}: {}",
                    marker.display(),
                    error
                ));
            }
        }
    }

    Ok(report)
}

pub(super) fn cleanup_successful_music_backup(backup_root: &Path) -> Result<(), String> {
    let music_backup = music_backup_path(backup_root);
    if !path_exists(&music_backup)? {
        return Ok(());
    }
    remove_dir_all_path(&music_backup)
}

pub(super) fn cleanup_successful_music_backups(
    backup_parent: &Path,
) -> Result<ConversionRecoveryReport, String> {
    let mut report = ConversionRecoveryReport::default();
    if !path_exists(backup_parent)? {
        return Ok(report);
    }

    for entry in read_dir_path(backup_parent)? {
        let entry = entry.map_err(|error| {
            io_error_message(
                &format!(
                    "failed to read backup directory entry in {}",
                    backup_parent.display()
                ),
                &error,
            )
        })?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if !is_conversion_backup_dir(&path) {
            continue;
        }
        if is_incomplete_conversion_backup(&path)? {
            continue;
        }
        if let Err(error) = cleanup_successful_music_backup(&path) {
            report.warnings.push(format!(
                "failed to remove successful music backup {}: {}",
                music_backup_path(&path).display(),
                error
            ));
        }
    }

    Ok(report)
}

fn is_incomplete_conversion_backup(backup_root: &Path) -> Result<bool, String> {
    Ok(path_exists(&manifest_path(backup_root))?
        && !path_exists(&completed_marker_path(backup_root))?)
}

pub(super) fn cleanup_successful_database_backups(
    backup_parent: &Path,
    retain_count: usize,
) -> Result<ConversionRecoveryReport, String> {
    let mut report = ConversionRecoveryReport::default();
    if !path_exists(backup_parent)? {
        return Ok(report);
    }

    let mut successful_backups = Vec::new();
    for entry in read_dir_path(backup_parent)? {
        let entry = entry.map_err(|error| {
            io_error_message(
                &format!(
                    "failed to read backup directory entry in {}",
                    backup_parent.display()
                ),
                &error,
            )
        })?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !(name.starts_with(BACKUP_DIR_PREFIX) || name.starts_with(LEGACY_BACKUP_DIR_PREFIX)) {
            continue;
        }
        if is_incomplete_conversion_backup(&path)? {
            continue;
        }
        if !path_exists(&database_backup_path(&path))? {
            continue;
        }
        successful_backups.push((name.to_string(), path));
    }

    successful_backups.sort_by(|left, right| right.0.cmp(&left.0));
    for (_, backup_root) in successful_backups.into_iter().skip(retain_count) {
        let db_backup = database_backup_path(&backup_root);
        if let Err(error) = remove_file_path(&db_backup) {
            report.warnings.push(format!(
                "failed to remove older database backup {}: {}",
                db_backup.display(),
                error
            ));
        }
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rollback_restores_archived_source_and_removes_output() {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        let source = dir.path().join("track.flac");
        let archive = dir.path().join("track-1000kbps.flac");
        let output = dir.path().join("track.wav");

        fs::write(&archive, b"original audio").expect("archive fixture should be written");
        fs::write(&output, b"converted audio").expect("output fixture should be written");

        let track = Track {
            id: "1".to_string(),
            source_id: None,
            scan_issue: None,
            scan_note: None,
            analysis_state: None,
            analysis_note: None,
            title: "Track".to_string(),
            artist: "Artist".to_string(),
            file_type: "FLAC".to_string(),
            codec_name: None,
            bit_depth: Some(24),
            sample_rate: Some(48_000),
            bitrate: Some(1000),
            full_path: source.to_string_lossy().to_string(),
        };

        let mut session = ConversionSession::new();
        session.push(&track, track.clone(), output.clone(), archive.clone());
        let errors = session.rollback_all();

        assert!(errors.is_empty());
        assert!(source.exists());
        assert!(!archive.exists());
        assert!(!output.exists());
        assert_eq!(
            fs::read(&source).expect("source should be restored"),
            b"original audio"
        );
    }

    #[test]
    fn rollback_reports_restore_errors() {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        let source = dir.path().join("track.flac");
        let output = dir.path().join("track.wav");

        fs::write(&output, b"converted audio").expect("output fixture should be written");

        let track = Track {
            id: "1".to_string(),
            source_id: None,
            scan_issue: None,
            scan_note: None,
            analysis_state: None,
            analysis_note: None,
            title: "Track".to_string(),
            artist: "Artist".to_string(),
            file_type: "FLAC".to_string(),
            codec_name: None,
            bit_depth: Some(24),
            sample_rate: Some(48_000),
            bitrate: Some(1000),
            full_path: source.to_string_lossy().to_string(),
        };

        let mut session = ConversionSession::new();
        session.push(
            &track,
            track.clone(),
            output.clone(),
            dir.path().join("missing.flac"),
        );
        let errors = session.rollback_all();

        assert!(!errors.is_empty());
        assert!(!output.exists());
        assert!(errors
            .iter()
            .any(|error| error.contains("failed to restore archived source")));
    }

    #[test]
    fn write_conversion_receipts_creates_conversion_summary() {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        let backup_root = dir.path().join("rekordport-backup-123");
        let source = dir.path().join("track.flac");
        let archive = dir.path().join("track-1000kbps.flac");
        let output = dir.path().join("track.wav");

        fs::create_dir_all(&backup_root).expect("backup root should be created");

        let track = Track {
            id: "1".to_string(),
            source_id: None,
            scan_issue: None,
            scan_note: None,
            analysis_state: None,
            analysis_note: None,
            title: "Track".to_string(),
            artist: "Artist".to_string(),
            file_type: "FLAC".to_string(),
            codec_name: None,
            bit_depth: Some(24),
            sample_rate: Some(48_000),
            bitrate: Some(1000),
            full_path: source.to_string_lossy().to_string(),
        };
        let mut converted = track.clone();
        converted.full_path = output.to_string_lossy().to_string();

        let mut session = ConversionSession::new();
        session.push(&track, converted, output, archive);
        write_conversion_receipts(
            &backup_root,
            &session,
            Some("rekordport Converted 04-28 15:42"),
        )
        .expect("summary should be written");

        let summary = fs::read_to_string(backup_root.join("conversion-summary.txt"))
            .expect("summary should be readable");
        assert!(summary.contains("Converted: 1 track(s)"));
        assert!(summary.contains("Artist - Track"));
        assert!(summary.contains("rekordport Converted 04-28 15:42"));
    }

    #[test]
    fn write_conversion_receipts_does_not_create_restore_artifacts() {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        let backup_root = dir.path().join("rekordport-backup-123");
        let source = dir.path().join("track.flac");
        let archive = dir.path().join("track-1000kbps.flac");
        let output = dir.path().join("track.wav");

        fs::create_dir_all(&backup_root).expect("backup root should be created");

        let track = Track {
            id: "1".to_string(),
            source_id: None,
            scan_issue: None,
            scan_note: None,
            analysis_state: None,
            analysis_note: None,
            title: "Track".to_string(),
            artist: "Artist".to_string(),
            file_type: "FLAC".to_string(),
            codec_name: None,
            bit_depth: Some(24),
            sample_rate: Some(48_000),
            bitrate: Some(1000),
            full_path: source.to_string_lossy().to_string(),
        };
        let mut converted = track.clone();
        converted.full_path = output.to_string_lossy().to_string();

        let mut session = ConversionSession::new();
        session.push(&track, converted, output, archive);
        write_conversion_receipts(&backup_root, &session, None).expect("summary should be written");

        let summary = fs::read_to_string(backup_root.join("conversion-summary.txt"))
            .expect("summary should be readable");
        assert!(!summary.contains("Restore:"));
        assert!(!backup_root.join("restore-plan.json").exists());
        assert!(!backup_root.join("Restore this conversion.command").exists());
        assert!(!backup_root.join("Restore this conversion.ps1").exists());
    }

    #[test]
    fn recover_stale_conversion_backups_restores_files_and_clears_manifest() {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        let backup_root = dir.path().join("rekordport-backup-123");
        let db_path = dir.path().join("master.db");
        let source = dir.path().join("track.flac");
        let archive = backup_root.join("music/track-1000kbps.flac");
        let output = dir.path().join("track.wav");

        fs::create_dir_all(archive.parent().expect("archive parent should exist"))
            .expect("backup directories should be created");
        fs::write(&db_path, b"converted database").expect("database fixture should be written");
        fs::write(database_backup_path(&backup_root), b"original database")
            .expect("database backup fixture should be written");
        fs::write(&archive, b"archived audio").expect("archive fixture should be written");
        fs::write(&output, b"converted audio").expect("output fixture should be written");

        let entry = ConversionManifestEntry {
            track_id: "1".to_string(),
            source_path: source.to_string_lossy().to_string(),
            archive_path: archive.to_string_lossy().to_string(),
            output_path: output.to_string_lossy().to_string(),
        };
        append_manifest_entry(&backup_root, &entry).expect("manifest should be written");

        let report = recover_stale_conversion_backups(dir.path(), &db_path)
            .expect("stale conversion backup should be recoverable");

        assert!(report.errors.is_empty());
        assert!(source.exists());
        assert!(!archive.exists());
        assert!(!output.exists());
        assert!(!backup_root.join("manifest.jsonl").exists());
        assert_eq!(
            fs::read(&source).expect("source should be restored"),
            b"archived audio"
        );
        assert_eq!(
            fs::read(&db_path).expect("database should be restored"),
            b"original database"
        );
    }

    #[test]
    fn recover_stale_conversion_backups_removes_duplicate_archive_when_source_is_present() {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        let backup_root = dir.path().join("rekordport-backup-123");
        let db_path = dir.path().join("master.db");
        let source = dir.path().join("track.flac");
        let archive = backup_root.join("music/track-1000kbps.flac");
        let output = dir.path().join("track.wav");

        fs::create_dir_all(archive.parent().expect("archive parent should exist"))
            .expect("backup directories should be created");
        fs::write(&db_path, b"converted database").expect("database fixture should be written");
        fs::write(database_backup_path(&backup_root), b"original database")
            .expect("database backup fixture should be written");
        fs::write(&source, b"original audio").expect("source fixture should be written");
        fs::write(&archive, b"duplicate archived audio")
            .expect("archive fixture should be written");
        fs::write(&output, b"converted audio").expect("output fixture should be written");

        let entry = ConversionManifestEntry {
            track_id: "1".to_string(),
            source_path: source.to_string_lossy().to_string(),
            archive_path: archive.to_string_lossy().to_string(),
            output_path: output.to_string_lossy().to_string(),
        };
        append_manifest_entry(&backup_root, &entry).expect("manifest should be written");

        let report = recover_stale_conversion_backups(dir.path(), &db_path)
            .expect("stale conversion backup should be recoverable");

        assert!(report.errors.is_empty());
        assert!(source.exists());
        assert!(!archive.exists());
        assert!(!output.exists());
    }

    #[test]
    fn recover_stale_same_format_before_archive_keeps_source() {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        let backup_root = dir.path().join("rekordport-backup-123");
        let db_path = dir.path().join("master.db");
        let source = dir.path().join("track.wav");
        let archive = backup_root.join("music/track-1536kbps.wav");

        fs::create_dir_all(archive.parent().expect("archive parent should exist"))
            .expect("backup directories should be created");
        fs::write(&db_path, b"converted database").expect("database fixture should be written");
        fs::write(database_backup_path(&backup_root), b"original database")
            .expect("database backup fixture should be written");
        fs::write(&source, b"original audio").expect("source fixture should be written");

        let entry = ConversionManifestEntry {
            track_id: "1".to_string(),
            source_path: source.to_string_lossy().to_string(),
            archive_path: archive.to_string_lossy().to_string(),
            output_path: source.to_string_lossy().to_string(),
        };
        append_manifest_entry(&backup_root, &entry).expect("manifest should be written");

        let report = recover_stale_conversion_backups(dir.path(), &db_path)
            .expect("stale conversion backup should be recoverable");

        assert!(report.errors.is_empty());
        assert!(source.exists());
        assert!(!archive.exists());
        assert!(!backup_root.join("manifest.jsonl").exists());
        assert_eq!(
            fs::read(&source).expect("source should be kept"),
            b"original audio"
        );
    }

    #[test]
    fn recover_stale_same_format_after_archive_restores_source() {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        let backup_root = dir.path().join("rekordport-backup-123");
        let db_path = dir.path().join("master.db");
        let source = dir.path().join("track.wav");
        let archive = backup_root.join("music/track-1536kbps.wav");

        fs::create_dir_all(archive.parent().expect("archive parent should exist"))
            .expect("backup directories should be created");
        fs::write(&db_path, b"converted database").expect("database fixture should be written");
        fs::write(database_backup_path(&backup_root), b"original database")
            .expect("database backup fixture should be written");
        fs::write(&archive, b"archived audio").expect("archive fixture should be written");

        let entry = ConversionManifestEntry {
            track_id: "1".to_string(),
            source_path: source.to_string_lossy().to_string(),
            archive_path: archive.to_string_lossy().to_string(),
            output_path: source.to_string_lossy().to_string(),
        };
        append_manifest_entry(&backup_root, &entry).expect("manifest should be written");

        let report = recover_stale_conversion_backups(dir.path(), &db_path)
            .expect("stale conversion backup should be recoverable");

        assert!(report.errors.is_empty());
        assert!(source.exists());
        assert!(!archive.exists());
        assert!(!backup_root.join("manifest.jsonl").exists());
        assert_eq!(
            fs::read(&source).expect("source should be restored"),
            b"archived audio"
        );
    }

    #[test]
    fn recover_stale_same_format_after_output_replaces_converted_with_archive() {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        let backup_root = dir.path().join("rekordport-backup-123");
        let db_path = dir.path().join("master.db");
        let source = dir.path().join("track.wav");
        let archive = backup_root.join("music/track-1536kbps.wav");

        fs::create_dir_all(archive.parent().expect("archive parent should exist"))
            .expect("backup directories should be created");
        fs::write(&db_path, b"converted database").expect("database fixture should be written");
        fs::write(database_backup_path(&backup_root), b"original database")
            .expect("database backup fixture should be written");
        fs::write(&source, b"converted audio").expect("source output should be written");
        fs::write(&archive, b"archived audio").expect("archive fixture should be written");

        let entry = ConversionManifestEntry {
            track_id: "1".to_string(),
            source_path: source.to_string_lossy().to_string(),
            archive_path: archive.to_string_lossy().to_string(),
            output_path: source.to_string_lossy().to_string(),
        };
        append_manifest_entry(&backup_root, &entry).expect("manifest should be written");

        let report = recover_stale_conversion_backups(dir.path(), &db_path)
            .expect("stale conversion backup should be recoverable");

        assert!(report.errors.is_empty());
        assert!(source.exists());
        assert!(!archive.exists());
        assert!(!backup_root.join("manifest.jsonl").exists());
        assert_eq!(
            fs::read(&source).expect("source should be restored"),
            b"archived audio"
        );
    }

    #[test]
    fn recover_stale_conversion_backups_keeps_archive_when_output_removal_fails() {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        let backup_root = dir.path().join("rekordport-backup-123");
        let db_path = dir.path().join("master.db");
        let source = dir.path().join("track.wav");
        let archive = backup_root.join("music/track-1536kbps.wav");

        fs::create_dir_all(archive.parent().expect("archive parent should exist"))
            .expect("backup directories should be created");
        fs::write(&db_path, b"converted database").expect("database fixture should be written");
        fs::write(database_backup_path(&backup_root), b"original database")
            .expect("database backup fixture should be written");
        fs::create_dir_all(&source).expect("source directory should be created");
        fs::write(&archive, b"archived audio").expect("archive fixture should be written");

        let entry = ConversionManifestEntry {
            track_id: "1".to_string(),
            source_path: source.to_string_lossy().to_string(),
            archive_path: archive.to_string_lossy().to_string(),
            output_path: source.to_string_lossy().to_string(),
        };
        append_manifest_entry(&backup_root, &entry).expect("manifest should be written");

        let report = recover_stale_conversion_backups(dir.path(), &db_path)
            .expect("stale conversion backup should be recoverable");

        assert!(!report.errors.is_empty());
        assert!(source.exists());
        assert!(archive.exists());
        assert!(backup_root.join("manifest.jsonl").exists());
    }

    #[test]
    fn recover_stale_conversion_backups_keeps_manifest_when_database_restore_fails() {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        let backup_root = dir.path().join("rekordport-backup-123");
        let db_path = dir.path().join("master.db");
        let source = dir.path().join("track.flac");
        let archive = backup_root.join("music/track-1000kbps.flac");
        let output = dir.path().join("track.wav");

        fs::create_dir_all(archive.parent().expect("archive parent should exist"))
            .expect("backup directories should be created");
        fs::create_dir_all(&db_path).expect("database restore target should be blocked");
        fs::write(database_backup_path(&backup_root), b"original database")
            .expect("database backup fixture should be written");
        fs::write(&archive, b"archived audio").expect("archive fixture should be written");
        fs::write(&output, b"converted audio").expect("output fixture should be written");

        let entry = ConversionManifestEntry {
            track_id: "1".to_string(),
            source_path: source.to_string_lossy().to_string(),
            archive_path: archive.to_string_lossy().to_string(),
            output_path: output.to_string_lossy().to_string(),
        };
        append_manifest_entry(&backup_root, &entry).expect("manifest should be written");

        let report = recover_stale_conversion_backups(dir.path(), &db_path)
            .expect("stale conversion recovery should report restore errors");

        assert!(!report.errors.is_empty());
        assert!(archive.exists());
        assert!(output.exists());
        assert!(backup_root.join("manifest.jsonl").exists());
    }

    #[test]
    fn cleanup_completed_conversion_backups_removes_completed_manifest_files() {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        let backup_root = dir.path().join("rekordport-backup-123");
        let manifest = manifest_path(&backup_root);
        let marker = completed_marker_path(&backup_root);

        fs::create_dir_all(&backup_root).expect("backup directory should be created");
        fs::write(&manifest, b"pending").expect("manifest should be written");
        fs::write(&marker, b"completed").expect("completion marker should be written");

        let report = cleanup_completed_conversion_backups(dir.path())
            .expect("completed backup cleanup should succeed");

        assert!(report.errors.is_empty());
        assert!(!manifest.exists());
        assert!(!marker.exists());
    }

    #[test]
    fn cleanup_completed_conversion_backups_keeps_marker_when_manifest_removal_fails() {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        let backup_root = dir.path().join("rekordport-backup-123");
        let manifest = manifest_path(&backup_root);
        let marker = completed_marker_path(&backup_root);

        fs::create_dir_all(&manifest).expect("manifest directory should be created");
        fs::create_dir_all(&backup_root).expect("backup directory should be created");
        fs::write(&marker, b"completed").expect("completion marker should be written");

        let report = cleanup_completed_conversion_backups(dir.path())
            .expect("completed backup cleanup should succeed");

        assert!(!report.warnings.is_empty());
        assert!(manifest.exists());
        assert!(marker.exists());
    }

    #[test]
    fn cleanup_successful_music_backup_removes_music_directory() {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        let backup_root = dir.path().join("rekordport-backup-123");
        let music_file = music_backup_path(&backup_root).join("library/track.flac");

        fs::create_dir_all(music_file.parent().expect("music parent should exist"))
            .expect("music backup directories should be created");
        fs::write(&music_file, b"source audio").expect("music backup should be written");
        fs::write(database_backup_path(&backup_root), b"database")
            .expect("database backup should be written");

        cleanup_successful_music_backup(&backup_root)
            .expect("successful music backup cleanup should succeed");

        assert!(!music_backup_path(&backup_root).exists());
        assert!(database_backup_path(&backup_root).exists());
    }

    #[test]
    fn cleanup_successful_music_backup_reports_removal_failure() {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        let backup_root = dir.path().join("rekordport-backup-123");
        let music_backup = music_backup_path(&backup_root);

        fs::create_dir_all(&backup_root).expect("backup root should be created");
        fs::write(&music_backup, b"not a directory").expect("blocked music path should be written");

        let error = cleanup_successful_music_backup(&backup_root)
            .expect_err("file at music path should block directory cleanup");

        assert!(error.contains("failed to remove directory"));
        assert!(music_backup.exists());
    }

    #[test]
    fn cleanup_successful_music_backups_skips_incomplete_backups() {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        let successful_backup = dir.path().join("rekordport-backup-100");
        let incomplete_backup = dir.path().join("rekordport-backup-200");
        let successful_music = music_backup_path(&successful_backup).join("track.flac");
        let incomplete_music = music_backup_path(&incomplete_backup).join("track.flac");

        fs::create_dir_all(
            successful_music
                .parent()
                .expect("successful music parent should exist"),
        )
        .expect("successful music backup should be created");
        fs::create_dir_all(
            incomplete_music
                .parent()
                .expect("incomplete music parent should exist"),
        )
        .expect("incomplete music backup should be created");
        fs::write(&successful_music, b"successful music")
            .expect("successful music should be written");
        fs::write(&incomplete_music, b"incomplete music")
            .expect("incomplete music should be written");
        fs::write(manifest_path(&incomplete_backup), b"pending")
            .expect("incomplete manifest should be written");

        let report = cleanup_successful_music_backups(dir.path())
            .expect("successful music backup cleanup should succeed");

        assert!(report.warnings.is_empty());
        assert!(!music_backup_path(&successful_backup).exists());
        assert!(incomplete_music.exists());
    }

    #[test]
    fn cleanup_successful_database_backups_keeps_latest_and_incomplete_backups() {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        let old_backup = dir.path().join("rekordport-backup-100");
        let latest_backup = dir.path().join("rekordport-backup-200");
        let incomplete_backup = dir.path().join("rekordport-backup-300");
        let old_music_file = music_backup_path(&old_backup).join("track.flac");

        fs::create_dir_all(
            old_music_file
                .parent()
                .expect("old music parent should exist"),
        )
        .expect("old music backup should be created");
        fs::create_dir_all(&latest_backup).expect("latest backup should be created");
        fs::create_dir_all(&incomplete_backup).expect("incomplete backup should be created");
        fs::write(database_backup_path(&old_backup), b"old database")
            .expect("old database backup should be written");
        fs::write(database_backup_path(&latest_backup), b"latest database")
            .expect("latest database backup should be written");
        fs::write(
            database_backup_path(&incomplete_backup),
            b"incomplete database",
        )
        .expect("incomplete database backup should be written");
        fs::write(manifest_path(&incomplete_backup), b"pending")
            .expect("incomplete manifest should be written");
        fs::write(&old_music_file, b"old music").expect("old music backup should be written");

        let report = cleanup_successful_database_backups(dir.path(), 1)
            .expect("database backup cleanup should succeed");

        assert!(report.warnings.is_empty());
        assert!(!database_backup_path(&old_backup).exists());
        assert!(database_backup_path(&latest_backup).exists());
        assert!(database_backup_path(&incomplete_backup).exists());
        assert!(old_music_file.exists());
    }
}
