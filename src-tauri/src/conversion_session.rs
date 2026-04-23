use super::*;
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::Write;

const COMPLETED_MARKER_NAME: &str = "manifest.completed";

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

fn manifest_path(backup_root: &Path) -> PathBuf {
    backup_root.join("manifest.jsonl")
}

fn completed_marker_path(backup_root: &Path) -> PathBuf {
    backup_root.join(COMPLETED_MARKER_NAME)
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
        if !path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("rkb-lossless-backup-"))
        {
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

    if path_exists(&output).unwrap_or(false) {
        if let Err(error) = remove_file_path(&output) {
            report.errors.push(format!(
                "failed to remove interrupted output {} for track {}: {}",
                output.display(),
                entry.track_id,
                error
            ));
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
    }

    report
}

pub(super) fn recover_stale_conversion_backups(
    backup_parent: &Path,
) -> Result<ConversionRecoveryReport, String> {
    let mut report = ConversionRecoveryReport::default();
    for manifest in stale_conversion_backup_manifests(backup_parent)? {
        let manifest_dir = manifest.parent().unwrap_or(backup_parent);
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
        if !path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("rkb-lossless-backup-"))
        {
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
    fn recover_stale_conversion_backups_restores_files_and_clears_manifest() {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        let backup_root = dir.path().join("rkb-lossless-backup-123");
        let source = dir.path().join("track.flac");
        let archive = backup_root.join("music/track-1000kbps.flac");
        let output = dir.path().join("track.wav");

        fs::create_dir_all(archive.parent().expect("archive parent should exist"))
            .expect("backup directories should be created");
        fs::write(&archive, b"archived audio").expect("archive fixture should be written");
        fs::write(&output, b"converted audio").expect("output fixture should be written");

        let entry = ConversionManifestEntry {
            track_id: "1".to_string(),
            source_path: source.to_string_lossy().to_string(),
            archive_path: archive.to_string_lossy().to_string(),
            output_path: output.to_string_lossy().to_string(),
        };
        append_manifest_entry(&backup_root, &entry).expect("manifest should be written");

        let report = recover_stale_conversion_backups(dir.path())
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
    }

    #[test]
    fn recover_stale_conversion_backups_removes_duplicate_archive_when_source_is_present() {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        let backup_root = dir.path().join("rkb-lossless-backup-123");
        let source = dir.path().join("track.flac");
        let archive = backup_root.join("music/track-1000kbps.flac");
        let output = dir.path().join("track.wav");

        fs::create_dir_all(archive.parent().expect("archive parent should exist"))
            .expect("backup directories should be created");
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

        let report = recover_stale_conversion_backups(dir.path())
            .expect("stale conversion backup should be recoverable");

        assert!(report.errors.is_empty());
        assert!(source.exists());
        assert!(!archive.exists());
        assert!(!output.exists());
    }

    #[test]
    fn cleanup_completed_conversion_backups_removes_completed_manifest_files() {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        let backup_root = dir.path().join("rkb-lossless-backup-123");
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
        let backup_root = dir.path().join("rkb-lossless-backup-123");
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
}
