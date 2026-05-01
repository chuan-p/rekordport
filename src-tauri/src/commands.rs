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

#[cfg(any(target_os = "windows", test))]
fn windows_explorer_select_arg(path: &str) -> String {
    format!("/select,\"{path}\"")
}

#[tauri::command]
fn open_path_in_file_manager(path: String) -> Result<(), String> {
    let path = PathBuf::from(path);
    if !path.exists() {
        return Err(format!("path not found: {}", path.display()));
    }

    #[cfg(target_os = "macos")]
    {
        let mut command = Command::new("open");
        if path.is_dir() {
            command.arg(&path);
        } else {
            command.args(["-R"]).arg(&path);
        }

        let status = command.status().map_err(|e| e.to_string())?;
        if status.success() {
            return Ok(());
        }
        Err(format!(
            "failed to open path in the file manager: {}",
            path.display()
        ))
    }

    #[cfg(target_os = "windows")]
    {
        let normalized = normalized_user_path_string(&path);
        let mut command = Command::new("explorer");
        if path.is_dir() {
            command.arg(&normalized);
        } else {
            command.raw_arg(windows_explorer_select_arg(&normalized));
        }

        let status = command.status().map_err(|e| e.to_string())?;
        if status.success() {
            return Ok(());
        }
        return Err(format!(
            "failed to open path in the file manager: {}",
            path.display()
        ));
    }

    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        let target = if path.is_dir() {
            path
        } else {
            path.parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| path.to_path_buf())
        };

        let status = Command::new("xdg-open")
            .arg(&target)
            .status()
            .map_err(|e| e.to_string())?;

        if status.success() {
            return Ok(());
        }
        return Err(format!("failed to open path: {}", target.display()));
    }
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
