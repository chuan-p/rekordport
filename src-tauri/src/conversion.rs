fn backup_file_tree(source: &Path, backup_root: &Path) -> Result<PathBuf, String> {
    let relative = backup_relative_path(source);
    let target = backup_root.join(relative);
    duplicate_path_best_effort(source, &target)?;
    Ok(target)
}

fn existing_paths_refer_to_same_file(left: &Path, right: &Path) -> Result<bool, String> {
    if !path_exists(left)? || !path_exists(right)? {
        return Ok(false);
    }

    Ok(canonicalize_path(left)? == canonicalize_path(right)?)
}

fn build_target_path(
    source: &Path,
    spec: &ConversionSpec,
    resolution: ConflictResolution,
) -> Result<PathBuf, String> {
    let parent = source
        .parent()
        .ok_or_else(|| format!("missing parent directory for {}", source.display()))?;
    let stem = source
        .file_stem()
        .ok_or_else(|| format!("missing file stem for {}", source.display()))?
        .to_string_lossy();
    let candidate = parent.join(format!("{stem}.{}", spec.extension));
    if existing_paths_refer_to_same_file(source, &candidate)? {
        return Ok(candidate);
    }
    if !path_exists(&candidate)? {
        return Ok(candidate);
    }

    match resolution {
        ConflictResolution::Error => Err(format!(
            "target file already exists: {}",
            candidate.display()
        )),
        ConflictResolution::Overwrite => Ok(candidate),
        ConflictResolution::Redirect => unique_redirect_path(&candidate),
    }
}

fn build_source_archive_path(
    source: &Path,
    bitrate_kbps: u32,
    resolution: ConflictResolution,
) -> Result<PathBuf, String> {
    let parent = source
        .parent()
        .ok_or_else(|| format!("missing parent directory for {}", source.display()))?;
    let stem = source
        .file_stem()
        .ok_or_else(|| format!("missing file stem for {}", source.display()))?
        .to_string_lossy();
    let extension = source
        .extension()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_default();
    let candidate = if extension.is_empty() {
        parent.join(format!("{stem}-{bitrate_kbps}kbps"))
    } else {
        parent.join(format!("{stem}-{bitrate_kbps}kbps.{extension}"))
    };
    if !path_exists(&candidate)? {
        return Ok(candidate);
    }

    match resolution {
        ConflictResolution::Error => Err(format!(
            "source archive already exists, refusing to overwrite: {}",
            candidate.display()
        )),
        ConflictResolution::Overwrite => Ok(candidate),
        ConflictResolution::Redirect => unique_redirect_path(&candidate),
    }
}

fn convert_one_track(
    track: &Track,
    spec: &ConversionSpec,
    manifest_root: &Path,
    source_backup_root: &Path,
    archive_conflict_resolution: ConflictResolution,
    output_conflict_resolution: ConflictResolution,
    png_encoder_available: bool,
) -> Result<(Track, PathBuf, PathBuf, bool), String> {
    let source = Path::new(&track.full_path);
    if !path_exists(source)? {
        return Err(format!("source file not found: {}", source.display()));
    }

    let source_probe = probe_audio(source)?;
    let source_sample_rate = source_probe.sample_rate.or(track.sample_rate);
    let target_sample_rate = target_sample_rate_for_source(source_sample_rate);
    let source_bitrate = source_bitrate_kbps(track, &source_probe);
    let mut archive_path =
        build_source_archive_path(source, source_bitrate, archive_conflict_resolution)?;
    let mut output_path = build_target_path(source, spec, output_conflict_resolution)?;

    backup_file_tree(source, source_backup_root)?;
    conversion_session::append_manifest_entry(
        manifest_root,
        &conversion_session::ConversionManifestEntry {
            track_id: track.id.clone(),
            source_path: track.full_path.clone(),
            archive_path: archive_path.to_string_lossy().to_string(),
            output_path: output_path.to_string_lossy().to_string(),
        },
    )?;

    if path_exists(&archive_path)? {
        match archive_conflict_resolution {
            ConflictResolution::Error => {
                return Err(format!(
                    "source archive already exists, refusing to overwrite: {}",
                    archive_path.display()
                ));
            }
            ConflictResolution::Overwrite => remove_file_path(&archive_path)?,
            ConflictResolution::Redirect => {
                archive_path = unique_redirect_path(&archive_path)?;
            }
        }
    }

    rename_path(source, &archive_path)?;

    let output_parent = output_path
        .parent()
        .ok_or_else(|| format!("missing output parent for {}", output_path.display()))?;
    let temp_output = TempBuilder::new()
        .prefix(".rkb-lossless-")
        .suffix(&format!(".{}", spec.extension))
        .tempfile_in(output_parent)
        .map_err(|e| {
            io_error_message(
                &format!(
                    "failed to create temporary output file in {}",
                    output_parent.display()
                ),
                &e,
            )
        })?;
    let temp_output_path = temp_output.path().to_path_buf();
    drop(temp_output);

    let mut skipped_embedded_artwork = false;
    let conversion_result = (|| -> Result<(), String> {
        let cover_art_supported = spec.supports_embedded_artwork();
        let has_attached_pic = cover_art_supported && source_probe.has_attached_pic;

        let mut ffmpeg = prepared_command("ffmpeg")?;
        ffmpeg.args(["-hide_banner", "-loglevel", "error", "-y", "-i"]);
        ffmpeg.arg(&archive_path);
        ffmpeg.args([
            "-map",
            "0:a:0",
            "-map_metadata",
            "0",
            "-c:a",
            spec.ffmpeg_codec,
        ]);
        if has_attached_pic {
            if png_encoder_available {
                ffmpeg.args([
                    "-map",
                    "0:v:0?",
                    "-c:v",
                    "png",
                    "-disposition:v:0",
                    "attached_pic",
                ]);
            } else {
                skipped_embedded_artwork = true;
            }
        }
        if spec.extension == "wav" {
            ffmpeg.arg("-vn");
        }
        if spec.extension == "wav" || spec.extension == "aiff" || spec.extension == "m4a" {
            ffmpeg.args(["-ar", &target_sample_rate.to_string()]);
        }
        if let Some(bitrate) = spec.bitrate_kbps {
            ffmpeg.args(["-b:a", &format!("{bitrate}k")]);
        }
        if spec.extension == "aiff" {
            ffmpeg.args(["-write_id3v2", "1", "-id3v2_version", "3"]);
        }
        if spec.extension == "m4a" {
            ffmpeg.args(["-movflags", "+faststart"]);
        }
        ffmpeg.arg(&temp_output_path);

        let output = ffmpeg.output().map_err(|e| {
            io_error_message(
                &format!(
                    "failed to run ffmpeg while converting {} -> {}",
                    archive_path.display(),
                    temp_output_path.display()
                ),
                &e,
            )
        })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let detail = if !stderr.is_empty() { stderr } else { stdout };
            return Err(format!(
                "ffmpeg failed while converting {} -> {}: {}",
                archive_path.display(),
                temp_output_path.display(),
                detail
            ));
        }

        Ok(())
    })();

    if let Err(error) = conversion_result {
        let rollback_errors = rollback_current_conversion(&temp_output_path, &archive_path, source);
        return Err(append_rollback_errors(error, rollback_errors));
    }

    if path_exists(&output_path)? {
        match output_conflict_resolution {
            ConflictResolution::Error => {
                let rollback_errors =
                    rollback_current_conversion(&temp_output_path, &archive_path, source);
                return Err(append_rollback_errors(
                    format!("target file already exists: {}", output_path.display()),
                    rollback_errors,
                ));
            }
            ConflictResolution::Overwrite => {
                if let Err(error) = remove_file_path(&output_path) {
                    let rollback_errors =
                        rollback_current_conversion(&temp_output_path, &archive_path, source);
                    return Err(append_rollback_errors(error, rollback_errors));
                }
            }
            ConflictResolution::Redirect => {
                output_path = match unique_redirect_path(&output_path) {
                    Ok(path) => path,
                    Err(error) => {
                        let rollback_errors =
                            rollback_current_conversion(&temp_output_path, &archive_path, source);
                        return Err(append_rollback_errors(error, rollback_errors));
                    }
                };
            }
        }
    }

    if let Err(error) = rename_path(&temp_output_path, &output_path) {
        let rollback_errors = rollback_current_conversion(&temp_output_path, &archive_path, source);
        return Err(append_rollback_errors(error, rollback_errors));
    }

    let channels = source_probe.channels.unwrap_or(2);
    let sample_rate = if spec.extension == "mp3" {
        source_sample_rate.unwrap_or_else(|| track.sample_rate.unwrap_or(44_100))
    } else {
        target_sample_rate
    };
    let bitrate = spec
        .bitrate_kbps
        .unwrap_or_else(|| compute_pcm_bitrate(sample_rate, channels, spec.bit_depth));

    let mut converted = track.clone();
    converted.file_type = match spec.extension {
        "wav" => "WAV".to_string(),
        "aiff" => "AIFF".to_string(),
        "mp3" => "MP3".to_string(),
        "m4a" => "M4A".to_string(),
        _ => converted.file_type.clone(),
    };
    converted.codec_name = None;
    converted.bit_depth = Some(spec.bit_depth);
    converted.sample_rate = Some(sample_rate);
    converted.bitrate = Some(bitrate);
    converted.full_path = output_path.to_string_lossy().to_string();

    Ok((
        converted,
        output_path,
        archive_path,
        skipped_embedded_artwork,
    ))
}

fn conversion_review_playlist_name() -> String {
    format!("rekordport Converted {}", playlist_timestamp_label())
}

fn create_conversion_review_playlist(
    db_path: &Path,
    key: &str,
    tracks: &[Track],
) -> Result<Option<String>, String> {
    if tracks.is_empty() {
        return Ok(None);
    }

    let schema_columns = table_columns_map(db_path, key, &["djmdPlaylist", "djmdSongPlaylist"])?;
    let playlist_columns = schema_columns
        .get("djmdPlaylist")
        .ok_or_else(|| "missing djmdPlaylist schema".to_string())?;
    let song_playlist_columns = schema_columns
        .get("djmdSongPlaylist")
        .ok_or_else(|| "missing djmdSongPlaylist schema".to_string())?;

    for (table, columns) in [
        ("djmdPlaylist", playlist_columns),
        ("djmdSongPlaylist", song_playlist_columns),
    ] {
        if !has_column(columns, "ID") && table == "djmdPlaylist" {
            return Err("djmdPlaylist is missing ID column".to_string());
        }
    }
    if !has_column(playlist_columns, "Name") {
        return Err("djmdPlaylist is missing Name column".to_string());
    }
    if !has_column(song_playlist_columns, "ContentID")
        || !has_column(song_playlist_columns, "PlaylistID")
    {
        return Err("djmdSongPlaylist is missing ContentID or PlaylistID column".to_string());
    }

    let playlist_id = next_numeric_text_id(db_path, key, "djmdPlaylist")?;
    let playlist_uuid = Uuid::new_v4().to_string();
    let playlist_name = conversion_review_playlist_name();
    let now_expr = "strftime('%Y-%m-%d %H:%M:%f +00:00','now')";
    let sequence_expr = "COALESCE((SELECT MAX(CAST(Seq AS INTEGER)) FROM djmdPlaylist), 0) + 1";
    let mut playlist_columns_to_insert = Vec::new();
    for column in playlist_columns {
        if matches!(
            column.as_str(),
            "ID" | "Seq"
                | "Name"
                | "ImagePath"
                | "Attribute"
                | "ParentID"
                | "SmartList"
                | "UUID"
                | "rb_data_status"
                | "rb_local_data_status"
                | "rb_local_deleted"
                | "rb_local_synced"
                | "usn"
                | "rb_local_usn"
                | "created_at"
                | "updated_at"
        ) {
            playlist_columns_to_insert.push(column.clone());
        }
    }
    let playlist_values: Vec<String> = playlist_columns_to_insert
        .iter()
        .map(|column| match column.as_str() {
            "ID" => sql_quote(&playlist_id),
            "Seq" => sequence_expr.to_string(),
            "Name" => sql_quote(&playlist_name),
            "ImagePath" | "SmartList" => "''".to_string(),
            "Attribute" => "0".to_string(),
            "ParentID" => sql_quote("root"),
            "UUID" => sql_quote(&playlist_uuid),
            "rb_data_status" | "rb_local_data_status" | "rb_local_deleted" | "rb_local_synced" => {
                "0".to_string()
            }
            "usn" | "rb_local_usn" => "NULL".to_string(),
            "created_at" | "updated_at" => now_expr.to_string(),
            _ => unreachable!("playlist insert columns are filtered"),
        })
        .collect();

    let mut song_playlist_columns_to_insert = Vec::new();
    for column in song_playlist_columns {
        if matches!(
            column.as_str(),
            "ID" | "UUID"
                | "ContentID"
                | "PlaylistID"
                | "TrackNo"
                | "Seq"
                | "rb_data_status"
                | "rb_local_data_status"
                | "rb_local_deleted"
                | "rb_local_synced"
                | "usn"
                | "rb_local_usn"
                | "created_at"
                | "updated_at"
        ) {
            song_playlist_columns_to_insert.push(column.clone());
        }
    }

    let mut sql = String::from("BEGIN IMMEDIATE;\n");
    sql.push_str(&format!(
        "INSERT INTO djmdPlaylist ({columns}) VALUES ({values});\n",
        columns = playlist_columns_to_insert.join(", "),
        values = playlist_values.join(", "),
    ));

    for (index, track) in tracks.iter().enumerate() {
        let mut values = Vec::new();
        let row_uuid = Uuid::new_v4().to_string();
        for column in &song_playlist_columns_to_insert {
            values.push(match column.as_str() {
                "ID" | "UUID" => sql_quote(&row_uuid),
                "ContentID" => sql_quote(&track.id),
                "PlaylistID" => sql_quote(&playlist_id),
                "TrackNo" | "Seq" => (index + 1).to_string(),
                "rb_data_status"
                | "rb_local_data_status"
                | "rb_local_deleted"
                | "rb_local_synced" => "0".to_string(),
                "usn" | "rb_local_usn" => "NULL".to_string(),
                "created_at" | "updated_at" => now_expr.to_string(),
                _ => unreachable!("song playlist insert columns are filtered"),
            });
        }
        sql.push_str(&format!(
            "INSERT INTO djmdSongPlaylist ({columns}) SELECT {values} WHERE EXISTS (SELECT 1 FROM djmdPlaylist WHERE ID = {playlist_id});\n",
            columns = song_playlist_columns_to_insert.join(", "),
            values = values.join(", "),
            playlist_id = sql_quote(&playlist_id),
        ));
    }

    sql.push_str(&format!(
        "SELECT COUNT(*) FROM djmdPlaylist WHERE ID = {};\n",
        sql_quote(&playlist_id)
    ));
    sql.push_str("COMMIT;\n");

    let inserted_count = sqlcipher_lines(db_path, key, &sql)?
        .last()
        .cloned()
        .unwrap_or_default();
    if inserted_count != "1" {
        return Err("could not create review playlist".to_string());
    }

    Ok(Some(playlist_name))
}

fn migrate_tracks_in_db(
    db_path: &Path,
    tracks: &[Track],
    output_tracks: &[Track],
    key: &str,
    spec: &ConversionSpec,
) -> Result<Vec<Track>, String> {
    let schema_columns = table_columns_map(
        db_path,
        key,
        &[
            "djmdContent",
            "djmdCue",
            "contentActiveCensor",
            "djmdActiveCensor",
            "djmdMixerParam",
            "djmdPlaylist",
            "djmdSongPlaylist",
            "djmdSongMyTag",
            "djmdSongTagList",
            "djmdSongHotCueBanklist",
            "djmdSongHistory",
            "djmdSongRelatedTracks",
            "djmdSongSampler",
            "djmdRecommendLike",
            "contentFile",
            "contentCue",
        ],
    )?;
    let content_columns = schema_columns
        .get("djmdContent")
        .cloned()
        .ok_or_else(|| "missing djmdContent schema".to_string())?;
    let insert_columns = content_columns;
    let djmd_cue_columns = schema_columns.get("djmdCue").cloned().unwrap_or_default();
    let now_expr = "strftime('%Y-%m-%d %H:%M:%f +00:00','now')";
    let mut copied_resources: Vec<PathBuf> = Vec::new();
    let mut pending_content_cue_rewrites: Vec<ContentCueRewrite> = Vec::with_capacity(tracks.len());
    let result = (|| -> Result<Vec<Track>, String> {
        let mut sql = String::from("BEGIN IMMEDIATE;\n");
        sql.push_str(
            "CREATE TEMP TABLE IF NOT EXISTS migration_state (next_id INTEGER NOT NULL);\n",
        );
        sql.push_str("DELETE FROM migration_state;\n");
        sql.push_str("INSERT INTO migration_state (next_id) SELECT COALESCE(MAX(CAST(ID AS INTEGER)), 0) + 1 FROM djmdContent WHERE ID <> '' AND ID NOT GLOB '*[^0-9]*';\n");
        sql.push_str("CREATE TEMP TABLE IF NOT EXISTS migration_results (source_id TEXT NOT NULL, new_id TEXT NOT NULL, new_uuid TEXT NOT NULL, offset_ms INTEGER NOT NULL);\n");
        sql.push_str("DELETE FROM migration_results;\n");

        let new_content_id_expr = "(SELECT CAST(next_id AS TEXT) FROM migration_state LIMIT 1)";
        let mut analysis_summaries: Vec<(String, String)> = Vec::with_capacity(tracks.len());
        let track_ids: Vec<&str> = tracks.iter().map(|track| track.id.as_str()).collect();
        let source_data_map =
            fetch_track_migration_source_data_map(db_path, key, &track_ids, &schema_columns)?;

        for (track, output_track) in tracks.iter().zip(output_tracks.iter()) {
            let output_path = Path::new(&output_track.full_path);
            let file_name = output_path
                .file_name()
                .ok_or_else(|| format!("missing file name for {}", output_path.display()))?
                .to_string_lossy()
                .to_string();
            let folder_path = output_path.to_string_lossy().to_string();
            let file_size = metadata_path(output_path)?.len();
            let encoder_priming_offset_ms = encoder_priming_compensation_ms(
                spec.extension,
                output_path,
                output_track.sample_rate.unwrap_or(44_100),
            )?;
            let source_data = source_data_map
                .get(&track.id)
                .ok_or_else(|| format!("missing migration source data for track {}", track.id))?;
            let old_uuid = &source_data.old_uuid;
            let old_analysis_path = &source_data.old_analysis_path;
            let content_files = &source_data.content_files;
            let content_uuid = Uuid::new_v4().to_string();
            pending_content_cue_rewrites.push(ContentCueRewrite {
                old_content_id: track.id.clone(),
                old_content_uuid: old_uuid.clone(),
                new_content_id: String::new(),
                new_content_uuid: content_uuid.clone(),
                offset_ms: encoder_priming_offset_ms,
            });
            let select_columns: Vec<String> = insert_columns
                .iter()
                .map(|column| match column.as_str() {
                    "ID" => new_content_id_expr.to_string(),
                    "UUID" => sql_quote(&content_uuid),
                    "MasterSongID" => new_content_id_expr.to_string(),
                    _ => column.clone(),
                })
                .collect();
            let mut migrated_content_files: Vec<MigratedContentFile> = Vec::new();
            let mut missing_analysis_resource = false;
            let source_has_analysis = !content_files.is_empty() || !old_analysis_path.is_empty();

            match validate_analysis_resources(content_files) {
                Ok(validated_files) => {
                    for file in validated_files {
                        let source_path =
                            file.original.rb_local_path.as_ref().ok_or_else(|| {
                                format!("analysis resource path missing for {}", file.original.id)
                            })?;
                        let destination_path = rewrite_analysis_resource_path(
                            source_path,
                            old_uuid,
                            file.original.uuid.as_deref(),
                            &content_uuid,
                        );
                        let destination = PathBuf::from(&destination_path);
                        duplicate_file_with_parent_dirs(&file.source, &destination)?;
                        rewrite_anlz_ppth(&destination, &file_name)?;
                        compensate_anlz_encoder_priming(&destination, encoder_priming_offset_ms)?;
                        copied_resources.push(destination.clone());
                        let size = metadata_path(&destination)?.len();
                        let hash = md5_hex(&destination)?;
                        let new_id = rewrite_analysis_resource_value(
                            &file.original.id,
                            old_uuid,
                            file.original.uuid.as_deref(),
                            &content_uuid,
                        );
                        let new_path = rewrite_analysis_resource_path(
                            &file.original.path,
                            old_uuid,
                            file.original.uuid.as_deref(),
                            &content_uuid,
                        );
                        let new_local_path = file.original.rb_local_path.as_ref().map(|path| {
                            rewrite_analysis_resource_path(
                                path,
                                old_uuid,
                                file.original.uuid.as_deref(),
                                &content_uuid,
                            )
                        });
                        migrated_content_files.push(MigratedContentFile {
                            original: file.original,
                            new_id,
                            new_uuid: Some(content_uuid.clone()),
                            new_path,
                            new_local_path,
                            hash,
                            size,
                        });
                    }
                }
                Err(error) => {
                    if !content_files.is_empty() {
                        return Err(format!(
              "source analysis is not safe to migrate for '{}': {}. Re-analyze this track in Rekordbox before converting if you want to preserve beat grid.",
              track.title, error
            ));
                    }
                    missing_analysis_resource = true;
                }
            }

            let analysis_summary = if !migrated_content_files.is_empty() {
                (
                    "migrated".to_string(),
                    "Existing beat grid / waveform migrated".to_string(),
                )
            } else if source_has_analysis || missing_analysis_resource {
                (
                    "none".to_string(),
                    "The source track does not have analysis files that can be migrated"
                        .to_string(),
                )
            } else {
                (
          "none".to_string(),
          "The source track does not have analysis files. You can re-analyze it later in rekordbox.".to_string(),
        )
            };
            analysis_summaries.push(analysis_summary);

            sql.push_str(&format!(
        "INSERT INTO djmdContent ({columns}) SELECT {select_columns} FROM djmdContent WHERE ID = {source_id};\n",
        columns = insert_columns.join(", "),
        select_columns = select_columns.join(", "),
        source_id = sql_quote(&track.id),
      ));

            let new_analysis_path = if old_analysis_path.is_empty() || missing_analysis_resource {
                old_analysis_path.clone()
            } else {
                rewrite_analysis_resource_path(old_analysis_path, old_uuid, None, &content_uuid)
            };
            let new_analysis_path = if missing_analysis_resource {
                String::new()
            } else {
                new_analysis_path
            };

            let mut content_assignments = Vec::new();
            if has_column(&insert_columns, "FolderPath") {
                content_assignments.push(format!("FolderPath = {}", sql_quote(&folder_path)));
            }
            if has_column(&insert_columns, "FileNameL") {
                content_assignments.push(format!("FileNameL = {}", sql_quote(&file_name)));
            }
            if has_column(&insert_columns, "FileNameS") {
                content_assignments.push(format!("FileNameS = {}", sql_quote(&file_name)));
            }
            if has_column(&insert_columns, "AnalysisDataPath") {
                content_assignments.push(format!(
                    "AnalysisDataPath = {}",
                    sql_quote(&new_analysis_path)
                ));
            }
            if has_column(&insert_columns, "FileType") {
                content_assignments.push(format!("FileType = {}", spec.file_type));
            }
            if has_column(&insert_columns, "BitDepth") {
                content_assignments.push(format!("BitDepth = {}", spec.bit_depth));
            }
            if has_column(&insert_columns, "BitRate") {
                content_assignments
                    .push(format!("BitRate = {}", output_track.bitrate.unwrap_or(0)));
            }
            if has_column(&insert_columns, "SampleRate") {
                content_assignments.push(format!(
                    "SampleRate = {}",
                    output_track.sample_rate.unwrap_or(44_100)
                ));
            }
            if has_column(&insert_columns, "FileSize") {
                content_assignments.push(format!("FileSize = {}", file_size));
            }
            if has_column(&insert_columns, "updated_at") {
                content_assignments.push(format!("updated_at = {now_expr}"));
            }
            if !content_assignments.is_empty() {
                sql.push_str(&format!(
                    "UPDATE djmdContent SET {} WHERE ID = {new_content_id_expr};\n",
                    content_assignments.join(", "),
                ));
            }

            if schema_has_table(&schema_columns, "djmdCue") {
                sql.push_str(&djmd_cue_migration_sql(
                    &djmd_cue_columns,
                    new_content_id_expr,
                    &content_uuid,
                    &track.id,
                    encoder_priming_offset_ms,
                    now_expr,
                ));
            }
            if schema_has_column(&schema_columns, "contentActiveCensor", "ContentID") {
                let mut assignments = Vec::new();
                if schema_has_column(&schema_columns, "contentActiveCensor", "ID") {
                    assignments.push(format!(
                        "ID = REPLACE(ID, {}, {})",
                        sql_quote(old_uuid),
                        sql_quote(&content_uuid)
                    ));
                }
                assignments.push(format!("ContentID = {new_content_id_expr}"));
                if let Some(updated_at) =
                    updated_at_assignment(&schema_columns, "contentActiveCensor", now_expr)
                {
                    assignments.push(updated_at);
                }
                sql.push_str(&format!(
                    "UPDATE contentActiveCensor SET {} WHERE ContentID = {};\n",
                    assignments.join(", "),
                    sql_quote(&track.id),
                ));
            }
            if schema_has_column(&schema_columns, "djmdActiveCensor", "ContentID") {
                let mut assignments = vec![format!("ContentID = {new_content_id_expr}")];
                if schema_has_column(&schema_columns, "djmdActiveCensor", "ID") {
                    assignments.push(format!(
                        "ID = REPLACE(ID, {}, {})",
                        sql_quote(old_uuid),
                        sql_quote(&content_uuid)
                    ));
                }
                if schema_has_column(&schema_columns, "djmdActiveCensor", "ContentUUID") {
                    assignments.push(format!("ContentUUID = {}", sql_quote(&content_uuid)));
                }
                if let Some(updated_at) =
                    updated_at_assignment(&schema_columns, "djmdActiveCensor", now_expr)
                {
                    assignments.push(updated_at);
                }
                sql.push_str(&format!(
                    "UPDATE djmdActiveCensor SET {} WHERE ContentID = {};\n",
                    assignments.join(", "),
                    sql_quote(&track.id),
                ));
            }
            for table in [
                "djmdMixerParam",
                "djmdSongMyTag",
                "djmdSongTagList",
                "djmdSongHotCueBanklist",
                "djmdSongHistory",
                "djmdSongRelatedTracks",
                "djmdSongSampler",
            ] {
                if let Some(statement) = update_content_id_sql(
                    &schema_columns,
                    table,
                    new_content_id_expr,
                    &track.id,
                    now_expr,
                ) {
                    sql.push_str(&statement);
                }
            }
            if schema_has_column(&schema_columns, "djmdSongPlaylist", "ContentID")
                && schema_has_column(&schema_columns, "djmdSongPlaylist", "PlaylistID")
                && schema_has_column(&schema_columns, "djmdPlaylist", "ID")
            {
                let mut assignments = vec![format!("ContentID = {new_content_id_expr}")];
                if let Some(updated_at) =
                    updated_at_assignment(&schema_columns, "djmdSongPlaylist", now_expr)
                {
                    assignments.push(updated_at);
                }
                // Smart playlists are rule-based; rekordbox refreshes them when the library opens.
                let smart_list_filter =
                    if schema_has_column(&schema_columns, "djmdPlaylist", "SmartList") {
                        "COALESCE(SmartList, '') = ''"
                    } else {
                        "1 = 1"
                    };
                sql.push_str(&format!(
                    "UPDATE djmdSongPlaylist SET {} WHERE ContentID = {} AND PlaylistID IN (SELECT ID FROM djmdPlaylist WHERE {smart_list_filter});\n",
                    assignments.join(", "),
                    sql_quote(&track.id),
                ));
            }
            if schema_has_column(&schema_columns, "djmdRecommendLike", "ContentID1")
                && schema_has_column(&schema_columns, "djmdRecommendLike", "ContentID2")
            {
                let mut assignments = vec![
                    format!(
                        "ContentID1 = CASE WHEN ContentID1 = {} THEN {new_content_id_expr} ELSE ContentID1 END",
                        sql_quote(&track.id)
                    ),
                    format!(
                        "ContentID2 = CASE WHEN ContentID2 = {} THEN {new_content_id_expr} ELSE ContentID2 END",
                        sql_quote(&track.id)
                    ),
                ];
                if let Some(updated_at) =
                    updated_at_assignment(&schema_columns, "djmdRecommendLike", now_expr)
                {
                    assignments.push(updated_at);
                }
                sql.push_str(&format!(
                    "UPDATE djmdRecommendLike SET {} WHERE ContentID1 = {} OR ContentID2 = {};\n",
                    assignments.join(", "),
                    sql_quote(&track.id),
                    sql_quote(&track.id),
                ));
            }
            for file in &migrated_content_files {
                if !schema_has_column(&schema_columns, "contentFile", "ID")
                    || !schema_has_column(&schema_columns, "contentFile", "ContentID")
                {
                    continue;
                }
                let new_local_path = file.new_local_path.clone().unwrap_or_default();
                let mut assignments = vec![
                    format!("ID = {}", sql_quote(&file.new_id)),
                    format!("ContentID = {new_content_id_expr}"),
                ];
                if schema_has_column(&schema_columns, "contentFile", "UUID") {
                    assignments.push(format!(
                        "UUID = {}",
                        file.new_uuid
                            .as_ref()
                            .map(|uuid| sql_quote(uuid))
                            .unwrap_or_else(|| "UUID".to_string())
                    ));
                }
                if schema_has_column(&schema_columns, "contentFile", "Path") {
                    assignments.push(format!("Path = {}", sql_quote(&file.new_path)));
                }
                if schema_has_column(&schema_columns, "contentFile", "rb_local_path") {
                    assignments.push(format!(
                        "rb_local_path = {}",
                        if new_local_path.is_empty() {
                            "NULL".to_string()
                        } else {
                            sql_quote(&new_local_path)
                        }
                    ));
                }
                if schema_has_column(&schema_columns, "contentFile", "Hash") {
                    assignments.push(format!("Hash = {}", sql_quote(&file.hash)));
                }
                if schema_has_column(&schema_columns, "contentFile", "Size") {
                    assignments.push(format!("Size = {}", file.size));
                }
                if let Some(updated_at) =
                    updated_at_assignment(&schema_columns, "contentFile", now_expr)
                {
                    assignments.push(updated_at);
                }
                sql.push_str(&format!(
                    "UPDATE contentFile SET {} WHERE ID = {} AND ContentID = {};\n",
                    assignments.join(", "),
                    sql_quote(&file.original.id),
                    sql_quote(&track.id),
                ));
            }

            for file in content_files {
                if !schema_has_column(&schema_columns, "contentFile", "ID")
                    || !schema_has_column(&schema_columns, "contentFile", "ContentID")
                {
                    continue;
                }
                if migrated_content_files
                    .iter()
                    .any(|candidate| candidate.original.id == file.id)
                {
                    continue;
                }
                sql.push_str(&format!(
                    "DELETE FROM contentFile WHERE ID = {} AND ContentID = {};\n",
                    sql_quote(&file.id),
                    sql_quote(&track.id),
                ));
            }

            sql.push_str(&format!(
                "DELETE FROM djmdContent WHERE ID = {};\n",
                sql_quote(&track.id),
            ));
            sql.push_str(&format!(
        "INSERT INTO migration_results (source_id, new_id, new_uuid, offset_ms) VALUES ({}, {new_content_id_expr}, {}, {});\n",
        sql_quote(&track.id),
        sql_quote(&content_uuid),
        encoder_priming_offset_ms,
      ));
            sql.push_str("UPDATE migration_state SET next_id = next_id + 1;\n");
        }

        if schema_has_column(&schema_columns, "contentCue", "ContentID") {
            let mut assignments = Vec::new();
            if schema_has_column(&schema_columns, "contentCue", "ID") {
                assignments.push(
                    "ID = (SELECT new_uuid FROM migration_results WHERE source_id = contentCue.ContentID LIMIT 1)"
                        .to_string(),
                );
            }
            assignments.push(
                "ContentID = (SELECT new_id FROM migration_results WHERE source_id = contentCue.ContentID LIMIT 1)"
                    .to_string(),
            );
            if schema_has_column(&schema_columns, "contentCue", "Cues") {
                assignments.push(
                    "Cues = CASE
    WHEN Cues IS NULL THEN NULL
    WHEN json_type(Cues) = 'array' THEN COALESCE((SELECT json_group_array(CASE WHEN json_type(value) = 'object' THEN json_set(json_set(value, '$.ContentID', (SELECT new_id FROM migration_results WHERE source_id = contentCue.ContentID LIMIT 1)), '$.ContentUUID', (SELECT new_uuid FROM migration_results WHERE source_id = contentCue.ContentID LIMIT 1)) ELSE value END) FROM json_each(contentCue.Cues)), '[]')
    WHEN json_type(Cues) = 'object' THEN json_set(json_set(Cues, '$.ContentID', (SELECT new_id FROM migration_results WHERE source_id = contentCue.ContentID LIMIT 1)), '$.ContentUUID', (SELECT new_uuid FROM migration_results WHERE source_id = contentCue.ContentID LIMIT 1))
    ELSE Cues
  END"
                    .to_string(),
                );
                if schema_has_column(&schema_columns, "contentCue", "rb_cue_count") {
                    assignments.push(
                        "rb_cue_count = CASE
    WHEN Cues IS NULL THEN COALESCE(rb_cue_count, 0)
    WHEN json_type(Cues) = 'array' THEN COALESCE(json_array_length(Cues), 0)
    ELSE COALESCE(rb_cue_count, 0)
  END"
                        .to_string(),
                    );
                }
            }
            if let Some(updated_at) = updated_at_assignment(&schema_columns, "contentCue", now_expr)
            {
                assignments.push(updated_at);
            }
            sql.push_str(&format!(
                "UPDATE contentCue SET\n  {}\nWHERE ContentID IN (SELECT source_id FROM migration_results);\n",
                assignments.join(",\n  "),
            ));
        }

        sql.push_str("SELECT source_id || '|' || new_id FROM migration_results ORDER BY rowid;\n");
        sql.push_str("COMMIT;\n");
        let returned_rows = sqlcipher_lines(db_path, key, &sql)?;
        if returned_rows.len() != tracks.len() {
            return Err(format!(
                "expected {} migrated content ids, but sqlcipher returned {}",
                tracks.len(),
                returned_rows.len()
            ));
        }

        let mut migrated_tracks = Vec::with_capacity(tracks.len());
        for (((track, output_track), row), (analysis_state, analysis_note)) in tracks
            .iter()
            .zip(output_tracks.iter())
            .zip(returned_rows.into_iter())
            .zip(analysis_summaries.into_iter())
        {
            let (_, new_id) = row
                .split_once('|')
                .ok_or_else(|| format!("unexpected migration result row: {row}"))?;
            let mut migrated = output_track.clone();
            migrated.id = new_id.to_string();
            if let Some(rewrite) = pending_content_cue_rewrites.get_mut(migrated_tracks.len()) {
                rewrite.new_content_id = new_id.to_string();
            }
            migrated.source_id = Some(track.id.clone());
            migrated.analysis_state = Some(analysis_state);
            migrated.analysis_note = Some(analysis_note);
            migrated_tracks.push(migrated);
        }

        if schema_has_column(&schema_columns, "contentCue", "ContentID")
            && schema_has_column(&schema_columns, "contentCue", "Cues")
        {
            rewrite_content_cues_rows(
                db_path,
                key,
                &pending_content_cue_rewrites,
                now_expr,
                schema_has_column(&schema_columns, "contentCue", "rb_cue_count"),
                schema_has_column(&schema_columns, "contentCue", "updated_at"),
            )?;
        }

        Ok(migrated_tracks)
    })();

    if result.is_err() {
        for path in copied_resources {
            let _ = remove_file_path(&path);
        }
    }

    result
}

fn convert_impl_with_progress<F>(
    req: ConvertRequest,
    mut on_progress: F,
) -> Result<ConvertResponse, String>
where
    F: FnMut(ScanProgressPayload),
{
    refresh_command_discovery_caches();
    if req.tracks.is_empty() {
        return Err("no tracks selected".into());
    }

    if !command_available("ffmpeg") {
        return Err("ffmpeg command not found in PATH or bundled sidecar".into());
    }

    let spec = preset_spec(&req.preset)?;
    let source_handling = source_handling_mode(&req.source_handling)?;
    let archive_conflict_resolution =
        conflict_resolution_mode(req.archive_conflict_resolution.as_deref())?;
    let output_conflict_resolution =
        conflict_resolution_mode(req.output_conflict_resolution.as_deref())?;
    let db_path = PathBuf::from(&req.db_path);
    if !path_exists(&db_path)? {
        return Err(format!("database file not found: {}", db_path.display()));
    }
    if !check_sqlcipher_json_available(&db_path, DEFAULT_KEY) {
        return Err(
            "sqlcipher was built without SQLite JSON functions required for cue migration"
                .to_string(),
        );
    }

    if spec.extension == "m4a" && !ffmpeg_has_encoder("aac_at")? {
        return Err(
            "ffmpeg was built without Apple's aac_at encoder, so M4A 320kbps is unavailable".into(),
        );
    }
    let png_encoder_available = ffmpeg_has_encoder("png").unwrap_or(false);
    let mut warnings = Vec::new();
    if let Some(backup_parent) = db_path.parent() {
        match conversion_session::recover_stale_conversion_backups(backup_parent, &db_path) {
            Ok(report) => {
                warnings.extend(report.warnings);
                warnings.extend(report.errors);
            }
            Err(error) => warnings.push(format!(
                "failed to recover interrupted conversion backups before starting: {}",
                error
            )),
        }
        match conversion_session::cleanup_completed_conversion_backups(backup_parent) {
            Ok(report) => warnings.extend(report.warnings),
            Err(error) => warnings.push(format!(
                "failed to clean completed conversion backups before starting: {}",
                error
            )),
        }
        match conversion_session::cleanup_successful_music_backups(backup_parent) {
            Ok(report) => warnings.extend(report.warnings),
            Err(error) => warnings.push(format!(
                "failed to clean successful music backups before starting: {}",
                error
            )),
        }
        match conversion_session::cleanup_successful_database_backups(backup_parent, 1) {
            Ok(report) => warnings.extend(report.warnings),
            Err(error) => warnings.push(format!(
                "failed to clean old database backups before starting: {}",
                error
            )),
        }
    }

    let backup_root = db_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(
            "{}{}",
            conversion_session::BACKUP_DIR_PREFIX,
            Uuid::new_v4()
        ));
    create_dir_path(&backup_root)?;

    let db_backup = backup_root.join("master.db");
    copy_path(&db_path, &db_backup)?;

    let music_backup_root = conversion_session::music_backup_path(&backup_root);
    let mut session = conversion_session::ConversionSession::new();
    let mut skipped_embedded_artwork_count = 0usize;
    let total_tracks = req.tracks.len();

    on_progress(ScanProgressPayload {
        phase: "preparing".to_string(),
        current: 0,
        total: total_tracks,
        message: format!("Preparing conversion for 0 / {total_tracks} tracks…"),
    });

    for (index, track) in req.tracks.iter().enumerate() {
        let current = index;
        on_progress(ScanProgressPayload {
            phase: "processing".to_string(),
            current,
            total: total_tracks,
            message: format!("Converting {} / {} tracks…", current, total_tracks),
        });
        match convert_one_track(
            track,
            &spec,
            &backup_root,
            &music_backup_root,
            archive_conflict_resolution,
            output_conflict_resolution,
            png_encoder_available,
        ) {
            Ok((converted_track, output_path, archive_path, skipped_embedded_artwork)) => {
                if skipped_embedded_artwork {
                    skipped_embedded_artwork_count += 1;
                }
                session.push(track, converted_track, output_path, archive_path);
            }
            Err(error) => {
                let rollback_errors = session.rollback_all();
                if rollback_errors.is_empty() && !error_contains_rollback_failure(&error) {
                    let _ = conversion_session::remove_manifest(&backup_root);
                }
                return Err(append_rollback_errors(error, rollback_errors));
            }
        }
    }

    on_progress(ScanProgressPayload {
        phase: "migrating".to_string(),
        current: total_tracks,
        total: total_tracks,
        message: "Migrating metadata and analysis…".to_string(),
    });

    let converted_tracks = session.converted_tracks();
    let migrated_tracks =
        match migrate_tracks_in_db(&db_path, &req.tracks, &converted_tracks, DEFAULT_KEY, &spec) {
            Ok(tracks) => tracks,
            Err(error) => {
                let mut rollback_errors = session.rollback_all();
                rollback_errors.extend(restore_database_backup(&db_backup, &db_path));
                if rollback_errors.is_empty() {
                    let _ = conversion_session::remove_manifest(&backup_root);
                }
                return Err(append_rollback_errors(error, rollback_errors));
            }
        };
    let verification_playlist_name = match create_conversion_review_playlist(
        &db_path,
        DEFAULT_KEY,
        &migrated_tracks,
    ) {
        Ok(name) => name,
        Err(error) => {
            warnings.push(format!(
                    "Converted files and database changes were saved, but the review playlist could not be created automatically: {}",
                    error
                ));
            None
        }
    };

    if let Err(error) = conversion_session::mark_manifest_completed(&backup_root) {
        warnings.push(format!(
            "Converted files and database changes were saved, but the completion marker could not be written automatically: {}",
            error
        ));
    }
    if let Err(error) = conversion_session::remove_manifest(&backup_root) {
        if !backup_root.join("manifest.completed").exists() {
            warnings.push(format!(
                "Converted files and database changes were saved, but the interrupted-conversion manifest could not be removed automatically: {}",
                error
            ));
        }
    }
    if backup_root.join("manifest.completed").exists() {
        if let Err(error) = conversion_session::cleanup_successful_music_backup(&backup_root) {
            warnings.push(format!(
                "Converted files and database changes were saved, but the temporary music backup could not be removed automatically from {}: {}",
                backup_root.display(),
                error
            ));
        }
    }

    if skipped_embedded_artwork_count > 0 {
        warnings.push(
            format!(
                "Embedded cover art was skipped for {skipped_embedded_artwork_count} converted track(s) because the current ffmpeg build does not include the PNG encoder."
            ),
        );
    }
    let cleanup_report = match cleanup_orphan_zero_analysis_dirs(&db_path, DEFAULT_KEY) {
        Ok(report) => report,
        Err(error) => {
            warnings.push(format!(
                "Converted files and database changes were saved, but orphaned zero-byte analysis folders could not be archived automatically: {}",
                error
            ));
            CleanupReport::default()
        }
    };
    warnings.extend(cleanup_report.warnings.iter().cloned());
    let analysis_migrated_count = migrated_tracks
        .iter()
        .filter(|track| track.analysis_state.as_deref() == Some("migrated"))
        .count();
    let analysis_missing_count = migrated_tracks
        .len()
        .saturating_sub(analysis_migrated_count);
    let mut source_cleanup_failures = 0usize;

    if matches!(source_handling, SourceHandling::Trash) {
        for archive_path in session.archive_paths() {
            if trash::delete(archive_path).is_err() {
                source_cleanup_failures += 1;
            }
        }
    }
    if let Err(error) = conversion_session::write_conversion_receipts(
        &backup_root,
        &session,
        verification_playlist_name.as_deref(),
    ) {
        warnings.push(format!(
            "Converted files and database changes were saved, but the conversion summary could not be written automatically: {}",
            error
        ));
    }
    if let Some(backup_parent) = db_path.parent() {
        match conversion_session::cleanup_successful_database_backups(backup_parent, 1) {
            Ok(report) => warnings.extend(report.warnings),
            Err(error) => warnings.push(format!(
                "Converted files and database changes were saved, but old database backups could not be cleaned automatically: {}",
                error
            )),
        }
    }

    let response = ConvertResponse {
        backup_dir: backup_root.to_string_lossy().to_string(),
        converted_count: migrated_tracks.len(),
        analysis_migrated_count,
        analysis_missing_count,
        verification_playlist_name,
        source_cleanup_mode: source_handling_name(source_handling).to_string(),
        source_cleanup_failures,
        cleanup_archived_dirs: cleanup_report.archived_dirs,
        cleanup_archive_dir: cleanup_report.archive_dir,
        warnings,
        converted_tracks: migrated_tracks,
    };

    on_progress(ScanProgressPayload {
        phase: "done".to_string(),
        current: response.converted_count,
        total: total_tracks,
        message: format!(
            "Conversion complete. {} tracks processed.",
            response.converted_count
        ),
    });

    Ok(response)
}

fn scan_impl_with_progress<F>(req: ScanRequest, mut on_progress: F) -> Result<ScanResponse, String>
where
    F: FnMut(ScanProgressPayload),
{
    refresh_command_discovery_caches();
    on_progress(ScanProgressPayload {
        phase: "querying".to_string(),
        current: 0,
        total: 0,
        message: "Reading rekordbox database…".to_string(),
    });

    let library_total =
        library_track_total(Path::new(&req.db_path), DEFAULT_KEY, req.include_sampler)?;
    let outcome = scan_tracks_with_progress(
        Path::new(&req.db_path),
        DEFAULT_KEY,
        req.min_bit_depth,
        req.include_sampler,
        &mut on_progress,
    )?;
    let tracks = outcome.tracks;
    let flac = tracks
        .iter()
        .filter(|track| track.file_type == "FLAC")
        .count();
    let alac = tracks
        .iter()
        .filter(|track| track.file_type == "ALAC")
        .count();
    let hi_res = tracks
        .iter()
        .filter(|track| {
            matches!(track.file_type.as_str(), "WAV" | "AIFF")
                && track.scan_issue.as_deref() != Some("wav_extensible")
        })
        .count();
    let wav_extensible = tracks
        .iter()
        .filter(|track| track.scan_issue.as_deref() == Some("wav_extensible"))
        .count();

    let response = ScanResponse {
        summary: ScanSummary {
            library_total,
            candidate_total: outcome.stats.candidate_total,
            total: tracks.len(),
            flac,
            alac,
            hi_res,
            wav_extensible,
            m4a_candidates: outcome.stats.m4a_candidates,
            unreadable_m4a: outcome.stats.unreadable_m4a,
            non_alac_m4a: outcome.stats.non_alac_m4a,
            sampler_included: req.include_sampler,
            min_bit_depth: req.min_bit_depth,
            db_path: req.db_path,
        },
        tracks,
    };

    on_progress(ScanProgressPayload {
        phase: "done".to_string(),
        current: response.summary.total,
        total: response.summary.total,
        message: String::new(),
    });

    Ok(response)
}

#[cfg(test)]
fn scan_impl(req: ScanRequest) -> Result<ScanResponse, String> {
    scan_impl_with_progress(req, |_| {})
}
