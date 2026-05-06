#[tauri::command]
fn pick_database_path() -> Option<String> {
    rfd::FileDialog::new()
        .set_title("Choose rekordbox master.db")
        .add_filter("rekordbox database", &["db"])
        .set_file_name("master.db")
        .pick_file()
        .map(|path| path.to_string_lossy().to_string())
}

#[tauri::command]
fn prepare_preview_path(path: String) -> Result<String, String> {
    prepare_preview_path_impl(path)
}

fn require_existing_folder(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err(format!("folder does not exist: {}", path.display()));
    }
    if !path.is_dir() {
        return Err(format!("path is not a folder: {}", path.display()));
    }
    Ok(())
}

fn containing_folder_target(path: &Path) -> Result<PathBuf, String> {
    if !path.exists() {
        return Err(format!("path does not exist: {}", path.display()));
    }
    if path.is_dir() {
        return Ok(path.to_path_buf());
    }
    if !path.is_file() {
        return Err(format!("path is neither a file nor a folder: {}", path.display()));
    }

    let parent = path
        .parent()
        .ok_or_else(|| format!("file has no containing folder: {}", path.display()))?;
    require_existing_folder(parent).map_err(|error| {
        format!(
            "containing folder is unavailable for {}: {error}",
            path.display()
        )
    })?;
    Ok(parent.to_path_buf())
}

fn open_folder_in_file_manager(folder: &Path) -> Result<(), String> {
    require_existing_folder(folder)?;
    #[cfg(target_os = "macos")]
    {
        let status = Command::new("open")
            .arg(folder)
            .status()
            .map_err(|e| format!("failed to launch file manager for {}: {e}", folder.display()))?;

        if status.success() {
            return Ok(());
        }
        Err(format!(
            "file manager exited unsuccessfully while opening folder: {}",
            folder.display()
        ))
    }

    #[cfg(target_os = "windows")]
    {
        let normalized = normalized_user_path_string(folder);

        let mut command = Command::new("explorer");
        command.arg(normalized);
        command.spawn().map_err(|e| {
            format!(
                "failed to launch file manager for folder {}: {e}",
                folder.display()
            )
        })?;
        Ok(())
    }

    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        let status = Command::new("xdg-open")
            .arg(folder)
            .status()
            .map_err(|e| format!("failed to launch file manager for {}: {e}", folder.display()))?;

        if status.success() {
            return Ok(());
        }
        return Err(format!(
            "file manager exited unsuccessfully while opening folder: {}",
            folder.display()
        ));
    }
}

#[tauri::command]
fn open_folder(path: String) -> Result<(), String> {
    let folder = PathBuf::from(path);
    open_folder_in_file_manager(&folder)
}

#[tauri::command]
fn open_containing_folder(path: String) -> Result<(), String> {
    let path = PathBuf::from(path);
    let folder = containing_folder_target(&path)?;
    open_folder_in_file_manager(&folder)
}

#[tauri::command]
fn open_path_in_file_manager(path: String) -> Result<(), String> {
    open_containing_folder(path)
}

#[tauri::command]
fn open_external_url(url: String) -> Result<(), String> {
    let trimmed = url.trim();
    if !(trimmed.starts_with("https://") || trimmed.starts_with("http://")) {
        return Err(format!("unsupported url: {trimmed}"));
    }

    #[cfg(target_os = "macos")]
    {
        let status = Command::new("open")
            .arg(trimmed)
            .status()
            .map_err(|e| e.to_string())?;

        if status.success() {
            return Ok(());
        }
        Err(format!("failed to open url: {trimmed}"))
    }

    #[cfg(target_os = "windows")]
    {
        let status = Command::new("explorer")
            .arg(trimmed)
            .status()
            .map_err(|e| e.to_string())?;

        if status.success() {
            return Ok(());
        }
        return Err(format!("failed to open url: {trimmed}"));
    }

    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        let status = Command::new("xdg-open")
            .arg(trimmed)
            .status()
            .map_err(|e| e.to_string())?;

        if status.success() {
            return Ok(());
        }
        return Err(format!("failed to open url: {trimmed}"));
    }
}

fn latest_release_impl() -> Result<LatestReleaseResponse, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(UPDATE_CHECK_TIMEOUT)
        .timeout_read(UPDATE_CHECK_TIMEOUT)
        .build();
    let response = agent
        .get(LATEST_RELEASE_URL)
        .set(
            "User-Agent",
            concat!("rekordport/", env!("CARGO_PKG_VERSION")),
        )
        .call()
        .map_err(|error| format!("failed to check GitHub releases: {error}"))?;

    let html_url = response.get_url().to_string();
    let tag_name = html_url
        .rsplit_once("/releases/tag/")
        .map(|(_, tag)| tag)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("GitHub latest release did not redirect to a tag: {html_url}"))?;
    let changelog = fetch_release_changelog(tag_name);

    Ok(LatestReleaseResponse {
        tag_name: tag_name.to_string(),
        html_url,
        changelog,
    })
}

fn fetch_release_changelog(tag_name: &str) -> Option<String> {
    let url =
        format!("https://raw.githubusercontent.com/chuan-p/rekordport/{tag_name}/CHANGELOG.md");
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(UPDATE_CHECK_TIMEOUT)
        .timeout_read(UPDATE_CHECK_TIMEOUT)
        .build();
    agent
        .get(&url)
        .set(
            "User-Agent",
            concat!("rekordport/", env!("CARGO_PKG_VERSION")),
        )
        .call()
        .ok()?
        .into_string()
        .ok()
        .filter(|text| !text.trim().is_empty())
}

#[tauri::command]
async fn latest_release() -> Result<LatestReleaseResponse, String> {
    tauri::async_runtime::spawn_blocking(latest_release_impl)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn scan_library(app: tauri::AppHandle, req: ScanRequest) -> Result<ScanResponse, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let operation_id = req.operation_id.clone();
        scan_impl_with_progress(req, |payload| {
            let _ = app.emit(
                "scan-progress",
                ProgressEventPayload::new(operation_id.clone(), payload),
            );
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn convert_tracks(
    app: tauri::AppHandle,
    req: ConvertRequest,
) -> Result<ConvertResponse, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let operation_id = req.operation_id.clone();
        let conversion_lock = CONVERSION_LOCK.get_or_init(|| Mutex::new(()));
        let _conversion_guard = conversion_lock
            .try_lock()
            .map_err(|_| "another conversion is already running".to_string())?;
        let _database_lock = acquire_database_conversion_lock(Path::new(&req.db_path))?;
        if process::rekordbox_process_running()? {
            return Err(
                "rekordbox appears to be running. Close rekordbox before converting, then try again. No files or database rows were changed."
                    .to_string(),
            );
        }

        let result = convert_impl_with_progress(req, |payload| {
            let _ = app.emit(
                "convert-progress",
                ProgressEventPayload::new(operation_id.clone(), payload),
            );
        });

        if let Err(error) = &result {
            let _ = app.emit(
                "convert-progress",
                ProgressEventPayload::new(
                    operation_id.clone(),
                    ScanProgressPayload {
                        phase: "error".to_string(),
                        current: 0,
                        total: 0,
                        message: error.clone(),
                    },
                ),
            );
        }

        result
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn preflight_check(req: PreflightRequest) -> Result<PreflightResponse, String> {
    tauri::async_runtime::spawn_blocking(move || {
        Ok::<PreflightResponse, String>(preflight_impl(req))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn rekordbox_process_running() -> Result<bool, String> {
    process::rekordbox_process_running()
}
