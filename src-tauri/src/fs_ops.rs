fn is_windows_lock_error(error: &io::Error) -> bool {
    #[cfg(target_os = "windows")]
    {
        matches!(error.raw_os_error(), Some(32) | Some(33))
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = error;
        false
    }
}

fn io_error_detail(error: &io::Error) -> String {
    match error.raw_os_error() {
        Some(code) => format!("{error} (kind: {:?}, os error: {code})", error.kind()),
        None => format!("{error} (kind: {:?})", error.kind()),
    }
}

fn io_error_message(action: &str, error: &io::Error) -> String {
    let mut message = format!("{action}: {}", io_error_detail(error));
    if is_windows_lock_error(error) {
        message.push_str(
            ". Windows reports that the file is locked by another process. Close Rekordbox, Explorer preview panes, audio players, or any app previewing this file, then try again.",
        );
    }
    message
}

fn retry_io_operation<T, F>(action: impl Into<String>, mut operation: F) -> Result<T, String>
where
    F: FnMut() -> io::Result<T>,
{
    let action = action.into();
    let attempts = if cfg!(target_os = "windows") { 24 } else { 1 };

    for attempt in 0..attempts {
        match operation() {
            Ok(value) => return Ok(value),
            Err(error) => {
                if attempt + 1 < attempts && is_windows_lock_error(&error) {
                    let delay_ms = 150 + (attempt as u64 * 150).min(1_500);
                    thread::sleep(Duration::from_millis(delay_ms));
                    continue;
                }
                return Err(io_error_message(&action, &error));
            }
        }
    }

    unreachable!("retry loop always returns on success or final failure")
}

fn rename_path(source: &Path, destination: &Path) -> Result<(), String> {
    retry_io_operation(
        format!(
            "failed to rename {} -> {}",
            source.display(),
            destination.display()
        ),
        || fs::rename(source, destination),
    )
}

fn copy_path(source: &Path, destination: &Path) -> Result<u64, String> {
    retry_io_operation(
        format!(
            "failed to copy {} -> {}",
            source.display(),
            destination.display()
        ),
        || fs::copy(source, destination),
    )
}

fn duplicate_path_best_effort(source: &Path, destination: &Path) -> Result<(), String> {
    retry_io_operation(
        format!(
            "failed to duplicate {} -> {}",
            source.display(),
            destination.display()
        ),
        || {
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }

            if clone_file_on_macos(source, destination).unwrap_or(false) {
                return Ok(());
            }

            fs::copy(source, destination)?;
            Ok(())
        },
    )
}

fn remove_file_path(path: &Path) -> Result<(), String> {
    retry_io_operation(format!("failed to remove {}", path.display()), || {
        fs::remove_file(path)
    })
}

fn remove_dir_all_path(path: &Path) -> Result<(), String> {
    retry_io_operation(
        format!("failed to remove directory {}", path.display()),
        || fs::remove_dir_all(path),
    )
}

fn create_dir_all_path(path: &Path) -> Result<(), String> {
    retry_io_operation(
        format!("failed to create directory {}", path.display()),
        || fs::create_dir_all(path),
    )
}

fn create_dir_path(path: &Path) -> Result<(), String> {
    retry_io_operation(
        format!("failed to create directory {}", path.display()),
        || fs::create_dir(path),
    )
}

fn metadata_path(path: &Path) -> Result<fs::Metadata, String> {
    retry_io_operation(
        format!("failed to read metadata for {}", path.display()),
        || fs::metadata(path),
    )
}

fn path_exists(path: &Path) -> Result<bool, String> {
    retry_io_operation(
        format!("failed to check whether {} exists", path.display()),
        || path.try_exists(),
    )
}

fn read_path(path: &Path) -> Result<Vec<u8>, String> {
    retry_io_operation(format!("failed to read {}", path.display()), || {
        fs::read(path)
    })
}

fn write_path(path: &Path, bytes: impl AsRef<[u8]>) -> Result<(), String> {
    let bytes = bytes.as_ref();
    retry_io_operation(format!("failed to write {}", path.display()), || {
        fs::write(path, bytes)
    })
}

fn canonicalize_path(path: &Path) -> Result<PathBuf, String> {
    retry_io_operation(format!("failed to canonicalize {}", path.display()), || {
        fs::canonicalize(path)
    })
}

fn open_file_path(path: &Path) -> Result<fs::File, String> {
    retry_io_operation(format!("failed to open {}", path.display()), || {
        fs::File::open(path)
    })
}

fn read_dir_path(path: &Path) -> Result<fs::ReadDir, String> {
    retry_io_operation(
        format!("failed to read directory {}", path.display()),
        || fs::read_dir(path),
    )
}
