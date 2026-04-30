fn rewrite_uuid_in_path(path: &str, old_uuid: &str, new_uuid: &str) -> String {
    let old_split = format!("{}/{}", &old_uuid[..3], &old_uuid[3..]);
    let new_split = format!("{}/{}", &new_uuid[..3], &new_uuid[3..]);
    let old_split_encoded = format!("{}%2F{}", &old_uuid[..3], &old_uuid[3..]);
    let new_split_encoded = format!("{}%2F{}", &new_uuid[..3], &new_uuid[3..]);

    let replaced_uuid = replace_ascii_case_insensitive(path, old_uuid, new_uuid);
    let replaced_split = replace_ascii_case_insensitive(&replaced_uuid, &old_split, &new_split);
    replace_ascii_case_insensitive(&replaced_split, &old_split_encoded, &new_split_encoded)
}

fn replace_ascii_case_insensitive(haystack: &str, needle: &str, replacement: &str) -> String {
    if needle.is_empty() {
        return haystack.to_string();
    }

    let haystack_lower = haystack.to_ascii_lowercase();
    let needle_lower = needle.to_ascii_lowercase();
    let mut rewritten = String::with_capacity(haystack.len());
    let mut cursor = 0usize;

    while let Some(found) = haystack_lower[cursor..].find(&needle_lower) {
        let start = cursor + found;
        let end = start + needle.len();
        rewritten.push_str(&haystack[cursor..start]);
        rewritten.push_str(replacement);
        cursor = end;
    }

    rewritten.push_str(&haystack[cursor..]);
    rewritten
}

fn rewrite_analysis_resource_value(
    value: &str,
    old_track_uuid: &str,
    old_file_uuid: Option<&str>,
    new_uuid: &str,
) -> String {
    let rewritten = rewrite_uuid_in_path(value, old_track_uuid, new_uuid);
    if rewritten != value {
        return rewritten;
    }

    let Some(old_file_uuid) = old_file_uuid.filter(|uuid| !uuid.is_empty()) else {
        return value.to_string();
    };

    rewrite_uuid_in_path(value, old_file_uuid, new_uuid)
}

fn fallback_analysis_resource_path(value: &str, new_uuid: &str) -> Option<String> {
    let source = Path::new(value);
    let file_name = source.file_name()?;
    let uuid_tail_dir = source.parent()?;
    let uuid_head_dir = uuid_tail_dir.parent()?;
    let base_dir = uuid_head_dir.parent()?;
    let uuid_head = uuid_head_dir.file_name()?.to_string_lossy();
    let uuid_tail = uuid_tail_dir.file_name()?.to_string_lossy();

    if uuid_head.len() != 3 || uuid_tail.is_empty() {
        return None;
    }

    let mut destination = base_dir.to_path_buf();
    destination.push(&new_uuid[..3]);
    destination.push(&new_uuid[3..]);
    destination.push(file_name);
    Some(destination.to_string_lossy().to_string())
}

fn rewrite_analysis_resource_path(
    value: &str,
    old_track_uuid: &str,
    old_file_uuid: Option<&str>,
    new_uuid: &str,
) -> String {
    let rewritten = rewrite_analysis_resource_value(value, old_track_uuid, old_file_uuid, new_uuid);
    if rewritten != value {
        return rewritten;
    }

    fallback_analysis_resource_path(value, new_uuid).unwrap_or_else(|| value.to_string())
}

fn sqlcipher_csv_records(
    db_path: &Path,
    key: &str,
    sql: &str,
) -> Result<Vec<csv::StringRecord>, String> {
    let output = run_sqlcipher(db_path, key, sql)?;
    let filtered = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && *line != "ok")
        .collect::<Vec<_>>()
        .join("\n");
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_reader(filtered.as_bytes());

    let mut records = Vec::new();
    for record in reader.records() {
        records.push(record.map_err(|error| error.to_string())?);
    }

    Ok(records)
}

fn decode_json_string_field(value: Option<&str>) -> Result<Option<String>, String> {
    match value {
        None => Ok(None),
        Some("") => Ok(None),
        Some(text) => serde_json::from_str::<String>(text)
            .map(Some)
            .map_err(|error| error.to_string()),
    }
}

fn decode_json_string_field_required(value: Option<&str>, error: &str) -> Result<String, String> {
    decode_json_string_field(value)?.ok_or_else(|| error.to_string())
}

fn fetch_track_migration_source_data_map(
    db_path: &Path,
    key: &str,
    content_ids: &[&str],
    schema_columns: &HashMap<String, Vec<String>>,
) -> Result<HashMap<String, TrackMigrationSourceData>, String> {
    if content_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let content_filter = content_ids
        .iter()
        .map(|content_id| sql_quote(content_id))
        .collect::<Vec<_>>()
        .join(", ");
    let content_uuid_expr = if schema_has_column(schema_columns, "djmdContent", "UUID") {
        "json_quote(COALESCE(UUID, ''))"
    } else {
        "json_quote('')"
    };
    let content_analysis_expr =
        if schema_has_column(schema_columns, "djmdContent", "AnalysisDataPath") {
            "json_quote(COALESCE(AnalysisDataPath, ''))"
        } else {
            "json_quote('')"
        };
    let content_select = format!(
        "SELECT 0 AS sort_key, 'content' AS row_type, json_quote(CAST(ID AS TEXT)) AS source_id, {content_uuid_expr} AS c1, {content_analysis_expr} AS c2, '' AS c3, '' AS c4, '' AS c5, '' AS c6 FROM djmdContent WHERE ID IN ({content_filter})"
    );
    let mut selects = vec![content_select];

    if schema_has_column(schema_columns, "contentFile", "ContentID")
        && schema_has_column(schema_columns, "contentFile", "ID")
        && schema_has_column(schema_columns, "contentFile", "Path")
    {
        let rb_local_expr = if schema_has_column(schema_columns, "contentFile", "rb_local_path") {
            "CASE WHEN rb_local_path IS NULL THEN '' ELSE json_quote(CAST(rb_local_path AS TEXT)) END"
        } else {
            "''"
        };
        let file_uuid_expr = if schema_has_column(schema_columns, "contentFile", "UUID") {
            "CASE WHEN UUID IS NULL THEN '' ELSE json_quote(CAST(UUID AS TEXT)) END"
        } else {
            "''"
        };
        let hash_expr = if schema_has_column(schema_columns, "contentFile", "Hash") {
            "CASE WHEN Hash IS NULL THEN '' ELSE json_quote(CAST(Hash AS TEXT)) END"
        } else {
            "''"
        };
        let size_expr = if schema_has_column(schema_columns, "contentFile", "Size") {
            "CASE WHEN Size IS NULL THEN '' ELSE json_quote(CAST(Size AS TEXT)) END"
        } else {
            "''"
        };
        selects.push(format!(
            "SELECT 1 AS sort_key, 'file' AS row_type, json_quote(CAST(ContentID AS TEXT)) AS source_id, json_quote(CAST(ID AS TEXT)) AS c1, json_quote(COALESCE(Path, '')) AS c2, {rb_local_expr} AS c3, {file_uuid_expr} AS c4, {hash_expr} AS c5, {size_expr} AS c6 FROM contentFile WHERE ContentID IN ({content_filter})"
        ));
    }

    let sql = format!(
        ".headers on\n.mode csv\n{} ORDER BY sort_key, source_id, c1;",
        selects.join("\nUNION ALL\n")
    );

    let mut builders: HashMap<String, TrackMigrationSourceDataBuilder> = HashMap::new();
    for record in sqlcipher_csv_records(db_path, key, &sql)? {
        let source_id = decode_json_string_field_required(
            record.get(2),
            "missing source content id in migration source data",
        )?;
        let builder = builders.entry(source_id).or_default();
        match record.get(1).unwrap_or_default() {
            "content" => {
                builder.old_uuid = Some(decode_json_string_field_required(
                    record.get(3),
                    "missing UUID for source content",
                )?);
                builder.old_analysis_path = Some(decode_json_string_field_required(
                    record.get(4),
                    "missing AnalysisDataPath for source content",
                )?);
            }
            "file" => {
                let id = decode_json_string_field_required(
                    record.get(3),
                    "missing contentFile ID for source content",
                )?;
                let path = decode_json_string_field_required(
                    record.get(4),
                    "missing contentFile Path for source content",
                )?;
                let rb_local_path = decode_json_string_field(record.get(5))?;
                let uuid = decode_json_string_field(record.get(6))?;
                let hash = decode_json_string_field(record.get(7))?;
                let size = decode_json_string_field(record.get(8))?
                    .and_then(|value| value.parse::<u64>().ok());
                builder.content_files.push(ContentFileRef {
                    id,
                    path,
                    rb_local_path,
                    uuid,
                    hash,
                    size,
                });
            }
            _ => {}
        }
    }

    let mut data = HashMap::new();
    for content_id in content_ids {
        let builder = builders
            .remove(*content_id)
            .ok_or_else(|| format!("missing djmdContent row for source content {}", content_id))?;
        data.insert(
            (*content_id).to_string(),
            TrackMigrationSourceData {
                old_uuid: builder
                    .old_uuid
                    .ok_or_else(|| format!("missing UUID for source content {}", content_id))?,
                old_analysis_path: builder.old_analysis_path.unwrap_or_default(),
                content_files: builder.content_files,
            },
        );
    }

    Ok(data)
}

#[cfg(target_os = "macos")]
fn clone_file_on_macos(source: &Path, destination: &Path) -> Result<bool, String> {
    use std::os::raw::{c_char, c_int};

    extern "C" {
        fn clonefile(src: *const c_char, dst: *const c_char, flags: u32) -> c_int;
    }

    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|error| format!("failed to prepare clone source path: {}", error))?;
    let destination = CString::new(destination.as_os_str().as_bytes())
        .map_err(|error| format!("failed to prepare clone destination path: {}", error))?;

    let result = unsafe { clonefile(source.as_ptr(), destination.as_ptr(), 0) };
    if result == 0 {
        Ok(true)
    } else {
        Ok(false)
    }
}

#[cfg(not(target_os = "macos"))]
fn clone_file_on_macos(_source: &Path, _destination: &Path) -> Result<bool, String> {
    Ok(false)
}

fn duplicate_file_with_parent_dirs(source: &Path, destination: &Path) -> Result<(), String> {
    if !path_exists(source)? {
        return Err(format!("source resource not found: {}", source.display()));
    }
    if source == destination {
        return Err(format!(
            "refusing to copy analysis resource onto itself: {}",
            source.display()
        ));
    }
    if let Some(parent) = destination.parent() {
        create_dir_all_path(parent)?;
    }
    if !clone_file_on_macos(source, destination)? {
        copy_path(source, destination)?;
    }
    Ok(())
}

fn encode_anlz_path(file_name: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity((file_name.len() + 3) * 2);
    for unit in format!("?/{file_name}\0").encode_utf16() {
        bytes.extend_from_slice(&unit.to_be_bytes());
    }
    bytes
}

fn rewrite_anlz_ppth(path: &Path, file_name: &str) -> Result<(), String> {
    let mut bytes = read_path(path)?;
    let Some(offset) = bytes.windows(4).position(|window| window == b"PPTH") else {
        return Ok(());
    };

    if bytes.len() < offset + 16 {
        return Err(format!("invalid ANLZ PPTH header in {}", path.display()));
    }

    let header_len = u32::from_be_bytes([
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ]) as usize;
    let chunk_len = u32::from_be_bytes([
        bytes[offset + 8],
        bytes[offset + 9],
        bytes[offset + 10],
        bytes[offset + 11],
    ]) as usize;

    if header_len < 16 || chunk_len < header_len || bytes.len() < offset + chunk_len {
        return Err(format!("invalid ANLZ PPTH chunk in {}", path.display()));
    }

    let replacement = encode_anlz_path(file_name);
    let start = offset + header_len;
    let end = offset + chunk_len;
    bytes.splice(start..end, replacement.iter().copied());

    let new_chunk_len = header_len + replacement.len();
    bytes[offset + 8..offset + 12].copy_from_slice(&(new_chunk_len as u32).to_be_bytes());
    bytes[offset + 12..offset + 16].copy_from_slice(&(replacement.len() as u32).to_be_bytes());

    if bytes.len() >= 12 && &bytes[0..4] == b"PMAI" {
        let file_len = bytes.len() as u32;
        bytes[8..12].copy_from_slice(&file_len.to_be_bytes());
    }

    write_path(path, bytes)
}

fn read_u32_be(bytes: &[u8], offset: usize) -> Option<u32> {
    bytes
        .get(offset..offset + 4)
        .map(|slice| u32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn write_u32_be(bytes: &mut [u8], offset: usize, value: u32) -> bool {
    let Some(slice) = bytes.get_mut(offset..offset + 4) else {
        return false;
    };
    slice.copy_from_slice(&value.to_be_bytes());
    true
}

fn add_ms_to_u32_be(bytes: &mut [u8], offset: usize, offset_ms: u32, skip_max: bool) -> bool {
    let Some(value) = read_u32_be(bytes, offset) else {
        return false;
    };
    if skip_max && value == u32::MAX {
        return false;
    }
    write_u32_be(bytes, offset, value.saturating_add(offset_ms))
}

fn compensate_anlz_encoder_priming(path: &Path, offset_ms: u32) -> Result<bool, String> {
    if offset_ms == 0 {
        return Ok(false);
    }

    let mut bytes = read_path(path)?;
    if bytes.len() < 12 || &bytes[0..4] != b"PMAI" {
        return Ok(false);
    }

    let mut changed = false;
    let mut offset = read_u32_be(&bytes, 4).unwrap_or(0) as usize;
    while offset + 12 <= bytes.len() {
        let tag_type = bytes[offset..offset + 4].to_vec();
        let header_len = read_u32_be(&bytes, offset + 4).unwrap_or(0) as usize;
        let tag_len = read_u32_be(&bytes, offset + 8).unwrap_or(0) as usize;
        if header_len < 12 || tag_len < header_len || offset + tag_len > bytes.len() {
            return Err(format!("invalid ANLZ tag in {}", path.display()));
        }

        match tag_type.as_slice() {
            b"PQTZ" if header_len >= 24 => {
                let entry_count = read_u32_be(&bytes, offset + 20).unwrap_or(0) as usize;
                let entries_start = offset + 24;
                for index in 0..entry_count {
                    let time_offset = entries_start + index * 8 + 4;
                    if time_offset + 4 <= offset + tag_len {
                        changed |= add_ms_to_u32_be(&mut bytes, time_offset, offset_ms, false);
                    }
                }
            }
            b"PQT2" if header_len >= 56 => {
                for index in 0..2 {
                    let time_offset = offset + 24 + index * 8 + 4;
                    if time_offset + 4 <= offset + header_len {
                        changed |= add_ms_to_u32_be(&mut bytes, time_offset, offset_ms, false);
                    }
                }
            }
            b"PCOB" if header_len >= 24 => {
                let entry_count = bytes
                    .get(offset + 18..offset + 20)
                    .map(|slice| u16::from_be_bytes([slice[0], slice[1]]) as usize)
                    .unwrap_or(0);
                let mut entry_offset = offset + 24;
                for _ in 0..entry_count {
                    if entry_offset + 40 > offset + tag_len
                        || bytes.get(entry_offset..entry_offset + 4) != Some(&b"PCPT"[..])
                    {
                        break;
                    }
                    let entry_len = read_u32_be(&bytes, entry_offset + 8).unwrap_or(0) as usize;
                    if entry_len < 40 || entry_offset + entry_len > offset + tag_len {
                        break;
                    }
                    changed |= add_ms_to_u32_be(&mut bytes, entry_offset + 32, offset_ms, false);
                    changed |= add_ms_to_u32_be(&mut bytes, entry_offset + 36, offset_ms, true);
                    entry_offset += entry_len;
                }
            }
            b"PCO2" if header_len >= 20 => {
                let entry_count = bytes
                    .get(offset + 16..offset + 18)
                    .map(|slice| u16::from_be_bytes([slice[0], slice[1]]) as usize)
                    .unwrap_or(0);
                let mut entry_offset = offset + 20;
                for _ in 0..entry_count {
                    if entry_offset + 28 > offset + tag_len
                        || bytes.get(entry_offset..entry_offset + 4) != Some(&b"PCP2"[..])
                    {
                        break;
                    }
                    let entry_len = read_u32_be(&bytes, entry_offset + 8).unwrap_or(0) as usize;
                    if entry_len < 28 || entry_offset + entry_len > offset + tag_len {
                        break;
                    }
                    changed |= add_ms_to_u32_be(&mut bytes, entry_offset + 20, offset_ms, false);
                    changed |= add_ms_to_u32_be(&mut bytes, entry_offset + 24, offset_ms, true);
                    entry_offset += entry_len;
                }
            }
            _ => {}
        }

        offset += tag_len;
    }

    if changed {
        write_path(path, bytes)?;
    }

    Ok(changed)
}

fn parse_ffprobe_skip_samples_json(text: &str) -> Option<u32> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    value
        .get("packets")?
        .as_array()?
        .iter()
        .flat_map(|packet| {
            packet
                .get("side_data_list")
                .and_then(|side| side.as_array())
        })
        .flatten()
        .find_map(|side_data| {
            side_data
                .get("skip_samples")
                .and_then(|samples| samples.as_u64())
        })
        .and_then(|samples| u32::try_from(samples).ok())
}

fn probe_skip_samples(path: &Path) -> Result<Option<u32>, String> {
    if !command_available("ffprobe") {
        return Ok(None);
    }

    let mut ffprobe = prepared_command("ffprobe")?;
    ffprobe.args([
        "-v",
        "error",
        "-select_streams",
        "a:0",
        "-show_packets",
        "-read_intervals",
        "%+0.001",
        "-show_entries",
        "packet=side_data_list",
        "-of",
        "json",
    ]);
    ffprobe.arg(path);

    let output = ffprobe.output().map_err(|e| {
        io_error_message(
            &format!("failed to run ffprobe while reading {}", path.display()),
            &e,
        )
    })?;
    if !output.status.success() {
        return Ok(None);
    }

    Ok(parse_ffprobe_skip_samples_json(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

fn samples_to_nearest_ms(samples: u32, sample_rate: u32) -> u32 {
    if sample_rate == 0 {
        return 0;
    }
    ((u64::from(samples) * 1000) + u64::from(sample_rate / 2))
        .checked_div(u64::from(sample_rate))
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(u32::MAX)
}

fn encoder_priming_compensation_ms(
    extension: &str,
    output_path: &Path,
    sample_rate: u32,
) -> Result<u32, String> {
    let Some(default_skip_samples) = (match extension {
        "m4a" => Some(2112),
        "mp3" => Some(1105),
        _ => None,
    }) else {
        return Ok(0);
    };
    let skip_samples = probe_skip_samples(output_path)?.unwrap_or(default_skip_samples);
    Ok(samples_to_nearest_ms(skip_samples, sample_rate))
}

fn has_column(columns: &[String], column: &str) -> bool {
    columns.iter().any(|candidate| candidate == column)
}

fn schema_has_table(schema: &HashMap<String, Vec<String>>, table: &str) -> bool {
    schema.contains_key(table)
}

fn schema_has_column(schema: &HashMap<String, Vec<String>>, table: &str, column: &str) -> bool {
    schema
        .get(table)
        .is_some_and(|columns| has_column(columns, column))
}

fn next_numeric_text_id(db_path: &Path, key: &str, table: &str) -> Result<String, String> {
    sqlcipher_required_value(
        db_path,
        key,
        &format!(
            "SELECT CAST(COALESCE(MAX(CAST(ID AS INTEGER)), 0) + 1 AS TEXT) FROM {table} WHERE ID <> '' AND ID NOT GLOB '*[^0-9]*';"
        ),
        &format!("expected next id for {table}"),
    )
}

fn updated_at_assignment(
    schema: &HashMap<String, Vec<String>>,
    table: &str,
    now_expr: &str,
) -> Option<String> {
    schema_has_column(schema, table, "updated_at").then(|| format!("updated_at = {now_expr}"))
}

fn update_content_id_sql(
    schema: &HashMap<String, Vec<String>>,
    table: &str,
    new_content_id_expr: &str,
    old_content_id: &str,
    now_expr: &str,
) -> Option<String> {
    if !schema_has_column(schema, table, "ContentID") {
        return None;
    }

    let mut assignments = vec![format!("ContentID = {new_content_id_expr}")];
    if let Some(updated_at) = updated_at_assignment(schema, table, now_expr) {
        assignments.push(updated_at);
    }

    Some(format!(
        "UPDATE {table} SET {} WHERE ContentID = {};\n",
        assignments.join(", "),
        sql_quote(old_content_id),
    ))
}

fn djmd_cue_migration_sql(
    columns: &[String],
    new_content_id_expr: &str,
    content_uuid: &str,
    old_content_id: &str,
    offset_ms: u32,
    now_expr: &str,
) -> String {
    if !has_column(columns, "ContentID") {
        return String::new();
    }

    let mut assignments = vec![format!("ContentID = {new_content_id_expr}")];
    if has_column(columns, "ContentUUID") {
        assignments.push(format!("ContentUUID = {}", sql_quote(content_uuid)));
    }

    if offset_ms > 0 {
        let offset_frames = ((u64::from(offset_ms) * 150) + 500) / 1000;
        let offset_microseconds = u64::from(offset_ms) * 1000;
        if has_column(columns, "InMsec") {
            assignments.push(format!(
                "InMsec = CASE WHEN InMsec >= 0 THEN InMsec + {offset_ms} ELSE InMsec END"
            ));
        }
        if has_column(columns, "OutMsec") {
            assignments.push(format!(
                "OutMsec = CASE WHEN OutMsec >= 0 THEN OutMsec + {offset_ms} ELSE OutMsec END"
            ));
        }
        if has_column(columns, "InFrame") {
            assignments.push(format!(
                "InFrame = CASE WHEN InFrame >= 0 THEN InFrame + {offset_frames} ELSE InFrame END"
            ));
        }
        if has_column(columns, "OutFrame") {
            assignments.push(format!(
                "OutFrame = CASE WHEN OutFrame > 0 THEN OutFrame + {offset_frames} ELSE OutFrame END"
            ));
        }
        if has_column(columns, "CueMicrosec") {
            assignments.push(format!(
                "CueMicrosec = CASE WHEN CueMicrosec >= 0 THEN CueMicrosec + {offset_microseconds} ELSE CueMicrosec END"
            ));
        }
    }

    if has_column(columns, "updated_at") {
        assignments.push(format!("updated_at = {now_expr}"));
    }

    format!(
        "UPDATE djmdCue SET {} WHERE ContentID = {};\n",
        assignments.join(", "),
        sql_quote(old_content_id),
    )
}

fn rewrite_content_cues_json(
    text: &str,
    old_content_id: &str,
    old_content_uuid: &str,
    new_content_id: &str,
    new_content_uuid: &str,
    offset_ms: u32,
) -> Result<(String, usize), String> {
    let mut value: serde_json::Value = serde_json::from_str(text).map_err(|error| {
        format!(
            "invalid contentCue JSON for content {}: {}",
            old_content_id, error
        )
    })?;

    let cue_count = match &value {
        serde_json::Value::Array(items) => items.len(),
        serde_json::Value::Object(_) => 1,
        _ => {
            return Err(format!(
                "contentCue JSON for content {} must be an array or object",
                old_content_id
            ))
        }
    };

    let _ = rewrite_content_cues_value(
        &mut value,
        old_content_id,
        old_content_uuid,
        new_content_id,
        new_content_uuid,
        offset_ms,
    );

    let rewritten = serde_json::to_string(&value).map_err(|error| {
        format!(
            "failed to serialize rewritten contentCue JSON for content {}: {}",
            old_content_id, error
        )
    })?;

    Ok((rewritten, cue_count))
}

fn rewrite_content_cues_value(
    value: &mut serde_json::Value,
    old_content_id: &str,
    old_content_uuid: &str,
    new_content_id: &str,
    new_content_uuid: &str,
    offset_ms: u32,
) -> usize {
    match value {
        serde_json::Value::Object(map) => {
            let mut replacements = 0usize;
            for (key, nested) in map.iter_mut() {
                match key.as_str() {
                    "ContentID" => {
                        if nested.as_str() == Some(old_content_id) {
                            *nested = serde_json::Value::String(new_content_id.to_string());
                            replacements += 1;
                        }
                    }
                    "ContentUUID" => {
                        if nested.as_str() == Some(old_content_uuid) {
                            *nested = serde_json::Value::String(new_content_uuid.to_string());
                            replacements += 1;
                        }
                    }
                    "CueMsec" | "InMsec" | "OutMsec" => {
                        replacements += add_json_u32_offset(nested, offset_ms, true);
                    }
                    "CueMicrosec" => {
                        replacements +=
                            add_json_u32_offset(nested, offset_ms.saturating_mul(1000), true);
                    }
                    "InFrame" | "OutFrame" => {
                        let offset_frames = ((u64::from(offset_ms) * 150) + 500) / 1000;
                        replacements +=
                            add_json_u64_offset(nested, offset_frames, key != "OutFrame");
                    }
                    _ => {
                        replacements += rewrite_content_cues_value(
                            nested,
                            old_content_id,
                            old_content_uuid,
                            new_content_id,
                            new_content_uuid,
                            offset_ms,
                        );
                    }
                }
            }
            replacements
        }
        serde_json::Value::Array(items) => items
            .iter_mut()
            .map(|item| {
                rewrite_content_cues_value(
                    item,
                    old_content_id,
                    old_content_uuid,
                    new_content_id,
                    new_content_uuid,
                    offset_ms,
                )
            })
            .sum(),
        _ => 0,
    }
}

fn add_json_u32_offset(value: &mut serde_json::Value, offset: u32, include_zero: bool) -> usize {
    add_json_u64_offset(value, u64::from(offset), include_zero)
}

fn add_json_u64_offset(value: &mut serde_json::Value, offset: u64, include_zero: bool) -> usize {
    if offset == 0 {
        return 0;
    }
    let Some(current) = value.as_i64() else {
        return 0;
    };
    if current < 0 || (!include_zero && current == 0) {
        return 0;
    }
    let Some(updated) = u64::try_from(current)
        .ok()
        .and_then(|current| current.checked_add(offset))
    else {
        return 0;
    };
    *value = serde_json::Value::Number(serde_json::Number::from(updated));
    1
}

fn decode_hex_text(hex: &str) -> Result<String, String> {
    if !hex.len().is_multiple_of(2) {
        return Err("hex text has an odd number of characters".to_string());
    }
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    for index in (0..hex.len()).step_by(2) {
        let byte = u8::from_str_radix(&hex[index..index + 2], 16)
            .map_err(|error| format!("invalid hex text at byte {}: {}", index / 2, error))?;
        bytes.push(byte);
    }
    String::from_utf8(bytes).map_err(|error| format!("contentCue JSON is not UTF-8: {error}"))
}

fn rewrite_content_cues_rows(
    db_path: &Path,
    key: &str,
    rewrites: &[ContentCueRewrite],
    now_expr: &str,
    has_rb_cue_count: bool,
    has_updated_at: bool,
) -> Result<(), String> {
    if rewrites.is_empty() {
        return Ok(());
    }

    let mut sql = String::from("BEGIN IMMEDIATE;\n");
    for rewrite in rewrites {
        let rows = sqlcipher_lines(
            db_path,
            key,
            &format!(
                "SELECT rowid || '|' || hex(Cues) FROM contentCue WHERE ContentID = {} AND Cues IS NOT NULL;",
                sql_quote(&rewrite.new_content_id)
            ),
        )?;

        for row in rows {
            let (rowid, cues_hex) = row
                .split_once('|')
                .ok_or_else(|| format!("unexpected contentCue row while rewriting JSON: {row}"))?;
            let cues_text = decode_hex_text(cues_hex)?;
            let (rewritten, cue_count) = rewrite_content_cues_json(
                &cues_text,
                &rewrite.old_content_id,
                &rewrite.old_content_uuid,
                &rewrite.new_content_id,
                &rewrite.new_content_uuid,
                rewrite.offset_ms,
            )?;
            let mut assignments = vec![format!("Cues = {}", sql_quote(&rewritten))];
            if has_rb_cue_count {
                assignments.push(format!("rb_cue_count = {cue_count}"));
            }
            if has_updated_at {
                assignments.push(format!("updated_at = {now_expr}"));
            }
            sql.push_str(&format!(
                "UPDATE contentCue SET {} WHERE rowid = {};\n",
                assignments.join(", "),
                rowid
            ));
        }
    }
    sql.push_str("COMMIT;\n");

    run_sqlcipher(db_path, key, &sql).map(|_| ())
}

fn md5_hex(path: &Path) -> Result<String, String> {
    let mut file = open_file_path(path)?;
    let mut context = md5::Context::new();
    let mut buffer = [0_u8; 16 * 1024];

    loop {
        let bytes_read = retry_io_operation(format!("failed to read {}", path.display()), || {
            file.read(&mut buffer)
        })?;
        if bytes_read == 0 {
            break;
        }
        context.consume(&buffer[..bytes_read]);
    }

    Ok(format!("{:x}", context.compute()))
}

fn validate_analysis_resources(
    content_files: &[ContentFileRef],
) -> Result<Vec<ValidatedContentFile>, String> {
    let mut validated = Vec::new();

    for file in content_files {
        let Some(source_path) = &file.rb_local_path else {
            continue;
        };

        let source = PathBuf::from(source_path);
        if !path_exists(&source)? {
            return Err(format!("analysis resource missing: {}", source.display()));
        }

        let metadata = metadata_path(&source)?;
        let actual_size = metadata.len();
        if actual_size == 0 {
            return Err(format!("analysis resource is empty: {}", source.display()));
        }

        if let Some(expected_size) = file.size {
            if expected_size == 0 {
                return Err(format!(
                    "analysis resource size is empty for {}",
                    source.display()
                ));
            }
        }

        if let Some(expected_hash) = &file.hash {
            if expected_hash.is_empty() || expected_hash == "d41d8cd98f00b204e9800998ecf8427e" {
                return Err(format!(
                    "analysis resource hash is empty for {}",
                    source.display()
                ));
            }
        }

        validated.push(ValidatedContentFile {
            original: ContentFileRef {
                id: file.id.clone(),
                path: file.path.clone(),
                rb_local_path: file.rb_local_path.clone(),
                uuid: file.uuid.clone(),
                hash: file.hash.clone(),
                size: file.size,
            },
            source,
        });
    }

    Ok(validated)
}

fn collect_anlz_dat_paths(base: &Path, paths: &mut Vec<PathBuf>) -> Result<(), String> {
    if !path_exists(base)? {
        return Ok(());
    }

    for entry in read_dir_path(base)? {
        let entry = entry.map_err(|e| {
            io_error_message(
                &format!("failed to read directory entry in {}", base.display()),
                &e,
            )
        })?;
        let path = entry.path();
        if metadata_path(&path)?.is_dir() {
            collect_anlz_dat_paths(&path, paths)?;
            continue;
        }

        if path.file_name().is_some_and(|name| name == "ANLZ0000.DAT") {
            paths.push(path);
        }
    }

    Ok(())
}

fn cleanup_orphan_zero_analysis_dirs(db_path: &Path, key: &str) -> Result<CleanupReport, String> {
    let rekordbox_root = db_path.parent().unwrap_or_else(|| Path::new("."));
    let analysis_root = rekordbox_root.join("share/PIONEER/USBANLZ");
    if !path_exists(&analysis_root)? {
        return Ok(CleanupReport::default());
    }

    let sql = format!(
        "SELECT DISTINCT COALESCE(rb_local_path, '') FROM contentFile WHERE rb_local_path LIKE {};",
        sql_quote(&format!("{}/%", analysis_root.to_string_lossy()))
    );
    let referenced_dirs: HashSet<PathBuf> = sqlcipher_lines(db_path, key, &sql)?
        .into_iter()
        .filter(|line| !line.is_empty())
        .filter_map(|line| Path::new(&line).parent().map(Path::to_path_buf))
        .collect();

    let mut dat_paths = Vec::new();
    collect_anlz_dat_paths(&analysis_root, &mut dat_paths)?;

    let mut orphan_dirs = Vec::new();
    for dat_path in dat_paths {
        let dir = dat_path
            .parent()
            .ok_or_else(|| {
                format!(
                    "missing analysis parent directory for {}",
                    dat_path.display()
                )
            })?
            .to_path_buf();
        if referenced_dirs.contains(&dir) {
            continue;
        }

        let ext_path = dir.join("ANLZ0000.EXT");
        let ex2_path = dir.join("ANLZ0000.2EX");
        let candidates = [dat_path.clone(), ext_path, ex2_path];
        let existing_sizes: Vec<u64> = candidates
            .iter()
            .filter_map(|path| metadata_path(path).ok().map(|meta| meta.len()))
            .collect();

        if !existing_sizes.is_empty() && existing_sizes.iter().all(|size| *size == 0) {
            orphan_dirs.push(dir);
        }
    }

    if orphan_dirs.is_empty() {
        return Ok(CleanupReport::default());
    }

    let archive_root = rekordbox_root.join(format!("anlz-orphan-cleanup-{}", timestamp_token()));
    create_dir_all_path(&archive_root)?;
    let mut archived_dirs = 0usize;
    let mut warnings = Vec::new();

    for dir in &orphan_dirs {
        let relative = match dir.strip_prefix(&analysis_root) {
            Ok(relative) => relative,
            Err(error) => {
                warnings.push(format!(
                    "Orphaned analysis folder cleanup skipped for {}: {}",
                    dir.display(),
                    error
                ));
                continue;
            }
        };
        let target = archive_root.join(relative);
        if let Some(parent) = target.parent() {
            if let Err(error) = create_dir_all_path(parent) {
                warnings.push(format!(
                    "Orphaned analysis folder cleanup skipped for {}: {}",
                    dir.display(),
                    error
                ));
                continue;
            }
        }
        match rename_path(dir, &target) {
            Ok(()) => archived_dirs += 1,
            Err(error) => warnings.push(format!(
                "Orphaned analysis folder cleanup skipped for {}: {}",
                dir.display(),
                error
            )),
        }
    }

    Ok(CleanupReport {
        archived_dirs,
        archive_dir: if archived_dirs > 0 {
            Some(archive_root.to_string_lossy().to_string())
        } else {
            None
        },
        warnings,
    })
}
