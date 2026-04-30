fn run_sqlcipher(db_path: &Path, key: &str, sql: &str) -> Result<String, String> {
    if !command_available("sqlcipher") {
        return Err("sqlcipher command not found in PATH or bundled sidecar".into());
    }

    let script = format!(".bail on\nPRAGMA key = '{key}';\nPRAGMA foreign_keys = ON;\n{sql}\n");

    let mut sqlcipher = prepared_command("sqlcipher")?;
    let output = sqlcipher
        .arg(db_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(script.as_bytes())?;
            }
            child.wait_with_output()
        })
        .map_err(|e| {
            io_error_message(
                &format!("failed to run sqlcipher on {}", db_path.display()),
                &e,
            )
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        return Err(if !stderr.is_empty() { stderr } else { stdout });
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn sqlcipher_lines(db_path: &Path, key: &str, sql: &str) -> Result<Vec<String>, String> {
    let output = run_sqlcipher(db_path, key, sql)?;
    Ok(output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && *line != "ok")
        .map(str::to_string)
        .collect())
}

fn sqlcipher_single_value(db_path: &Path, key: &str, sql: &str) -> Result<Option<String>, String> {
    let lines = sqlcipher_lines(db_path, key, sql)?;
    Ok(lines.last().cloned())
}

fn sqlcipher_required_value(
    db_path: &Path,
    key: &str,
    sql: &str,
    error: &str,
) -> Result<String, String> {
    sqlcipher_single_value(db_path, key, sql)?.ok_or_else(|| error.to_string())
}

fn table_columns_map(
    db_path: &Path,
    key: &str,
    tables: &[&str],
) -> Result<HashMap<String, Vec<String>>, String> {
    if tables.is_empty() {
        return Ok(HashMap::new());
    }

    let sql = format!(
        ".headers on\n.mode csv\n{}\nORDER BY table_name, cid;",
        tables
            .iter()
            .map(|table| format!(
                "SELECT {} AS table_name, cid, name FROM pragma_table_info({})",
                sql_quote(table),
                sql_quote(table)
            ))
            .collect::<Vec<_>>()
            .join("\nUNION ALL\n")
    );

    let mut columns: HashMap<String, Vec<(usize, String)>> = HashMap::new();
    for record in sqlcipher_csv_records(db_path, key, &sql)? {
        let table_name = record.get(0).unwrap_or_default().to_string();
        let cid = record
            .get(1)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("missing column ordinal for table {table_name}"))?
            .parse::<usize>()
            .map_err(|error| error.to_string())?;
        let name = record.get(2).unwrap_or_default().to_string();
        columns.entry(table_name).or_default().push((cid, name));
    }

    Ok(columns
        .into_iter()
        .map(|(table_name, mut table_columns)| {
            table_columns.sort_by_key(|(cid, _)| *cid);
            (
                table_name,
                table_columns
                    .into_iter()
                    .map(|(_, name)| name)
                    .collect::<Vec<_>>(),
            )
        })
        .collect())
}

fn parse_optional_u32(value: Option<&str>) -> Option<u32> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<u32>().ok())
}

fn positive_u32(value: Option<u32>) -> Option<u32> {
    value.filter(|value| *value > 0)
}

fn is_hi_res_pcm_row(row: &ScanRow, min_bit_depth: u32) -> bool {
    matches!(row.file_type, 11 | 12)
        && (row.bit_depth.unwrap_or(0) > min_bit_depth
            || row.sample_rate.unwrap_or(0) > HI_RES_SAMPLE_RATE_THRESHOLD)
}

fn lossless_scan_bitrate(row: &ScanRow) -> Option<u32> {
    positive_u32(row.bitrate).or_else(|| {
        if !matches!(row.file_type, 5 | 6) || row.full_path.trim().is_empty() {
            return None;
        }

        let source = Path::new(&row.full_path);
        if !path_is_file(source) {
            return None;
        }

        positive_u32(probe_audio(source).ok()?.bitrate_kbps)
    })
}

fn probe_wav_format_tag(path: &Path) -> Result<Option<u16>, String> {
    let mut file = retry_io_operation(format!("failed to open {}", path.display()), || {
        fs::File::open(path)
    })?;

    let mut header = [0_u8; 12];
    match file.read_exact(&mut header) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => {
            return Err(io_error_message(
                &format!("failed to read WAV header from {}", path.display()),
                &error,
            ));
        }
    }

    if &header[..4] != b"RIFF" || &header[8..12] != b"WAVE" {
        return Ok(None);
    }

    loop {
        let mut chunk_header = [0_u8; 8];
        match file.read_exact(&mut chunk_header) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(error) => {
                return Err(io_error_message(
                    &format!("failed to read RIFF chunk header from {}", path.display()),
                    &error,
                ));
            }
        }

        let chunk_size =
            u32::from_le_bytes(chunk_header[4..8].try_into().expect("chunk size bytes")) as u64;

        if &chunk_header[..4] == b"fmt " {
            if chunk_size < 2 {
                return Ok(None);
            }

            let mut format_tag = [0_u8; 2];
            file.read_exact(&mut format_tag).map_err(|error| {
                io_error_message(
                    &format!("failed to read WAV fmt chunk from {}", path.display()),
                    &error,
                )
            })?;
            return Ok(Some(u16::from_le_bytes(format_tag)));
        }

        let padded_size = chunk_size + (chunk_size % 2);
        file.seek(SeekFrom::Current(padded_size as i64))
            .map_err(|error| {
                io_error_message(
                    &format!("failed to seek to next RIFF chunk in {}", path.display()),
                    &error,
                )
            })?;
    }
}

fn sampler_path_predicate(column: &str) -> String {
    format!(r"REPLACE(COALESCE({column}, ''), '\', '/') NOT LIKE '%/Sampler/%'")
}

fn percent_decode_path_value(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = &value[index + 1..index + 3];
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                decoded.push(byte);
                index += 3;
                continue;
            }
        }

        decoded.push(bytes[index]);
        index += 1;
    }

    String::from_utf8(decoded).unwrap_or_else(|_| value.to_string())
}

fn normalize_rekordbox_path_value(value: &str) -> String {
    let trimmed = value.trim();
    let path = trimmed
        .strip_prefix("file://localhost")
        .or_else(|| trimmed.strip_prefix("file://"))
        .unwrap_or(trimmed);
    let decoded = percent_decode_path_value(path);

    #[cfg(target_os = "windows")]
    {
        let bytes = decoded.as_bytes();
        if bytes.len() >= 3 && bytes[0] == b'/' && bytes[2] == b':' {
            return decoded[1..].to_string();
        }
    }

    decoded
}

fn path_is_file(path: &Path) -> bool {
    metadata_path(path)
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
}

fn resolve_rekordbox_audio_path(folder_path: &str, file_name_l: &str, file_name_s: &str) -> String {
    let base = normalize_rekordbox_path_value(folder_path);
    if base.is_empty() {
        return String::new();
    }

    let base_path = PathBuf::from(&base);
    if path_is_file(&base_path) {
        return base;
    }

    for file_name in [file_name_l, file_name_s] {
        let file_name = normalize_rekordbox_path_value(file_name);
        if file_name.is_empty() {
            continue;
        }

        let candidate = base_path.join(file_name);
        if path_is_file(&candidate) {
            return candidate.to_string_lossy().to_string();
        }
    }

    base
}

fn resolve_existing_rekordbox_audio_path(row: &ScanRow) -> Option<String> {
    let full_path =
        resolve_rekordbox_audio_path(&row.full_path, &row.file_name_l, &row.file_name_s);
    if full_path.trim().is_empty() || !path_is_file(Path::new(&full_path)) {
        return None;
    }

    Some(full_path)
}

fn build_scan_query(min_bit_depth: u32, include_sampler: bool) -> String {
    let sampler_filter = if include_sampler {
        String::new()
    } else {
        format!("\n  AND {}", sampler_path_predicate("c.FolderPath"))
    };

    format!(
        ".headers on\n.mode csv\nSELECT\n  COALESCE(c.ID, '') AS id,\n  COALESCE(c.Title, '') AS title,\n  COALESCE(a.Name, c.SrcArtistName, '') AS artist,\n  c.FileType AS file_type,\n  c.BitDepth AS bit_depth,\n  c.SampleRate AS sample_rate,\n  c.BitRate AS bitrate,\n  COALESCE(c.FolderPath, '') AS full_path,\n  COALESCE(c.FileNameL, '') AS file_name_l,\n  COALESCE(c.FileNameS, '') AS file_name_s\nFROM djmdContent c\nLEFT JOIN djmdArtist a ON a.ID = c.ArtistID\nWHERE\n  (\n    c.FileType = 5\n    OR c.FileType = 6\n    OR c.FileType = 11\n    OR (\n      c.FileType = 12\n      AND (\n        COALESCE(c.BitDepth, 0) > {min_bit_depth}\n        OR COALESCE(c.SampleRate, 0) > {HI_RES_SAMPLE_RATE_THRESHOLD}\n      )\n    )\n  ){sampler_filter}\nORDER BY\n  artist COLLATE NOCASE,\n  title COLLATE NOCASE,\n  full_path COLLATE NOCASE;"
    )
}

fn scan_rows(
    db_path: &Path,
    key: &str,
    min_bit_depth: u32,
    include_sampler: bool,
) -> Result<Vec<ScanRow>, String> {
    if !path_exists(db_path)? {
        return Err(format!("database file not found: {}", db_path.display()));
    }
    if !command_available("sqlcipher") {
        return Err("sqlcipher command not found in PATH or bundled sidecar".into());
    }
    let output = run_sqlcipher(
        db_path,
        key,
        &build_scan_query(min_bit_depth, include_sampler),
    )?;
    let filtered = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && *line != "ok")
        .collect::<Vec<_>>()
        .join("\n");
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_reader(filtered.as_bytes());

    let mut rows = Vec::new();
    for record in reader.records() {
        let record = record.map_err(|e| e.to_string())?;
        let file_type = record
            .get(3)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "missing file type in scan row".to_string())?
            .parse::<i32>()
            .map_err(|e| e.to_string())?;

        rows.push(ScanRow {
            id: record.get(0).unwrap_or_default().to_string(),
            title: record.get(1).unwrap_or_default().to_string(),
            artist: record.get(2).unwrap_or_default().to_string(),
            file_type,
            bit_depth: parse_optional_u32(record.get(4)),
            sample_rate: parse_optional_u32(record.get(5)),
            bitrate: parse_optional_u32(record.get(6)),
            full_path: record.get(7).unwrap_or_default().to_string(),
            file_name_l: record.get(8).unwrap_or_default().to_string(),
            file_name_s: record.get(9).unwrap_or_default().to_string(),
        });
    }

    Ok(rows)
}

fn scan_tracks_with_progress<F>(
    db_path: &Path,
    key: &str,
    min_bit_depth: u32,
    include_sampler: bool,
    mut on_progress: F,
) -> Result<ScanOutcome, String>
where
    F: FnMut(ScanProgressPayload),
{
    let rows = scan_rows(db_path, key, min_bit_depth, include_sampler)?;
    let total = rows.len();
    let mut stats = ScanStats {
        candidate_total: total,
        ..ScanStats::default()
    };
    let progress_step = (total / 120).max(1);
    on_progress(ScanProgressPayload {
        phase: "processing".to_string(),
        current: 0,
        total,
        message: if total == 0 {
            "No matching candidate tracks found".to_string()
        } else {
            format!("Inspecting 0 / {total} candidate tracks…")
        },
    });
    let mut tracks = Vec::new();

    for (index, row) in rows.into_iter().enumerate() {
        let hi_res_pcm = is_hi_res_pcm_row(&row, min_bit_depth);
        let mut scan_issue = None;
        let mut scan_note = None;
        let mut include_track = matches!(row.file_type, 5 | 6) || hi_res_pcm;
        let Some(full_path) = resolve_existing_rekordbox_audio_path(&row) else {
            let current = index + 1;
            if current == total || current == 1 || current % progress_step == 0 {
                on_progress(ScanProgressPayload {
                    phase: "processing".to_string(),
                    current,
                    total,
                    message: format!("Inspecting {current} / {total} candidate tracks…"),
                });
            }
            continue;
        };
        let row = ScanRow { full_path, ..row };

        if row.file_type == 11 && !hi_res_pcm {
            let source = Path::new(&row.full_path);
            if let Some(WAV_FORMAT_TAG_EXTENSIBLE) =
                probe_wav_format_tag(source).unwrap_or(None)
            {
                include_track = true;
                stats.wav_extensible += 1;
                scan_issue = Some("wav_extensible".to_string());
                scan_note = Some(
                    "WAV header uses WAVE_FORMAT_EXTENSIBLE. Some CDJ/XDJ players reject these files even when the bit depth and sample rate look compatible.".to_string(),
                );
            }
        }

        if !include_track {
            let current = index + 1;
            if current == total || current == 1 || current % progress_step == 0 {
                on_progress(ScanProgressPayload {
                    phase: "processing".to_string(),
                    current,
                    total,
                    message: format!("Inspecting {current} / {total} candidate tracks…"),
                });
            }
            continue;
        }

        let codec_name = if row.file_type == 6 {
            stats.m4a_candidates += 1;
            Some("alac".to_string())
        } else {
            None
        };
        let bitrate = lossless_scan_bitrate(&row);

        tracks.push(Track {
            id: row.id,
            source_id: None,
            scan_issue,
            scan_note,
            analysis_state: None,
            analysis_note: None,
            title: row.title,
            artist: row.artist,
            file_type: file_type_name(row.file_type, codec_name.as_deref()),
            codec_name,
            bit_depth: row.bit_depth,
            sample_rate: row.sample_rate,
            bitrate,
            full_path: row.full_path,
        });

        let current = index + 1;
        if current == total || current == 1 || current % progress_step == 0 {
            on_progress(ScanProgressPayload {
                phase: "processing".to_string(),
                current,
                total,
                message: format!("Inspecting {current} / {total} candidate tracks…"),
            });
        }
    }

    Ok(ScanOutcome { tracks, stats })
}

fn library_track_total(db_path: &Path, key: &str, include_sampler: bool) -> Result<usize, String> {
    let sampler_filter = if include_sampler {
        String::new()
    } else {
        format!("WHERE {}", sampler_path_predicate("FolderPath"))
    };
    let sql = format!("SELECT COUNT(*) FROM djmdContent {sampler_filter};");
    let value = sqlcipher_required_value(db_path, key, &sql, "failed to count library tracks")?;
    value
        .trim()
        .parse::<usize>()
        .map_err(|error| format!("failed to parse library track count: {error}"))
}
