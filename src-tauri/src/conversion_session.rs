use super::*;

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

    pub(super) fn remove_outputs(&self) {
        for artifact in &self.artifacts {
            let _ = remove_file_path(&artifact.output_path);
        }
    }

    pub(super) fn restore_archives(&self) {
        for artifact in &self.artifacts {
            let _ = rename_path(&artifact.archive_path, &artifact.source_path);
        }
    }

    pub(super) fn rollback_all(&self) {
        self.remove_outputs();
        self.restore_archives();
    }
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
        session.rollback_all();

        assert!(source.exists());
        assert!(!archive.exists());
        assert!(!output.exists());
        assert_eq!(
            fs::read(&source).expect("source should be restored"),
            b"original audio"
        );
    }
}
