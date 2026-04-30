fn command_exists(command: &str) -> bool {
    Command::new(command).arg("--version").output().is_ok()
}

fn command_exists_at(path: &Path) -> bool {
    Command::new(path).arg("--version").output().is_ok()
}

fn target_triple() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "aarch64-apple-darwin"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "x86_64-apple-darwin"
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "x86_64-pc-windows-msvc"
    }
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    {
        "aarch64-pc-windows-msvc"
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "x86_64-unknown-linux-gnu"
    }
}

fn tool_override_var(command: &str) -> Option<&'static str> {
    match command {
        "sqlcipher" => Some("RKB_SQLCIPHER_PATH"),
        "ffmpeg" => Some("RKB_FFMPEG_PATH"),
        "ffprobe" => Some("RKB_FFPROBE_PATH"),
        _ => None,
    }
}

fn sidecar_filename(command: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("{command}-{}.exe", target_triple())
    } else {
        format!("{command}-{}", target_triple())
    }
}

fn bundled_command_filenames(command: &str) -> Vec<String> {
    let mut names = vec![sidecar_filename(command)];
    if !names.iter().any(|name| name == command) {
        names.push(command.to_string());
    }
    names
}

include!(concat!(env!("OUT_DIR"), "/embedded_windows_sidecars.rs"));

fn embedded_windows_sidecar_path(command: &str) -> Option<PathBuf> {
    let bytes = embedded_windows_sidecar_bytes(command)?;
    let digest = format!("{:x}", md5::compute(bytes));
    let path = embedded_windows_sidecar_root()
        .join(target_triple())
        .join(format!("{command}-{digest}.exe"));

    let needs_write = match fs::metadata(&path) {
        Ok(meta) => meta.len() != bytes.len() as u64,
        Err(_) => true,
    };
    if needs_write {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).ok()?;
        }
        fs::write(&path, bytes).ok()?;
    }

    Some(path)
}

fn embedded_windows_sidecar_root() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        return runtime_support_root("sidecars");
    }

    #[cfg(not(target_os = "windows"))]
    {
        env::temp_dir().join("rekordport-sidecars")
    }
}

fn candidate_search_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut seen = HashSet::new();

    let mut push_root = |path: PathBuf| {
        if seen.insert(path.clone()) {
            roots.push(path);
        }
    };

    if let Ok(exe_path) = env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            push_root(exe_dir.to_path_buf());
            push_root(exe_dir.join("bin"));
            if let Some(contents_dir) = exe_dir.parent() {
                push_root(contents_dir.to_path_buf());
                push_root(contents_dir.join("Resources"));
                push_root(contents_dir.join("Resources").join("bin"));
                push_root(contents_dir.join("Resources").join("sidecars"));
                if let Some(app_dir) = contents_dir.parent() {
                    push_root(app_dir.join("Resources"));
                    push_root(app_dir.join("Resources").join("bin"));
                    push_root(app_dir.join("Resources").join("sidecars"));
                }
            }
        }
    }

    if let Ok(cwd) = env::current_dir() {
        push_root(cwd.join("src-tauri").join("bin"));
    }

    roots
}

fn resolve_command(command: &str) -> Option<PathBuf> {
    let cache = COMMAND_PATH_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    {
        let guard = cache.lock().expect("command path cache lock poisoned");
        if let Some(value) = guard.get(command) {
            return value.clone();
        }
    }

    let resolved = (|| -> Option<PathBuf> {
        if let Some(env_name) = tool_override_var(command) {
            if let Some(value) = env::var_os(env_name) {
                let candidate = PathBuf::from(value);
                if candidate.exists() && command_exists_at(&candidate) {
                    return Some(candidate);
                }
                return None;
            }
        }

        let sidecars = bundled_command_filenames(command);
        for root in candidate_search_roots() {
            for sidecar in &sidecars {
                let candidate = root.join(sidecar);
                if candidate.exists() && command_exists_at(&candidate) {
                    return Some(candidate);
                }
            }
        }

        if let Some(candidate) = embedded_windows_sidecar_path(command) {
            if command_exists_at(&candidate) {
                return Some(candidate);
            }
        }

        if command_exists(command) {
            return Some(PathBuf::from(command));
        }

        None
    })();

    let mut guard = cache.lock().expect("command path cache lock poisoned");
    guard.insert(command.to_string(), resolved.clone());
    resolved
}

fn invalid_tool_override_message(command: &str) -> Option<String> {
    let env_name = tool_override_var(command)?;
    let value = env::var_os(env_name)?;
    let candidate = PathBuf::from(value);
    if candidate.exists() && command_exists_at(&candidate) {
        return None;
    }
    Some(format!(
        "{env_name} is set to {}, but that path is not a runnable {command} executable",
        candidate.display()
    ))
}

fn is_bundled_command_path(path: &Path) -> bool {
    candidate_search_roots()
        .into_iter()
        .any(|root| path.starts_with(root))
}

fn command_source(command: &str) -> Option<String> {
    let resolved = resolve_command(command)?;
    if let Some(env_name) = tool_override_var(command) {
        if let Some(value) = env::var_os(env_name) {
            let candidate = PathBuf::from(value);
            if candidate.exists() && command_exists_at(&candidate) && candidate == resolved {
                return Some(format!(
                    "environment override {} ({})",
                    env_name,
                    resolved.display()
                ));
            }
        }
    }

    if is_bundled_command_path(&resolved) {
        Some(format!("bundled sidecar ({})", resolved.display()))
    } else if resolved.starts_with(embedded_windows_sidecar_root()) {
        Some(format!("embedded sidecar ({})", resolved.display()))
    } else if resolved.components().count() == 1 {
        Some("system PATH".to_string())
    } else {
        Some(format!("custom path ({})", resolved.display()))
    }
}

fn prepared_command(command: &str) -> Result<Command, String> {
    let resolved = resolve_command(command).ok_or_else(|| {
        invalid_tool_override_message(command)
            .unwrap_or_else(|| format!("{command} command not found in PATH or bundled sidecar"))
    })?;
    Ok(Command::new(resolved))
}

fn file_type_name(file_type: i32, codec_name: Option<&str>) -> String {
    if codec_name == Some("alac") {
        return "ALAC".to_string();
    }

    match file_type {
        4 => "M4A",
        6 => "ALAC",
        5 => "FLAC",
        11 => "WAV",
        12 => "AIFF",
        _ => "Unknown",
    }
    .to_string()
}

#[tauri::command]
fn default_database_path() -> Option<String> {
    default_database_path_value()
}

fn sql_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn timestamp_token() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let sequence = TIMESTAMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{millis}-{sequence}")
}

fn playlist_timestamp_label() -> String {
    chrono::Local::now().format("%m-%d %H:%M").to_string()
}

fn conflict_resolution_mode(value: Option<&str>) -> Result<ConflictResolution, String> {
    match value
        .unwrap_or("error")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "" | "error" => Ok(ConflictResolution::Error),
        "overwrite" => Ok(ConflictResolution::Overwrite),
        "redirect" => Ok(ConflictResolution::Redirect),
        other => Err(format!("unsupported conflict resolution mode: {other}")),
    }
}

fn unique_redirect_path(path: &Path) -> Result<PathBuf, String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("missing parent directory for {}", path.display()))?;
    let stem = path
        .file_stem()
        .ok_or_else(|| format!("missing file stem for {}", path.display()))?
        .to_string_lossy()
        .to_string();
    let extension = path
        .extension()
        .map(|value| format!(".{}", value.to_string_lossy()))
        .unwrap_or_default();

    for index in 2..10_000 {
        let candidate = parent.join(format!("{stem} ({index}){extension}"));
        if !path_exists(&candidate)? {
            return Ok(candidate);
        }
    }

    Err(format!(
        "could not find an available redirected file name for {}",
        path.display()
    ))
}

fn backup_relative_path(path: &Path) -> PathBuf {
    let mut rel = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => {
                #[cfg(target_os = "windows")]
                {
                    match prefix.kind() {
                        std::path::Prefix::Disk(letter)
                        | std::path::Prefix::VerbatimDisk(letter) => {
                            rel.push(format!("drive-{}", char::from(letter)));
                        }
                        std::path::Prefix::UNC(server, share)
                        | std::path::Prefix::VerbatimUNC(server, share) => {
                            rel.push("unc");
                            rel.push(server);
                            rel.push(share);
                        }
                        _ => rel.push(prefix.as_os_str()),
                    }
                }

                #[cfg(not(target_os = "windows"))]
                {
                    rel.push(prefix.as_os_str());
                }
            }
            Component::RootDir => {}
            Component::CurDir => rel.push("."),
            Component::ParentDir => rel.push(".."),
            Component::Normal(part) => rel.push(part),
        }
    }
    rel
}

fn command_available(command: &str) -> bool {
    let cache = COMMAND_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = cache.lock().expect("command cache lock poisoned");
    if let Some(value) = guard.get(command) {
        return *value;
    }
    let available = resolve_command(command).is_some();
    guard.insert(command.to_string(), available);
    available
}

fn ffmpeg_has_encoder(name: &str) -> Result<bool, String> {
    if !command_available("ffmpeg") {
        return Ok(false);
    }

    let cache = ENCODER_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    {
        let guard = cache.lock().expect("encoder cache lock poisoned");
        if let Some(value) = guard.get(name) {
            return Ok(*value);
        }
    }

    let mut ffmpeg = prepared_command("ffmpeg")?;
    ffmpeg.args(["-hide_banner", "-encoders"]);
    let output = ffmpeg
        .output()
        .map_err(|e| io_error_message("failed to run ffmpeg -encoders", &e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let available = stdout.lines().any(|line| line.contains(name));
    let mut guard = cache.lock().expect("encoder cache lock poisoned");
    guard.insert(name.to_string(), available);
    Ok(available)
}
