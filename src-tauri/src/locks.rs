fn platform_name() -> String {
    if cfg!(target_os = "macos") {
        "macOS".to_string()
    } else if cfg!(target_os = "windows") {
        "Windows".to_string()
    } else if cfg!(target_os = "linux") {
        "Linux".to_string()
    } else {
        std::env::consts::OS.to_string()
    }
}

fn default_database_path_value() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        return std::env::var_os("HOME").map(|home| {
            PathBuf::from(home)
                .join("Library/Pioneer/rekordbox/master.db")
                .to_string_lossy()
                .to_string()
        });
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(app_data) = std::env::var_os("APPDATA") {
            return Some(
                PathBuf::from(app_data)
                    .join("Pioneer/rekordbox/master.db")
                    .to_string_lossy()
                    .to_string(),
            );
        }

        if let Some(home) = std::env::var_os("USERPROFILE") {
            return Some(
                PathBuf::from(home)
                    .join("AppData/Roaming/Pioneer/rekordbox/master.db")
                    .to_string_lossy()
                    .to_string(),
            );
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        return None;
    }

    #[allow(unreachable_code)]
    None
}

fn check_database_readable(db_path: &Path, key: &str) -> bool {
    if !db_path.exists() || !command_available("sqlcipher") {
        return false;
    }

    run_sqlcipher(db_path, key, "SELECT COUNT(*) FROM djmdContent LIMIT 1;").is_ok()
}

fn check_sqlcipher_json_available(db_path: &Path, key: &str) -> bool {
    if !db_path.exists() || !command_available("sqlcipher") {
        return false;
    }

    run_sqlcipher(db_path, key, "SELECT json_quote('x'), json_type('[]');").is_ok()
}

const ROLLBACK_FAILED_MARKER: &str = "Rollback also failed:";

fn append_rollback_errors(error: String, rollback_errors: Vec<String>) -> String {
    if rollback_errors.is_empty() {
        return error;
    }

    format!(
        "{error}. {ROLLBACK_FAILED_MARKER} {}",
        rollback_errors.join(" | ")
    )
}

fn error_contains_rollback_failure(error: &str) -> bool {
    error.contains(ROLLBACK_FAILED_MARKER)
}

fn rollback_current_conversion(
    temp_output_path: &Path,
    archive_path: &Path,
    source: &Path,
) -> Vec<String> {
    let mut errors = Vec::new();
    if path_exists(temp_output_path).unwrap_or(false) {
        if let Err(error) = remove_file_path(temp_output_path) {
            errors.push(format!(
                "failed to remove temporary output {}: {}",
                temp_output_path.display(),
                error
            ));
        }
    }

    if path_exists(archive_path).unwrap_or(false) {
        if let Err(error) = rename_path(archive_path, source) {
            errors.push(format!(
                "failed to restore archived source {} -> {}: {}",
                archive_path.display(),
                source.display(),
                error
            ));
        }
    } else {
        errors.push(format!(
            "missing archived source {} while rolling back current track {}",
            archive_path.display(),
            source.display()
        ));
    }

    errors
}

fn restore_database_backup(db_backup: &Path, db_path: &Path) -> Vec<String> {
    match copy_path(db_backup, db_path) {
        Ok(_) => Vec::new(),
        Err(error) => vec![format!(
            "failed to restore database backup {} -> {}: {}",
            db_backup.display(),
            db_path.display(),
            error
        )],
    }
}

fn parse_lock_pid(text: &str) -> Option<u32> {
    text.split_whitespace()
        .find_map(|part| part.strip_prefix("pid="))
        .and_then(|value| value.parse::<u32>().ok())
}

fn process_id_running(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }

    #[cfg(target_os = "windows")]
    {
        let output = hidden_windows_command("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output();
        return output
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).contains(&pid.to_string()))
            .unwrap_or(true);
    }

    #[cfg(not(target_os = "windows"))]
    {
        let status = Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stderr(Stdio::null())
            .status();
        return status.map(|status| status.success()).unwrap_or(true);
    }
}

fn remove_stale_database_conversion_lock(lock_path: &Path) -> Result<bool, String> {
    let Ok(text) = fs::read_to_string(lock_path) else {
        return Ok(false);
    };
    let Some(pid) = parse_lock_pid(&text) else {
        return Ok(false);
    };
    if process_id_running(pid) {
        return Ok(false);
    }
    remove_file_path(lock_path)?;
    Ok(true)
}

fn acquire_database_conversion_lock(db_path: &Path) -> Result<DatabaseConversionLock, String> {
    let parent = db_path
        .parent()
        .ok_or_else(|| format!("missing parent directory for {}", db_path.display()))?;
    let lock_path = parent.join(".rekordport-conversion.lock");
    let mut lock_file = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            if remove_stale_database_conversion_lock(&lock_path)? {
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&lock_path)
                    .map_err(|error| {
                        io_error_message(
                            &format!("failed to create conversion lock {}", lock_path.display()),
                            &error,
                        )
                    })?
            } else {
                let owner = fs::read_to_string(&lock_path)
                    .ok()
                    .map(|text| text.trim().to_string())
                    .filter(|text| !text.is_empty())
                    .unwrap_or_else(|| "another rekordport process".to_string());
                return Err(format!(
                    "another conversion or recovery appears to be running for this library ({owner}). If rekordport previously crashed, close other instances and remove {} only after confirming no conversion is active.",
                    lock_path.display()
                ));
            }
        }
        Err(error) => {
            return Err(io_error_message(
                &format!("failed to create conversion lock {}", lock_path.display()),
                &error,
            ));
        }
    };

    writeln!(
        lock_file,
        "pid={} db={}",
        std::process::id(),
        db_path.display()
    )
    .map_err(|error| {
        let _ = fs::remove_file(&lock_path);
        io_error_message(
            &format!("failed to write conversion lock {}", lock_path.display()),
            &error,
        )
    })?;

    Ok(DatabaseConversionLock { path: lock_path })
}

fn preflight_impl(req: PreflightRequest) -> PreflightResponse {
    refresh_command_discovery_caches();
    let db_path = req
        .db_path
        .filter(|value| !value.trim().is_empty())
        .or_else(default_database_path_value)
        .unwrap_or_default();
    let db_path_buf = PathBuf::from(&db_path);
    let sqlcipher_available = command_available("sqlcipher");
    let ffmpeg_available = command_available("ffmpeg");
    let sqlcipher_source = command_source("sqlcipher");
    let ffmpeg_source = command_source("ffmpeg");
    let m4a_encoder_available = ffmpeg_has_encoder("aac_at").unwrap_or(false);
    let png_encoder_available = ffmpeg_has_encoder("png").unwrap_or(false);
    let db_exists = !db_path.is_empty() && db_path_buf.exists();
    let mut db_readable = if db_exists {
        check_database_readable(&db_path_buf, DEFAULT_KEY)
    } else {
        false
    };
    let mut json_available =
        db_readable && check_sqlcipher_json_available(&db_path_buf, DEFAULT_KEY);

    let mut warnings = Vec::new();
    let mut preflight_database_lock = None;
    if db_exists {
        match acquire_database_conversion_lock(&db_path_buf) {
            Ok(lock) => preflight_database_lock = Some(lock),
            Err(error) => warnings.push(format!(
                "skipped interrupted-conversion recovery while checking this library: {error}"
            )),
        }
    }
    if db_exists && preflight_database_lock.is_some() {
        if let Some(backup_parent) = db_path_buf.parent() {
            match conversion_session::recover_stale_conversion_backups(backup_parent, &db_path_buf) {
                Ok(report) => {
                    warnings.extend(report.warnings);
                    warnings.extend(report.errors);
                    db_readable = check_database_readable(&db_path_buf, DEFAULT_KEY);
                    json_available =
                        db_readable && check_sqlcipher_json_available(&db_path_buf, DEFAULT_KEY);
                }
                Err(error) => warnings.push(format!(
                    "failed to recover interrupted conversion backups while checking this library: {}",
                    error
                )),
            }
            match conversion_session::cleanup_completed_conversion_backups(backup_parent) {
                Ok(report) => warnings.extend(report.warnings),
                Err(error) => warnings.push(format!(
                    "failed to clean completed conversion backups while checking this library: {}",
                    error
                )),
            }
            match conversion_session::cleanup_successful_music_backups(backup_parent) {
                Ok(report) => warnings.extend(report.warnings),
                Err(error) => warnings.push(format!(
                    "failed to clean successful music backups while checking this library: {}",
                    error
                )),
            }
            match conversion_session::cleanup_successful_database_backups(backup_parent, 1) {
                Ok(report) => warnings.extend(report.warnings),
                Err(error) => warnings.push(format!(
                    "failed to clean old database backups while checking this library: {}",
                    error
                )),
            }
        }
    }
    if !sqlcipher_available {
        warnings.push("sqlcipher was not found, so rekordbox master.db cannot be read. Add a bundled sidecar in src-tauri/bin or install it in the system PATH.".to_string());
    }
    if !ffmpeg_available {
        warnings.push("ffmpeg was not found, so format conversion is unavailable. Add a bundled sidecar in src-tauri/bin or install it in the system PATH.".to_string());
    }
    if !db_path.is_empty() && !db_exists {
        warnings.push(format!("Database path does not exist: {db_path}"));
    } else if db_exists && !db_readable {
        warnings.push(
            "master.db was found, but the current environment cannot read it correctly."
                .to_string(),
        );
    }
    if cfg!(target_os = "windows") && !m4a_encoder_available {
        warnings.push("The current ffmpeg build does not include Apple's aac_at encoder, so M4A 320kbps is usually unavailable on Windows.".to_string());
    }
    if ffmpeg_available && !png_encoder_available {
        warnings.push("The current ffmpeg build does not include the PNG encoder, so embedded cover art will be skipped during conversion.".to_string());
    }
    if db_readable && !json_available {
        warnings.push("The current sqlcipher build does not include SQLite JSON functions required for cue migration during conversion.".to_string());
    }

    let scan_ready = sqlcipher_available && db_readable;
    let convert_ready = ffmpeg_available && sqlcipher_available && db_readable && json_available;

    PreflightResponse {
        os: platform_name(),
        sqlcipher_available,
        ffmpeg_available,
        sqlcipher_source,
        ffmpeg_source,
        m4a_encoder_available,
        png_encoder_available,
        db_path,
        db_exists,
        db_readable,
        scan_ready,
        convert_ready,
        warnings,
    }
}
