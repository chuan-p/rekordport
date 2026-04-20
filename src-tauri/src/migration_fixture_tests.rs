use super::*;

fn normalize_path_separators(value: &str) -> String {
    if cfg!(target_os = "windows") {
        value.replace('/', "\\")
    } else {
        value.replace('\\', "/")
    }
}

#[test]
fn migrates_fixture_db_track_and_rebinds_standard_playlist() {
    if !command_available("sqlcipher") {
        eprintln!("skipping fixture migration test because sqlcipher is unavailable");
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir should be created");
    let db_path = dir.path().join("master.db");
    let source_path = dir.path().join("track.flac");
    let output_path = dir.path().join("track.wav");
    fs::write(&source_path, b"source").expect("source fixture should be written");
    fs::write(&output_path, b"converted").expect("output fixture should be written");

    run_sqlcipher(
        &db_path,
        DEFAULT_KEY,
        &format!(
            r#"
CREATE TABLE djmdContent (
  ID TEXT PRIMARY KEY,
  UUID TEXT,
  MasterSongID TEXT,
  Title TEXT,
  FolderPath TEXT,
  FileNameL TEXT,
  FileNameS TEXT,
  AnalysisDataPath TEXT,
  FileType INTEGER,
  BitDepth INTEGER,
  BitRate INTEGER,
  SampleRate INTEGER,
  FileSize INTEGER,
  updated_at TEXT
);
CREATE TABLE djmdCue (ContentID TEXT, ContentUUID TEXT, updated_at TEXT);
CREATE TABLE contentActiveCensor (ID TEXT, ContentID TEXT, updated_at TEXT);
CREATE TABLE djmdActiveCensor (ID TEXT, ContentID TEXT, ContentUUID TEXT, updated_at TEXT);
CREATE TABLE djmdMixerParam (ContentID TEXT, updated_at TEXT);
CREATE TABLE djmdPlaylist (ID TEXT, SmartList TEXT);
CREATE TABLE djmdSongPlaylist (ContentID TEXT, PlaylistID TEXT, updated_at TEXT);
CREATE TABLE djmdSongMyTag (ContentID TEXT, updated_at TEXT);
CREATE TABLE djmdSongTagList (ContentID TEXT, updated_at TEXT);
CREATE TABLE djmdSongHotCueBanklist (ContentID TEXT, updated_at TEXT);
CREATE TABLE djmdSongHistory (ContentID TEXT, updated_at TEXT);
CREATE TABLE djmdSongRelatedTracks (ContentID TEXT, updated_at TEXT);
CREATE TABLE djmdSongSampler (ContentID TEXT, updated_at TEXT);
CREATE TABLE djmdRecommendLike (ContentID1 TEXT, ContentID2 TEXT, updated_at TEXT);
CREATE TABLE contentCue (ID TEXT, ContentID TEXT, Cues TEXT, rb_cue_count INTEGER, updated_at TEXT);
CREATE TABLE contentFile (
  ID TEXT,
  ContentID TEXT,
  Path TEXT,
  rb_local_path TEXT,
  UUID TEXT,
  Hash TEXT,
  Size INTEGER,
  updated_at TEXT
);
INSERT INTO djmdContent
  (ID, UUID, MasterSongID, Title, FolderPath, FileNameL, FileNameS, AnalysisDataPath, FileType, BitDepth, BitRate, SampleRate, FileSize, updated_at)
VALUES
  ('1', 'old-uuid', '1', 'Fixture Track', {}, 'track.flac', 'track.flac', '', 5, 24, 1000, 48000, 6, '');
INSERT INTO djmdPlaylist (ID, SmartList) VALUES ('10', ''), ('11', 'rules');
INSERT INTO djmdSongPlaylist (ContentID, PlaylistID, updated_at) VALUES ('1', '10', ''), ('1', '11', '');
"#,
            sql_quote(&source_path.to_string_lossy()),
        ),
    )
    .expect("fixture database should be created");

    let track = Track {
        id: "1".to_string(),
        source_id: None,
        scan_issue: None,
        scan_note: None,
        analysis_state: None,
        analysis_note: None,
        title: "Fixture Track".to_string(),
        artist: "Fixture Artist".to_string(),
        file_type: "FLAC".to_string(),
        codec_name: None,
        bit_depth: Some(24),
        sample_rate: Some(48_000),
        bitrate: Some(1000),
        full_path: source_path.to_string_lossy().to_string(),
    };
    let mut output_track = track.clone();
    output_track.file_type = "WAV".to_string();
    output_track.bit_depth = Some(16);
    output_track.bitrate = Some(1536);
    output_track.full_path = output_path.to_string_lossy().to_string();

    let spec = preset_spec("wav-auto").expect("fixture preset should be valid");
    let migrated = migrate_tracks_in_db(
        &db_path,
        std::slice::from_ref(&track),
        &[output_track],
        DEFAULT_KEY,
        &spec,
    )
    .expect("fixture migration should succeed");
    let new_id = migrated
        .first()
        .and_then(|track| track.source_id.as_ref().map(|_| track.id.clone()))
        .expect("migrated track should have a new id");

    let old_content_count = sqlcipher_required_value(
        &db_path,
        DEFAULT_KEY,
        "SELECT COUNT(*) FROM djmdContent WHERE ID = '1';",
        "expected old content count",
    )
    .expect("old content query should succeed");
    let new_content_count = sqlcipher_required_value(
        &db_path,
        DEFAULT_KEY,
        &format!(
            "SELECT COUNT(*) FROM djmdContent WHERE ID = {};",
            sql_quote(&new_id)
        ),
        "expected new content count",
    )
    .expect("new content query should succeed");
    let standard_playlist_count = sqlcipher_required_value(
        &db_path,
        DEFAULT_KEY,
        &format!(
            "SELECT COUNT(*) FROM djmdSongPlaylist WHERE PlaylistID = '10' AND ContentID = {};",
            sql_quote(&new_id)
        ),
        "expected standard playlist count",
    )
    .expect("standard playlist query should succeed");
    let smart_playlist_old_count = sqlcipher_required_value(
        &db_path,
        DEFAULT_KEY,
        "SELECT COUNT(*) FROM djmdSongPlaylist WHERE PlaylistID = '11' AND ContentID = '1';",
        "expected smart playlist count",
    )
    .expect("smart playlist query should succeed");

    assert_eq!(old_content_count, "0");
    assert_eq!(new_content_count, "1");
    assert_eq!(standard_playlist_count, "1");
    assert_eq!(smart_playlist_old_count, "1");
}

#[test]
fn migrates_analysis_resources_when_content_file_uuid_differs_from_track_uuid() {
    if !command_available("sqlcipher") {
        eprintln!("skipping fixture migration test because sqlcipher is unavailable");
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir should be created");
    let db_path = dir.path().join("master.db");
    let source_path = dir.path().join("track.flac");
    let output_path = dir.path().join("track.mp3");
    fs::write(&source_path, b"source").expect("source fixture should be written");
    fs::write(&output_path, b"converted").expect("output fixture should be written");

    let analysis_source = dir
        .path()
        .join("share/PIONEER/USBANLZ/A49/581BE-9886-4241-90C9-02B687C04804/ANLZ0000.2EX");
    fs::create_dir_all(
        analysis_source
            .parent()
            .expect("analysis source parent should exist"),
    )
    .expect("analysis source directory should be created");
    fs::write(&analysis_source, b"analysis-bytes").expect("analysis fixture should be written");
    let analysis_hash = format!("{:x}", md5::compute(b"analysis-bytes"));

    run_sqlcipher(
        &db_path,
        DEFAULT_KEY,
        &format!(
            r#"
CREATE TABLE djmdContent (
  ID TEXT PRIMARY KEY,
  UUID TEXT,
  MasterSongID TEXT,
  Title TEXT,
  FolderPath TEXT,
  FileNameL TEXT,
  FileNameS TEXT,
  AnalysisDataPath TEXT,
  FileType INTEGER,
  BitDepth INTEGER,
  BitRate INTEGER,
  SampleRate INTEGER,
  FileSize INTEGER,
  updated_at TEXT
);
CREATE TABLE djmdCue (ContentID TEXT, ContentUUID TEXT, updated_at TEXT);
CREATE TABLE contentActiveCensor (ID TEXT, ContentID TEXT, updated_at TEXT);
CREATE TABLE djmdActiveCensor (ID TEXT, ContentID TEXT, ContentUUID TEXT, updated_at TEXT);
CREATE TABLE djmdMixerParam (ContentID TEXT, updated_at TEXT);
CREATE TABLE djmdPlaylist (ID TEXT, SmartList TEXT);
CREATE TABLE djmdSongPlaylist (ContentID TEXT, PlaylistID TEXT, updated_at TEXT);
CREATE TABLE djmdSongMyTag (ContentID TEXT, updated_at TEXT);
CREATE TABLE djmdSongTagList (ContentID TEXT, updated_at TEXT);
CREATE TABLE djmdSongHotCueBanklist (ContentID TEXT, updated_at TEXT);
CREATE TABLE djmdSongHistory (ContentID TEXT, updated_at TEXT);
CREATE TABLE djmdSongRelatedTracks (ContentID TEXT, updated_at TEXT);
CREATE TABLE djmdSongSampler (ContentID TEXT, updated_at TEXT);
CREATE TABLE djmdRecommendLike (ContentID1 TEXT, ContentID2 TEXT, updated_at TEXT);
CREATE TABLE contentCue (ID TEXT, ContentID TEXT, Cues TEXT, rb_cue_count INTEGER, updated_at TEXT);
CREATE TABLE contentFile (
  ID TEXT,
  ContentID TEXT,
  Path TEXT,
  rb_local_path TEXT,
  UUID TEXT,
  Hash TEXT,
  Size INTEGER,
  updated_at TEXT
);
INSERT INTO djmdContent
  (ID, UUID, MasterSongID, Title, FolderPath, FileNameL, FileNameS, AnalysisDataPath, FileType, BitDepth, BitRate, SampleRate, FileSize, updated_at)
VALUES
  ('1', 'track-uuid-ignored', '1', 'Fixture Track', {}, 'track.flac', 'track.flac', {}, 5, 24, 1000, 48000, 6, '');
INSERT INTO djmdPlaylist (ID, SmartList) VALUES ('10', '');
INSERT INTO djmdSongPlaylist (ContentID, PlaylistID, updated_at) VALUES ('1', '10', '');
INSERT INTO contentFile
  (ID, ContentID, Path, rb_local_path, UUID, Hash, Size, updated_at)
VALUES
  ('file-a49581be-9886-4241-90c9-02b687c04804', '1', {}, {}, 'a49581be-9886-4241-90c9-02b687c04804', {}, 14, '');
"#,
            sql_quote(&source_path.to_string_lossy()),
            sql_quote(&analysis_source.to_string_lossy()),
            sql_quote(&analysis_source.to_string_lossy()),
            sql_quote(&analysis_source.to_string_lossy()),
            sql_quote(&analysis_hash),
        ),
    )
    .expect("fixture database with analysis resources should be created");

    let track = Track {
        id: "1".to_string(),
        source_id: None,
        scan_issue: None,
        scan_note: None,
        analysis_state: None,
        analysis_note: None,
        title: "Fixture Track".to_string(),
        artist: "Fixture Artist".to_string(),
        file_type: "FLAC".to_string(),
        codec_name: None,
        bit_depth: Some(24),
        sample_rate: Some(48_000),
        bitrate: Some(1000),
        full_path: source_path.to_string_lossy().to_string(),
    };
    let mut output_track = track.clone();
    output_track.file_type = "MP3".to_string();
    output_track.bit_depth = Some(16);
    output_track.bitrate = Some(320);
    output_track.full_path = output_path.to_string_lossy().to_string();

    let spec = preset_spec("mp3-320").expect("fixture preset should be valid");
    let migrated = migrate_tracks_in_db(
        &db_path,
        std::slice::from_ref(&track),
        &[output_track],
        DEFAULT_KEY,
        &spec,
    )
    .expect("fixture migration with analysis should succeed");
    let migrated_track = migrated.first().expect("expected migrated track");
    let new_content_id = &migrated_track.id;
    let new_content_uuid = sqlcipher_required_value(
        &db_path,
        DEFAULT_KEY,
        &format!(
            "SELECT UUID FROM djmdContent WHERE ID = {};",
            sql_quote(new_content_id)
        ),
        "expected migrated content uuid",
    )
    .expect("migrated content uuid query should succeed");

    let expected_analysis_path = dir.path().join(format!(
        "share/PIONEER/USBANLZ/{}/{}",
        &new_content_uuid[..3],
        &new_content_uuid[3..]
    ));
    let expected_analysis_file = expected_analysis_path.join("ANLZ0000.2EX");

    assert!(expected_analysis_file.exists());
    assert_eq!(
        fs::read(&expected_analysis_file).expect("copied analysis file should be readable"),
        b"analysis-bytes"
    );

    let stored_analysis_path = sqlcipher_required_value(
        &db_path,
        DEFAULT_KEY,
        &format!(
            "SELECT AnalysisDataPath FROM djmdContent WHERE ID = {};",
            sql_quote(new_content_id)
        ),
        "expected migrated analysis path",
    )
    .expect("analysis path query should succeed");
    assert_eq!(
        normalize_path_separators(&stored_analysis_path),
        normalize_path_separators(&expected_analysis_file.to_string_lossy())
    );

    let stored_content_file = sqlcipher_required_value(
        &db_path,
        DEFAULT_KEY,
        &format!(
            "SELECT Path || '|' || COALESCE(rb_local_path, '') || '|' || COALESCE(UUID, '') FROM contentFile WHERE ContentID = {};",
            sql_quote(new_content_id)
        ),
        "expected migrated content file row",
    )
    .expect("contentFile query should succeed");
    assert_eq!(
        normalize_path_separators(&stored_content_file),
        normalize_path_separators(&format!(
            "{}|{}|{}",
            expected_analysis_file.to_string_lossy(),
            expected_analysis_file.to_string_lossy(),
            new_content_uuid
        ))
    );
}
