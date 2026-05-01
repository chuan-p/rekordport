use super::*;

fn normalize_path_separators(value: &str) -> String {
    if cfg!(target_os = "windows") {
        value.replace('/', "\\")
    } else {
        value.replace('\\', "/")
    }
}

#[test]
fn bundled_command_filenames_include_tauri_packaged_name() {
    let names = bundled_command_filenames("sqlcipher");
    let target_specific_name = sidecar_filename("sqlcipher");

    assert_eq!(
        names.first().map(String::as_str),
        Some(target_specific_name.as_str())
    );
    assert!(
        names.iter().any(|name| name == "sqlcipher"),
        "Tauri packages external binaries without the target triple in the app bundle"
    );
}

#[test]
fn normalizes_windows_path_strings() {
    assert_eq!(
        normalize_windows_path_string(r"D:/Music/Other\2 Unlimited,Remo-Conv - Twilight Zone.aiff"),
        r"D:\Music\Other\2 Unlimited,Remo-Conv - Twilight Zone.aiff"
    );
    assert_eq!(
        normalize_windows_path_string(r"\\?\D:\Music\Track.wav"),
        r"D:\Music\Track.wav"
    );
    assert_eq!(
        normalize_windows_path_string(r"\\?\UNC\server\share\Track.wav"),
        r"\\server\share\Track.wav"
    );
}

#[cfg(target_os = "windows")]
#[test]
fn normalizes_windows_rekordbox_file_urls() {
    assert_eq!(
        normalize_rekordbox_path_value("file://localhost/C:/Music/My%20Track.flac"),
        r"C:/Music/My Track.flac"
    );
    assert_eq!(
        normalize_rekordbox_path_value("file://server/share/Music/My%20Track.flac"),
        r"\\server\share\Music\My Track.flac"
    );
    assert_eq!(
        normalize_rekordbox_path_value("file:////server/share/Music/My%20Track.flac"),
        r"\\server\share\Music\My Track.flac"
    );
}

#[test]
fn chooses_windows_preview_strategy() {
    assert_eq!(
        windows_preview_strategy(Path::new("track.mp3")),
        WindowsPreviewStrategy::CopyOriginal
    );
    assert_eq!(
        windows_preview_strategy(Path::new("track.m4a")),
        WindowsPreviewStrategy::CopyOriginal
    );
    assert_eq!(
        windows_preview_strategy(Path::new("track.aac")),
        WindowsPreviewStrategy::CopyOriginal
    );
    assert_eq!(
        windows_preview_strategy(Path::new("track.aiff")),
        WindowsPreviewStrategy::TranscodeMp3
    );
    assert_eq!(
        windows_preview_strategy(Path::new("track.flac")),
        WindowsPreviewStrategy::TranscodeMp3
    );
    assert_eq!(
        windows_preview_strategy(Path::new("track.wav")),
        WindowsPreviewStrategy::TranscodeMp3
    );
    assert_eq!(
        windows_preview_strategy(Path::new("track.ogg")),
        WindowsPreviewStrategy::TranscodeMp3
    );
}

#[test]
fn parses_webview2_runtime_registry_version() {
    let output = format!(
        r#"
HKEY_CURRENT_USER\Software\Microsoft\EdgeUpdate\Clients\{WEBVIEW2_CLIENT_GUID}
    pv    REG_SZ    146.0.3856.109
"#
    );

    assert_eq!(
        parse_webview2_registry_version(&output),
        Some("146.0.3856.109".to_string())
    );
    assert_eq!(
        parse_webview2_registry_version("    pv    REG_SZ    0.0.0.0"),
        None
    );
    assert_eq!(parse_webview2_registry_version(""), None);
}

#[test]
fn parses_ffmpeg_stereo_probe() {
    let probe = parse_ffmpeg_audio_probe(
            "Input #0, flac, from 'song.flac':\n  Duration: 00:03:00.00, bitrate: 2847 kb/s\n  Stream #0:0: Audio: flac, 96000 Hz, stereo, s24, 2847 kb/s",
        );

    assert_eq!(probe.sample_rate, Some(96_000));
    assert_eq!(probe.channels, Some(2));
    assert_eq!(probe.bitrate_kbps, Some(2847));
}

#[test]
fn parses_ffmpeg_mono_and_surround_probe() {
    let mono =
        parse_ffmpeg_audio_probe("Stream #0:0: Audio: pcm_s16le, 44100 Hz, mono, s16, 705 kb/s");
    let surround =
        parse_ffmpeg_audio_probe("Stream #0:0: Audio: alac, 48000 Hz, 5.1(side), s24p, 6912 kb/s");

    assert_eq!(mono.channels, Some(1));
    assert_eq!(surround.channels, Some(6));
}

#[test]
fn parses_container_bitrate_when_stream_bitrate_is_missing() {
    let probe = parse_ffmpeg_audio_probe(
            "Input #0, wav, from 'song.wav':\n  Duration: 00:01:00.00, bitrate: 1411 kb/s\n  Stream #0:0: Audio: pcm_s16le, 44100 Hz, stereo, s16",
        );

    assert_eq!(probe.sample_rate, Some(44_100));
    assert_eq!(probe.channels, Some(2));
    assert_eq!(probe.bitrate_kbps, Some(1411));
}

#[test]
fn normalizes_rekordbox_file_url_paths() {
    assert_eq!(
        normalize_rekordbox_path_value("file://localhost/Users/me/Music/My%20Track.flac"),
        "/Users/me/Music/My Track.flac"
    );
}

#[test]
fn resolves_rekordbox_folder_path_with_file_name() {
    let dir = tempfile::tempdir().expect("tempdir should be created");
    let source = dir.path().join("My Track.flac");
    fs::write(&source, b"fixture").expect("source fixture should be written");

    assert_eq!(
        resolve_rekordbox_audio_path(&dir.path().to_string_lossy(), "My Track.flac", ""),
        source.to_string_lossy()
    );
}

#[test]
fn resolved_rekordbox_audio_path_filters_missing_files() {
    let dir = tempfile::tempdir().expect("tempdir should be created");
    let row = ScanRow {
        id: "1".to_string(),
        title: "Track".to_string(),
        artist: "Artist".to_string(),
        file_type: 5,
        bit_depth: None,
        sample_rate: None,
        bitrate: None,
        full_path: dir
            .path()
            .join("Missing.flac")
            .to_string_lossy()
            .to_string(),
        file_name_l: String::new(),
        file_name_s: String::new(),
    };

    assert_eq!(resolve_existing_rekordbox_audio_path(&row), None);
}

#[test]
fn derives_bitrate_from_duration_and_file_size_when_ffmpeg_reports_na() {
    let mut probe = parse_ffmpeg_audio_probe(
            "Input #0, flac, from 'song.flac':\n  Duration: 00:03:00.00, start: 0.000000, bitrate: N/A\n  Stream #0:0: Audio: flac, 44100 Hz, stereo, s24",
        );

    assert_eq!(probe.sample_rate, Some(44_100));
    assert_eq!(probe.channels, Some(2));
    assert_eq!(probe.bitrate_kbps, None);
    assert_eq!(probe.duration_seconds, Some(180.0));

    fill_audio_probe_bitrate_from_file_size(&mut probe, 45_000_000);

    assert_eq!(probe.bitrate_kbps, Some(2000));
}

#[test]
fn lossless_scan_bitrate_ignores_zero_database_value() {
    let row = ScanRow {
        id: "1".to_string(),
        title: "Track".to_string(),
        artist: "Artist".to_string(),
        file_type: 5,
        bit_depth: None,
        sample_rate: None,
        bitrate: Some(0),
        full_path: String::new(),
        file_name_l: String::new(),
        file_name_s: String::new(),
    };

    assert_eq!(lossless_scan_bitrate(&row), None);
}

#[test]
fn lossless_scan_bitrate_keeps_positive_database_value() {
    let row = ScanRow {
        id: "1".to_string(),
        title: "Track".to_string(),
        artist: "Artist".to_string(),
        file_type: 5,
        bit_depth: None,
        sample_rate: None,
        bitrate: Some(2847),
        full_path: String::new(),
        file_name_l: String::new(),
        file_name_s: String::new(),
    };

    assert_eq!(lossless_scan_bitrate(&row), Some(2847));
}

#[test]
fn reads_pcm_wav_format_tag() {
    let dir = tempfile::tempdir().expect("tempdir should be created");
    let path = dir.path().join("pcm.wav");
    let bytes = [
        b'R', b'I', b'F', b'F', 0x1C, 0x00, 0x00, 0x00, b'W', b'A', b'V', b'E', b'f', b'm', b't',
        b' ', 0x10, 0x00, 0x00, 0x00, 0x01, 0x00, 0x02, 0x00, 0x44, 0xAC, 0x00, 0x00, 0x10, 0xB1,
        0x02, 0x00, 0x04, 0x00, 0x10, 0x00,
    ];
    fs::write(&path, bytes).expect("fixture should be written");

    assert_eq!(
        probe_wav_format_tag(&path).expect("format tag should be readable"),
        Some(WAV_FORMAT_TAG_PCM)
    );
}

#[test]
fn reads_extensible_wav_format_tag_after_junk_chunk() {
    let dir = tempfile::tempdir().expect("tempdir should be created");
    let path = dir.path().join("ext.wav");
    let bytes = [
        b'R', b'I', b'F', b'F', 0x34, 0x00, 0x00, 0x00, b'W', b'A', b'V', b'E', b'J', b'U', b'N',
        b'K', 0x04, 0x00, 0x00, 0x00, 0xAA, 0xBB, 0xCC, 0xDD, b'f', b'm', b't', b' ', 0x28, 0x00,
        0x00, 0x00, 0xFE, 0xFF, 0x02, 0x00, 0x80, 0xBB, 0x00, 0x00, 0x00, 0x65, 0x04, 0x00, 0x06,
        0x00, 0x18, 0x00, 0x16, 0x00, 0x18, 0x00, 0x03, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71,
    ];
    fs::write(&path, bytes).expect("fixture should be written");

    assert_eq!(
        probe_wav_format_tag(&path).expect("format tag should be readable"),
        Some(WAV_FORMAT_TAG_EXTENSIBLE)
    );
}

#[test]
fn detects_attached_picture_in_ffmpeg_probe() {
    let probe = parse_ffmpeg_audio_probe(
            "Input #0, flac, from 'song.flac':\n  Metadata:\n    title           : demo\n  Stream #0:0: Audio: flac, 44100 Hz, stereo, s16\n  Stream #0:1: Video: png, rgb24(pc), 600x600, 90k tbr, 90k tbn (attached pic)",
        );

    assert!(probe.has_attached_pic);
    assert_eq!(probe.sample_rate, Some(44_100));
}

#[test]
fn rewrites_uuid_paths_case_insensitively() {
    let rewritten = rewrite_uuid_in_path(
            "D:/PIONEER/Master/share/PIONEER/USBANLZ/A49/581BE-9886-4241-90C9-02B687C04804/ANLZ0000.DAT",
            "a49581be-9886-4241-90c9-02b687c04804",
            "11111111-2222-3333-4444-555555555555",
        );

    assert_eq!(
            rewritten,
            "D:/PIONEER/Master/share/PIONEER/USBANLZ/111/11111-2222-3333-4444-555555555555/ANLZ0000.DAT"
        );
}

#[test]
fn rewrites_analysis_resource_paths_using_fallback_layout() {
    let rewritten = rewrite_analysis_resource_path(
            "D:/PIONEER/Master/share/PIONEER/USBANLZ/a49/581be-9886-4241-90c9-02b687c04804/ANLZ0000.2EX",
            "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
            None,
            "11111111-2222-3333-4444-555555555555",
        );

    assert_eq!(
            normalize_path_separators(&rewritten),
            normalize_path_separators(
                "D:/PIONEER/Master/share/PIONEER/USBANLZ/111/11111-2222-3333-4444-555555555555/ANLZ0000.2EX"
            )
        );
}

#[test]
fn parses_ffprobe_skip_samples_side_data() {
    let text = r#"{
          "packets": [
            {
              "side_data_list": [
                {
                  "side_data_type": "Skip Samples",
                  "skip_samples": 2112,
                  "discard_padding": 0
                }
              ]
            }
          ]
        }"#;

    assert_eq!(parse_ffprobe_skip_samples_json(text), Some(2112));
    assert_eq!(samples_to_nearest_ms(2112, 44_100), 48);
    assert_eq!(samples_to_nearest_ms(2112, 48_000), 44);
    assert_eq!(samples_to_nearest_ms(1105, 44_100), 25);
    assert_eq!(samples_to_nearest_ms(1105, 48_000), 23);
}

#[test]
fn compensates_anlz_grid_and_cue_times() {
    let dir = tempfile::tempdir().expect("tempdir should be created");
    let path = dir.path().join("ANLZ0000.DAT");
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"PMAI");
    bytes.extend_from_slice(&28_u32.to_be_bytes());
    bytes.extend_from_slice(&0_u32.to_be_bytes());
    bytes.extend_from_slice(&[0; 16]);

    bytes.extend_from_slice(b"PQTZ");
    bytes.extend_from_slice(&24_u32.to_be_bytes());
    bytes.extend_from_slice(&32_u32.to_be_bytes());
    bytes.extend_from_slice(&0_u32.to_be_bytes());
    bytes.extend_from_slice(&0x0008_0000_u32.to_be_bytes());
    bytes.extend_from_slice(&1_u32.to_be_bytes());
    bytes.extend_from_slice(&1_u16.to_be_bytes());
    bytes.extend_from_slice(&12_800_u16.to_be_bytes());
    bytes.extend_from_slice(&1_000_u32.to_be_bytes());

    bytes.extend_from_slice(b"PCOB");
    bytes.extend_from_slice(&24_u32.to_be_bytes());
    bytes.extend_from_slice(&80_u32.to_be_bytes());
    bytes.extend_from_slice(&1_u32.to_be_bytes());
    bytes.extend_from_slice(&0_u16.to_be_bytes());
    bytes.extend_from_slice(&1_u16.to_be_bytes());
    bytes.extend_from_slice(&(-1_i32).to_be_bytes());
    bytes.extend_from_slice(b"PCPT");
    bytes.extend_from_slice(&28_u32.to_be_bytes());
    bytes.extend_from_slice(&56_u32.to_be_bytes());
    bytes.extend_from_slice(&1_u32.to_be_bytes());
    bytes.extend_from_slice(&0_u32.to_be_bytes());
    bytes.extend_from_slice(&0x0001_0000_u32.to_be_bytes());
    bytes.extend_from_slice(&0xffff_u16.to_be_bytes());
    bytes.extend_from_slice(&0xffff_u16.to_be_bytes());
    bytes.push(1);
    bytes.push(0);
    bytes.extend_from_slice(&1_000_u16.to_be_bytes());
    bytes.extend_from_slice(&2_000_u32.to_be_bytes());
    bytes.extend_from_slice(&u32::MAX.to_be_bytes());
    bytes.extend_from_slice(&[0; 16]);

    let file_len = bytes.len() as u32;
    bytes[8..12].copy_from_slice(&file_len.to_be_bytes());
    fs::write(&path, bytes).expect("analysis fixture should be written");

    assert!(compensate_anlz_encoder_priming(&path, 48).expect("compensation should succeed"));
    let updated = fs::read(&path).expect("analysis fixture should be readable");
    assert_eq!(read_u32_be(&updated, 56), Some(1_048));
    assert_eq!(read_u32_be(&updated, 116), Some(2_048));
    assert_eq!(read_u32_be(&updated, 120), Some(u32::MAX));
}

#[test]
fn redirects_existing_target_file_names() {
    let dir = tempfile::tempdir().expect("tempdir should be created");
    let source = dir.path().join("track.flac");
    fs::write(&source, b"source").expect("source fixture should be written");
    fs::write(dir.path().join("track.mp3"), b"existing").expect("existing target should exist");

    let spec = preset_spec("mp3-320").expect("preset should be valid");
    let redirected = build_target_path(&source, &spec, ConflictResolution::Redirect)
        .expect("redirected target path should be created");

    assert_eq!(
        redirected.file_name().and_then(|name| name.to_str()),
        Some("track (2).mp3")
    );
}

#[test]
fn allows_same_format_target_path_when_it_is_the_source_file() {
    let dir = tempfile::tempdir().expect("tempdir should be created");
    let source = dir.path().join("track.wav");
    fs::write(&source, b"source").expect("source fixture should be written");

    let spec = preset_spec("wav-auto").expect("preset should be valid");
    let target = build_target_path(&source, &spec, ConflictResolution::Error).expect("target path");

    assert_eq!(target, source);
}

#[test]
fn backups_source_files_using_best_effort_duplication() {
    let dir = tempfile::tempdir().expect("tempdir should be created");
    let source = dir.path().join("music/track.flac");
    let backup_root = dir.path().join("backup");
    fs::create_dir_all(source.parent().expect("source parent should exist"))
        .expect("source parent should be created");
    fs::write(&source, b"source audio").expect("source fixture should be written");

    let backup = backup_file_tree(&source, &backup_root).expect("backup should succeed");

    assert_eq!(
        fs::read(&backup).expect("backup should be readable"),
        b"source audio"
    );
    assert!(backup.exists());
    assert!(backup.starts_with(&backup_root));

    fs::write(&backup, b"mutated backup").expect("backup should be writable");
    assert_eq!(
        fs::read(&source).expect("source should remain readable"),
        b"source audio"
    );
}

#[test]
fn duplicates_analysis_resources_without_mutating_source() {
    let dir = tempfile::tempdir().expect("tempdir should be created");
    let source = dir.path().join("analysis/ANLZ0001.DAT");
    let destination = dir.path().join("copy/ANLZ0001.DAT");
    fs::create_dir_all(source.parent().expect("source parent should exist"))
        .expect("source parent should be created");
    fs::write(&source, b"original analysis").expect("source fixture should be written");

    duplicate_file_with_parent_dirs(&source, &destination)
        .expect("analysis resource duplication should succeed");
    fs::write(&destination, b"rewritten analysis").expect("destination should be writable");

    assert_eq!(
        fs::read(&source).expect("source should remain readable"),
        b"original analysis"
    );
    assert_eq!(
        fs::read(&destination).expect("destination should remain readable"),
        b"rewritten analysis"
    );
}

#[test]
fn validates_analysis_resources_with_stale_rekordbox_hash_metadata() {
    let dir = tempfile::tempdir().expect("tempdir should be created");
    let source = dir.path().join("ANLZ0000.DAT");
    fs::write(&source, b"rewritten by rekordbox").expect("analysis fixture should be written");

    let files = vec![ContentFileRef {
        id: "analysis-id".to_string(),
        path: "/PIONEER/USBANLZ/abc/track/ANLZ0000.DAT".to_string(),
        rb_local_path: Some(source.to_string_lossy().to_string()),
        uuid: Some("track-uuid".to_string()),
        hash: Some("83863ccc7c42ba11314119165c5db124".to_string()),
        size: Some(7580),
    }];

    let validated =
        validate_analysis_resources(&files).expect("stale metadata should not block migration");

    assert_eq!(validated.len(), 1);
    assert_eq!(validated[0].source, source);
}

#[test]
fn rejects_empty_analysis_resource_hash_metadata() {
    let dir = tempfile::tempdir().expect("tempdir should be created");
    let source = dir.path().join("ANLZ0000.DAT");
    fs::write(&source, b"analysis").expect("analysis fixture should be written");

    let files = vec![ContentFileRef {
        id: "analysis-id".to_string(),
        path: "/PIONEER/USBANLZ/abc/track/ANLZ0000.DAT".to_string(),
        rb_local_path: Some(source.to_string_lossy().to_string()),
        uuid: Some("track-uuid".to_string()),
        hash: Some("d41d8cd98f00b204e9800998ecf8427e".to_string()),
        size: Some(8),
    }];

    let error = validate_analysis_resources(&files)
        .expect_err("empty hash metadata should still be rejected");

    assert!(error.contains("analysis resource hash is empty"));
}

#[test]
fn audio_probe_cache_signature_changes_when_file_is_rewritten() {
    let dir = tempfile::tempdir().expect("tempdir should be created");
    let source = dir.path().join("track.wav");
    fs::write(&source, b"old").expect("source fixture should be written");

    let first = audio_probe_cache_signature(&source).expect("signature should be readable");
    thread::sleep(Duration::from_millis(2_100));
    fs::write(&source, b"new").expect("rewritten fixture should be written");
    let second = audio_probe_cache_signature(&source).expect("signature should be readable");

    assert_eq!(first.len, second.len);
    assert_ne!(first.modified, second.modified);
}

#[test]
fn redirects_existing_archive_file_names() {
    let dir = tempfile::tempdir().expect("tempdir should be created");
    let source = dir.path().join("track.flac");
    fs::write(&source, b"source").expect("source fixture should be written");
    fs::write(dir.path().join("track-1000kbps.flac"), b"existing")
        .expect("existing archive should exist");

    let redirected = build_source_archive_path(&source, 1000, ConflictResolution::Redirect)
        .expect("redirected archive path should be created");

    assert_eq!(
        redirected.file_name().and_then(|name| name.to_str()),
        Some("track-1000kbps (2).flac")
    );
}

#[test]
fn refreshes_command_discovery_caches() {
    let command_key = "__rkb_cache_test_command__";
    let encoder_key = "__rkb_cache_test_encoder__";
    COMMAND_CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .expect("command cache lock poisoned")
        .insert(command_key.to_string(), false);
    COMMAND_PATH_CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .expect("command path cache lock poisoned")
        .insert(command_key.to_string(), None);
    ENCODER_CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .expect("encoder cache lock poisoned")
        .insert(encoder_key.to_string(), false);

    refresh_command_discovery_caches();

    assert!(!COMMAND_CACHE
        .get()
        .expect("command cache should exist")
        .lock()
        .expect("command cache lock poisoned")
        .contains_key(command_key));
    assert!(!COMMAND_PATH_CACHE
        .get()
        .expect("command path cache should exist")
        .lock()
        .expect("command path cache lock poisoned")
        .contains_key(command_key));
    assert!(!ENCODER_CACHE
        .get()
        .expect("encoder cache should exist")
        .lock()
        .expect("encoder cache lock poisoned")
        .contains_key(encoder_key));
}

#[test]
fn timestamp_tokens_are_unique_for_rapid_calls() {
    let first = timestamp_token();
    let second = timestamp_token();

    assert_ne!(first, second);
    assert!(first.contains('-'));
    assert!(second.contains('-'));
}

#[test]
fn current_conversion_rollback_reports_restore_failure() {
    let dir = tempfile::tempdir().expect("tempdir should be created");
    let source = dir.path().join("track.flac");
    let archive = dir.path().join("missing-archive.flac");
    let temp_output = dir.path().join("temp.wav");

    fs::write(&temp_output, b"partial output").expect("temp output should be written");

    let errors = rollback_current_conversion(&temp_output, &archive, &source);

    assert!(!temp_output.exists());
    assert!(errors
        .iter()
        .any(|error| error.contains("missing archived source")));
}

#[test]
fn restore_database_backup_replaces_database() {
    let dir = tempfile::tempdir().expect("tempdir should be created");
    let db_path = dir.path().join("master.db");
    let db_backup = dir.path().join("master.backup.db");

    fs::write(&db_path, b"mutated database").expect("database should be written");
    fs::write(&db_backup, b"original database").expect("backup should be written");

    let errors = restore_database_backup(&db_backup, &db_path);

    assert!(errors.is_empty());
    assert_eq!(
        fs::read(&db_path).expect("database should be readable"),
        b"original database"
    );
}

#[test]
fn rewrites_content_cues_recursively_and_offsets_times() {
    let (rewritten, cue_count) = rewrite_content_cues_json(
            r#"[{"ContentID":"1","ContentUUID":"old","CueMsec":1000,"Nested":{"InMsec":2000,"OutMsec":3000,"CueMicrosec":4000000,"InFrame":150,"OutFrame":300,"Label":"1000"}}]"#,
            "1",
            "old",
            "2",
            "new",
            23,
        )
        .expect("content cues should rewrite");
    let value: serde_json::Value =
        serde_json::from_str(&rewritten).expect("rewritten cues should be valid JSON");
    let cue = value
        .as_array()
        .and_then(|items| items.first())
        .expect("expected first cue");
    let nested = cue.get("Nested").expect("expected nested cue data");

    assert_eq!(cue_count, 1);
    assert_eq!(
        cue.get("ContentID").and_then(|value| value.as_str()),
        Some("2")
    );
    assert_eq!(
        cue.get("ContentUUID").and_then(|value| value.as_str()),
        Some("new")
    );
    assert_eq!(
        cue.get("CueMsec").and_then(|value| value.as_i64()),
        Some(1023)
    );
    assert_eq!(
        nested.get("InMsec").and_then(|value| value.as_i64()),
        Some(2023)
    );
    assert_eq!(
        nested.get("OutMsec").and_then(|value| value.as_i64()),
        Some(3023)
    );
    assert_eq!(
        nested.get("CueMicrosec").and_then(|value| value.as_i64()),
        Some(4_023_000)
    );
    assert_eq!(
        nested.get("InFrame").and_then(|value| value.as_i64()),
        Some(153)
    );
    assert_eq!(
        nested.get("OutFrame").and_then(|value| value.as_i64()),
        Some(303)
    );
    assert_eq!(
        nested.get("Label").and_then(|value| value.as_str()),
        Some("1000")
    );
}

#[test]
fn stale_database_conversion_lock_is_removed() {
    let dir = tempfile::tempdir().expect("tempdir should be created");
    let lock_path = dir.path().join(".rekordport-conversion.lock");
    fs::write(&lock_path, "pid=999999 db=/tmp/master.db\n")
        .expect("lock fixture should be written");

    assert!(remove_stale_database_conversion_lock(&lock_path)
        .expect("stale lock cleanup should succeed"));
    assert!(!lock_path.exists());
}

#[test]
fn sqlcipher_bails_before_committing_after_statement_error() {
    if !command_available("sqlcipher") {
        eprintln!("skipping sqlcipher bail test because sqlcipher is unavailable");
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir should be created");
    let db_path = dir.path().join("bail.db");
    let error = run_sqlcipher(
        &db_path,
        DEFAULT_KEY,
        r#"
BEGIN IMMEDIATE;
CREATE TABLE migration_bail_test (value TEXT);
INSERT INTO migration_bail_test VALUES ('before-error');
INSERT INTO missing_table VALUES ('boom');
INSERT INTO migration_bail_test VALUES ('after-error');
COMMIT;
"#,
    )
    .expect_err("sqlcipher should fail on the missing table");

    assert!(
        error.contains("missing_table") || error.contains("no such table"),
        "unexpected sqlcipher error: {error}"
    );

    let table_count = sqlcipher_required_value(
        &db_path,
        DEFAULT_KEY,
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'migration_bail_test';",
        "expected table count",
    )
    .expect("database should remain readable after failed script");

    assert_eq!(table_count, "0");
}

#[test]
#[ignore]
fn migrate_real_master_db_track() {
    let db_path = env::var("RKB_REAL_MASTER_DB_PATH")
        .expect("set RKB_REAL_MASTER_DB_PATH to run this ignored test");
    let scan = scan_impl(ScanRequest {
        db_path: db_path.clone(),
        min_bit_depth: 16,
        include_sampler: false,
        operation_id: None,
    })
    .expect("scan should succeed");

    let track = scan
        .tracks
        .iter()
        .find(|candidate| candidate.id == "94106400")
        .cloned()
        .or_else(|| scan.tracks.first().cloned())
        .expect("expected at least one convertible track");

    let result = convert_impl_with_progress(
        ConvertRequest {
            db_path: db_path.clone(),
            preset: "mp3-320".to_string(),
            source_handling: "rename".to_string(),
            archive_conflict_resolution: None,
            output_conflict_resolution: None,
            operation_id: None,
            tracks: vec![track.clone()],
        },
        |_| {},
    )
    .expect("conversion and migration should succeed");

    assert_eq!(result.converted_count, 1);
    let migrated = result
        .converted_tracks
        .first()
        .expect("expected one migrated track");
    assert_eq!(migrated.source_id.as_deref(), Some(track.id.as_str()));
    assert_ne!(migrated.id, track.id);
    assert_eq!(migrated.file_type, "MP3");

    let new_content_id = &migrated.id;
    let ordinary_playlist_count = sqlcipher_required_value(
      Path::new(&db_path),
      DEFAULT_KEY,
      &format!(
        "SELECT COUNT(*) FROM djmdSongPlaylist WHERE ContentID = {} AND PlaylistID IN (SELECT ID FROM djmdPlaylist WHERE COALESCE(SmartList, '') = '');",
        sql_quote(new_content_id)
      ),
      "expected playlist binding count",
    )
    .expect("playlist count query should succeed")
    .parse::<usize>()
    .expect("playlist count should parse");

    let old_playlist_count = sqlcipher_required_value(
      Path::new(&db_path),
      DEFAULT_KEY,
      &format!(
        "SELECT COUNT(*) FROM djmdSongPlaylist WHERE ContentID = {} AND PlaylistID IN (SELECT ID FROM djmdPlaylist WHERE COALESCE(SmartList, '') = '');",
        sql_quote(&track.id)
      ),
      "expected old playlist binding count",
    )
    .expect("old playlist count query should succeed")
    .parse::<usize>()
    .expect("old playlist count should parse");

    assert!(ordinary_playlist_count > 0);
    assert_eq!(old_playlist_count, 0);

    let old_content_exists = sqlcipher_required_value(
        Path::new(&db_path),
        DEFAULT_KEY,
        &format!(
            "SELECT COUNT(*) FROM djmdContent WHERE ID = {};",
            sql_quote(&track.id)
        ),
        "expected old content count",
    )
    .expect("old content count query should succeed")
    .parse::<usize>()
    .expect("old content count should parse");
    assert_eq!(old_content_exists, 0);

    let content_file_count = sqlcipher_required_value(
        Path::new(&db_path),
        DEFAULT_KEY,
        &format!(
            "SELECT COUNT(*) FROM contentFile WHERE ContentID = {};",
            sql_quote(new_content_id)
        ),
        "expected contentFile count",
    )
    .expect("contentFile count query should succeed")
    .parse::<usize>()
    .expect("contentFile count should parse");
    assert!(content_file_count > 0);
}
