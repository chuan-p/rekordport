fn preview_cache_root() -> Result<PathBuf, String> {
    let mut cache_root = std::env::temp_dir();
    cache_root.push("rekordport-preview-cache");
    create_dir_all_path(&cache_root)?;
    Ok(cache_root)
}

fn cleanup_preview_cache() -> Result<(), String> {
    let cache_root = preview_cache_root()?;
    let now = SystemTime::now();
    let mut entries = Vec::new();
    let mut total_bytes = 0u64;

    for entry in read_dir_path(&cache_root)? {
        let entry = entry.map_err(|error| {
            io_error_message(
                &format!(
                    "failed to read preview cache entry in {}",
                    cache_root.display()
                ),
                &error,
            )
        })?;
        let path = entry.path();
        let meta = match metadata_path(&path) {
            Ok(meta) if meta.is_file() => meta,
            _ => continue,
        };
        let size = meta.len();
        let modified = meta.modified().unwrap_or(UNIX_EPOCH);
        if now
            .duration_since(modified)
            .map(|age| age > PREVIEW_CACHE_MAX_AGE)
            .unwrap_or(false)
        {
            let _ = remove_file_path(&path);
            continue;
        }
        total_bytes = total_bytes.saturating_add(size);
        entries.push((modified, size, path));
    }

    entries.sort_by_key(|(modified, _, _)| *modified);
    for (_, size, path) in entries {
        if total_bytes <= PREVIEW_CACHE_MAX_BYTES {
            break;
        }
        if remove_file_path(&path).is_ok() {
            total_bytes = total_bytes.saturating_sub(size);
        }
    }

    Ok(())
}

fn preview_cache_token(source: &Path, suffix: &str) -> Result<String, String> {
    let meta = metadata_path(source)?;
    let modified = meta
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_secs())
        .unwrap_or_default();
    Ok(format!(
        "{:x}",
        md5::compute(format!(
            "{}::{suffix}::{}::{}",
            source.to_string_lossy(),
            meta.len(),
            modified
        ))
    ))
}

fn preview_cache_path_for(source: &Path) -> Result<PathBuf, String> {
    let extension = source
        .extension()
        .map(|value| format!(".{}", value.to_string_lossy()))
        .unwrap_or_default();
    let cache_root = preview_cache_root()?;
    let key = preview_cache_token(source, "original")?;
    Ok(cache_root.join(format!("{key}{extension}")))
}

fn preview_transcode_path_for(source: &Path, extension: &str) -> Result<PathBuf, String> {
    let cache_root = preview_cache_root()?;
    let key = preview_cache_token(source, &format!("transcoded-{extension}"))?;
    Ok(cache_root.join(format!("{key}.{extension}")))
}

#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowsPreviewStrategy {
    CopyOriginal,
    TranscodeMp3,
}

#[cfg(any(target_os = "windows", test))]
fn windows_preview_strategy(source: &Path) -> WindowsPreviewStrategy {
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase());

    if matches!(extension.as_deref(), Some("mp3" | "m4a" | "aac")) {
        WindowsPreviewStrategy::CopyOriginal
    } else {
        WindowsPreviewStrategy::TranscodeMp3
    }
}

#[cfg(any(not(target_os = "windows"), test))]
fn preview_requires_transcode(source: &Path) -> bool {
    #[cfg(any(target_os = "windows", test))]
    {
        matches!(
            windows_preview_strategy(source),
            WindowsPreviewStrategy::TranscodeMp3
        )
    }

    #[cfg(all(not(target_os = "windows"), not(test)))]
    {
        let _ = source;
        false
    }
}

fn ensure_preview_cached_copy(source: &Path) -> Result<PathBuf, String> {
    let cached = preview_cache_path_for(source)?;
    if path_exists(&cached)? {
        return Ok(cached);
    }

    copy_path(source, &cached).map_err(|e| {
        format!(
            "failed to cache preview file locally ({} -> {}): {}",
            source.display(),
            cached.display(),
            e
        )
    })?;

    Ok(cached)
}

fn ensure_preview_transcode(source: &Path) -> Result<PathBuf, String> {
    let cached = preview_transcode_path_for(source, "mp3")?;
    if path_exists(&cached)? {
        return Ok(cached);
    }

    if !command_available("ffmpeg") {
        return Err(format!(
            "ffmpeg is required to preview this file format on Windows: {}",
            source.display()
        ));
    }

    let mut ffmpeg = prepared_command("ffmpeg")?;
    ffmpeg.args(["-hide_banner", "-loglevel", "error", "-y", "-i"]);
    ffmpeg.arg(source);
    ffmpeg.args([
        "-vn",
        "-ac",
        "2",
        "-ar",
        "44100",
        "-c:a",
        "libmp3lame",
        "-b:a",
        "192k",
    ]);
    ffmpeg.arg(&cached);

    let output = ffmpeg.output().map_err(|e| {
        io_error_message(
            &format!(
                "failed to run ffmpeg while preparing preview for {}",
                source.display()
            ),
            &e,
        )
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if !stderr.is_empty() { stderr } else { stdout };
        return Err(format!(
            "ffmpeg failed while preparing preview for {}: {}",
            source.display(),
            detail
        ));
    }

    Ok(cached)
}

#[cfg(any(target_os = "windows", test))]
fn normalize_windows_path_string(value: &str) -> String {
    let normalized = value.replace('/', "\\");
    if let Some(stripped) = normalized.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{}", stripped);
    }
    if let Some(stripped) = normalized.strip_prefix(r"\\?\") {
        return stripped.to_string();
    }
    normalized
}

fn normalized_user_path_string(path: &Path) -> String {
    #[cfg(target_os = "windows")]
    {
        let resolved = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        normalize_windows_path_string(&resolved.to_string_lossy())
    }

    #[cfg(not(target_os = "windows"))]
    {
        path.to_string_lossy().to_string()
    }
}

fn preview_path_string(path: &Path) -> String {
    let normalized = normalized_user_path_string(path);

    #[cfg(target_os = "windows")]
    {
        normalized.replace('\\', "/")
    }

    #[cfg(not(target_os = "windows"))]
    {
        normalized
    }
}

fn prepare_preview_path_impl(path: String) -> Result<String, String> {
    refresh_command_discovery_caches();
    let _ = cleanup_preview_cache();
    let source = PathBuf::from(&path);
    if !path_exists(&source)? {
        return Err(format!("path not found: {}", source.display()));
    }
    if !metadata_path(&source)?.is_file() {
        return Err(format!("preview path is not a file: {}", source.display()));
    }

    #[cfg(target_os = "windows")]
    {
        let prepared = match windows_preview_strategy(&source) {
            WindowsPreviewStrategy::CopyOriginal => ensure_preview_cached_copy(&source)?,
            WindowsPreviewStrategy::TranscodeMp3 => ensure_preview_transcode(&source)?,
        };
        return Ok(preview_path_string(&prepared));
    }

    #[cfg(not(target_os = "windows"))]
    if preview_requires_transcode(&source) {
        let transcoded = ensure_preview_transcode(&source)?;
        return Ok(preview_path_string(&transcoded));
    }

    let cached = ensure_preview_cached_copy(&source)?;
    Ok(preview_path_string(&cached))
}
