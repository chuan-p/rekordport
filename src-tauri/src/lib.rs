use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::env;
#[cfg(target_os = "macos")]
use std::ffi::CString;
use std::fs::{self, OpenOptions};
use std::io;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
#[cfg(target_os = "macos")]
use std::os::unix::ffi::OsStrExt;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::Emitter;
use tempfile::Builder as TempBuilder;
use uuid::Uuid;

mod conversion_session;
#[cfg(test)]
mod migration_fixture_tests;
mod process;

include!("models.rs");
include!("platform.rs");
include!("tools.rs");
include!("fs_ops.rs");
include!("preview.rs");
include!("audio.rs");
include!("rekordbox.rs");
include!("migration.rs");
include!("locks.rs");
include!("conversion.rs");
include!("commands.rs");

pub fn run() {
    #[cfg(target_os = "windows")]
    if let Err(error) = ensure_webview2_runtime_before_launch() {
        show_webview2_install_failed_dialog(&error);
        return;
    }

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            default_database_path,
            preflight_check,
            pick_database_path,
            prepare_preview_path,
            open_path_in_file_manager,
            open_external_url,
            latest_release,
            scan_library,
            rekordbox_process_running,
            convert_tracks
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests;
