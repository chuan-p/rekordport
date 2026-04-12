use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io;
use std::io::Read;
use std::io::Write;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::Emitter;
use tempfile::Builder as TempBuilder;
use uuid::Uuid;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

mod conversion_session;
#[cfg(test)]
mod migration_fixture_tests;
mod process;

const DEFAULT_KEY: &str = "402fd482c38817c35ffa8ffb8c7d93143b749e7d315df7a81732a1ff43608497";
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;
#[cfg(target_os = "windows")]
const WEBVIEW2_BOOTSTRAPPER_URL: &str = "https://go.microsoft.com/fwlink/p/?LinkId=2124703";
#[cfg(any(target_os = "windows", test))]
const WEBVIEW2_CLIENT_GUID: &str = "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";
#[cfg(target_os = "windows")]
const WEBVIEW2_INSTALL_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Debug, Deserialize)]
struct ScanRequest {
    #[serde(rename = "dbPath")]
    db_path: String,
    #[serde(rename = "minBitDepth")]
    min_bit_depth: u32,
    #[serde(rename = "includeSampler")]
    include_sampler: bool,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct Track {
    id: String,
    #[serde(default)]
    source_id: Option<String>,
    #[serde(default)]
    analysis_state: Option<String>,
    #[serde(default)]
    analysis_note: Option<String>,
    title: String,
    artist: String,
    #[serde(rename = "file_type")]
    file_type: String,
    #[serde(default)]
    codec_name: Option<String>,
    bit_depth: Option<u32>,
    sample_rate: Option<u32>,
    bitrate: Option<u32>,
    full_path: String,
}

#[derive(Debug, Deserialize)]
struct ConvertRequest {
    #[serde(rename = "dbPath")]
    db_path: String,
    preset: String,
    #[serde(rename = "sourceHandling")]
    source_handling: String,
    #[serde(rename = "archiveConflictResolution", default)]
    archive_conflict_resolution: Option<String>,
    #[serde(rename = "outputConflictResolution", default)]
    output_conflict_resolution: Option<String>,
    tracks: Vec<Track>,
}

#[derive(Debug, Deserialize)]
struct PreflightRequest {
    #[serde(rename = "dbPath")]
    db_path: Option<String>,
}

#[derive(Debug, Serialize)]
struct ScanSummary {
    library_total: usize,
    candidate_total: usize,
    total: usize,
    flac: usize,
    alac: usize,
    hi_res: usize,
    m4a_candidates: usize,
    unreadable_m4a: usize,
    non_alac_m4a: usize,
    sampler_included: bool,
    min_bit_depth: u32,
    db_path: String,
}

#[derive(Debug, Serialize)]
struct ScanResponse {
    summary: ScanSummary,
    tracks: Vec<Track>,
}

#[derive(Debug, Serialize, Clone)]
struct ScanProgressPayload {
    phase: String,
    current: usize,
    total: usize,
    message: String,
}

#[derive(Debug, Deserialize)]
struct ExportRequest {
    #[serde(rename = "dbPath")]
    db_path: String,
    #[serde(rename = "minBitDepth")]
    min_bit_depth: u32,
    #[serde(rename = "includeSampler")]
    include_sampler: bool,
    #[serde(rename = "outputPath")]
    output_path: String,
    format: String,
}

#[derive(Debug, Serialize)]
struct ExportResponse {
    output_path: String,
    rows: usize,
}

#[derive(Debug, Serialize)]
struct ConvertResponse {
    backup_dir: String,
    converted_count: usize,
    analysis_migrated_count: usize,
    analysis_missing_count: usize,
    source_cleanup_mode: String,
    source_cleanup_failures: usize,
    cleanup_archived_dirs: usize,
    cleanup_archive_dir: Option<String>,
    converted_tracks: Vec<Track>,
}

#[derive(Debug, Serialize)]
struct PreflightResponse {
    os: String,
    sqlcipher_available: bool,
    ffmpeg_available: bool,
    sqlcipher_source: Option<String>,
    ffmpeg_source: Option<String>,
    m4a_encoder_available: bool,
    db_path: String,
    db_exists: bool,
    db_readable: bool,
    scan_ready: bool,
    convert_ready: bool,
    warnings: Vec<String>,
}

#[derive(Debug)]
struct ContentFileRef {
    id: String,
    path: String,
    rb_local_path: Option<String>,
    uuid: Option<String>,
    hash: Option<String>,
    size: Option<u64>,
}

#[derive(Debug)]
struct MigratedContentFile {
    original: ContentFileRef,
    new_id: String,
    new_uuid: Option<String>,
    new_path: String,
    new_local_path: Option<String>,
    hash: String,
    size: u64,
}

#[derive(Debug)]
struct ValidatedContentFile {
    original: ContentFileRef,
    source: PathBuf,
}

#[derive(Debug, Default)]
struct CleanupReport {
    archived_dirs: usize,
    archive_dir: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConflictResolution {
    Error,
    Overwrite,
    Redirect,
}

#[derive(Debug, Clone)]
struct ScanRow {
    id: String,
    title: String,
    artist: String,
    file_type: i32,
    bit_depth: Option<u32>,
    sample_rate: Option<u32>,
    bitrate: Option<u32>,
    full_path: String,
}

#[derive(Debug, Default)]
struct ScanStats {
    candidate_total: usize,
    m4a_candidates: usize,
    unreadable_m4a: usize,
    non_alac_m4a: usize,
}

#[derive(Debug)]
struct ScanOutcome {
    tracks: Vec<Track>,
    stats: ScanStats,
}

#[derive(Debug, Clone, Copy)]
enum SourceHandling {
    Rename,
    Trash,
}

#[derive(Debug, Clone, Default)]
struct AudioProbe {
    sample_rate: Option<u32>,
    channels: Option<u32>,
    bitrate_kbps: Option<u32>,
    has_attached_pic: bool,
}

static COMMAND_CACHE: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
static COMMAND_PATH_CACHE: OnceLock<Mutex<HashMap<String, Option<PathBuf>>>> = OnceLock::new();
static ENCODER_CACHE: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
static AUDIO_PROBE_CACHE: OnceLock<Mutex<HashMap<PathBuf, AudioProbe>>> = OnceLock::new();

#[cfg(target_os = "windows")]
fn hidden_windows_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(program);
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

#[cfg(target_os = "windows")]
fn webview2_runtime_installed() -> bool {
    let registry_keys = [
        format!(r"HKCU\Software\Microsoft\EdgeUpdate\Clients\{WEBVIEW2_CLIENT_GUID}"),
        format!(r"HKLM\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{WEBVIEW2_CLIENT_GUID}"),
        format!(r"HKLM\SOFTWARE\Microsoft\EdgeUpdate\Clients\{WEBVIEW2_CLIENT_GUID}"),
    ];

    registry_keys.iter().any(|key| {
        let output = hidden_windows_command("reg")
            .args(["query", key, "/v", "pv"])
            .output();

        output
            .ok()
            .and_then(|output| {
                if output.status.success() {
                    Some(String::from_utf8_lossy(&output.stdout).into_owned())
                } else {
                    None
                }
            })
            .and_then(|stdout| parse_webview2_registry_version(&stdout))
            .is_some()
    })
}

#[cfg(target_os = "windows")]
fn wait_for_webview2_runtime(timeout: Duration) -> bool {
    let started = std::time::Instant::now();
    while started.elapsed() < timeout {
        if webview2_runtime_installed() {
            return true;
        }
        thread::sleep(Duration::from_millis(500));
    }
    webview2_runtime_installed()
}

#[cfg(any(target_os = "windows", test))]
fn parse_webview2_registry_version(reg_query_stdout: &str) -> Option<String> {
    reg_query_stdout.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let name = parts.next()?;
        if !name.eq_ignore_ascii_case("pv") {
            return None;
        }

        let kind = parts.next()?;
        if !kind.eq_ignore_ascii_case("REG_SZ") {
            return None;
        }

        let version = parts.next()?.trim();
        if version.is_empty() || version == "0.0.0.0" {
            None
        } else {
            Some(version.to_string())
        }
    })
}

#[cfg(target_os = "windows")]
fn webview2_bootstrapper_path() -> PathBuf {
    runtime_support_root("webview2").join("MicrosoftEdgeWebview2Setup.exe")
}

#[cfg(target_os = "windows")]
fn runtime_support_root(category: &str) -> PathBuf {
    env::var_os("LOCALAPPDATA")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir)
        .join("rekordport")
        .join(category)
}

#[cfg(target_os = "windows")]
fn powershell_download_error(shell: &str, stderr: &[u8]) -> String {
    let detail = String::from_utf8_lossy(stderr).trim().to_string();
    if detail.is_empty() {
        format!("{shell} did not report any error details")
    } else {
        format!("{shell}: {detail}")
    }
}

#[cfg(target_os = "windows")]
fn download_webview2_bootstrapper(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            format!(
                "failed to create WebView2 bootstrapper directory {}: {e}",
                parent.display()
            )
        })?;
    }

    let mut errors = Vec::new();

    for shell in ["powershell.exe", "pwsh.exe"] {
        let output = match hidden_windows_command(shell)
            .args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                &format!(
                    "$ErrorActionPreference = 'Stop'; \
                     [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; \
                     Invoke-WebRequest -UseBasicParsing -Uri '{}' -OutFile '{}'",
                    WEBVIEW2_BOOTSTRAPPER_URL,
                    path.display().to_string().replace('\'', "''")
                ),
            ])
            .output()
        {
            Ok(output) => output,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                errors.push(format!("{shell} was not found"));
                continue;
            }
            Err(error) => {
                errors.push(format!("failed to start {shell}: {error}"));
                continue;
            }
        };

        if output.status.success() && path.exists() {
            return Ok(());
        }

        errors.push(powershell_download_error(shell, &output.stderr));
    }

    Err(format!(
        "failed to download the WebView2 Runtime bootstrapper. {}",
        errors.join(" | ")
    ))
}

#[cfg(target_os = "windows")]
fn install_webview2_runtime() -> Result<(), String> {
    let bootstrapper = webview2_bootstrapper_path();
    download_webview2_bootstrapper(&bootstrapper)?;

    let status = hidden_windows_command(&bootstrapper)
        .args(["/silent", "/install"])
        .status()
        .map_err(|e| format!("failed to start WebView2 Runtime installer: {e}"))?;

    if !status.success() {
        return Err(format!(
            "WebView2 Runtime installer exited with status: {status}"
        ));
    }

    if wait_for_webview2_runtime(WEBVIEW2_INSTALL_TIMEOUT) {
        return Ok(());
    }

    Err(format!(
        "WebView2 Runtime installer finished, but WebView2 was still not detected after waiting {} seconds",
        WEBVIEW2_INSTALL_TIMEOUT.as_secs()
    ))
}

#[cfg(target_os = "windows")]
fn show_webview2_installing_dialog() {
    let _ = rfd::MessageDialog::new()
        .set_title("Installing WebView2 Runtime")
        .set_description(
            "Rekordport needs Microsoft Edge WebView2 Runtime to open its window.\n\n\
             It is missing on this PC, so Rekordport will download Microsoft's Evergreen Bootstrapper and install it silently now. \
             Please keep this window open; the app will continue after installation.",
        )
        .set_buttons(rfd::MessageButtons::Ok)
        .set_level(rfd::MessageLevel::Info)
        .show();
}

#[cfg(target_os = "windows")]
fn show_webview2_install_failed_dialog(error: &str) {
    let _ = rfd::MessageDialog::new()
        .set_title("WebView2 Runtime is required")
        .set_description(format!(
            "Rekordport could not install Microsoft Edge WebView2 Runtime automatically.\n\n\
             Error: {error}\n\n\
             Please install WebView2 Runtime from Microsoft and open Rekordport again:\n\
             https://developer.microsoft.com/microsoft-edge/webview2/"
        ))
        .set_buttons(rfd::MessageButtons::Ok)
        .set_level(rfd::MessageLevel::Error)
        .show();
}

#[cfg(target_os = "windows")]
fn ensure_webview2_runtime_before_launch() -> Result<(), String> {
    if webview2_runtime_installed() {
        return Ok(());
    }

    show_webview2_installing_dialog();
    install_webview2_runtime()?;

    if wait_for_webview2_runtime(WEBVIEW2_INSTALL_TIMEOUT) {
        Ok(())
    } else {
        Err("WebView2 Runtime installation finished, but the runtime was still not detected in the registry".to_string())
    }
}

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

fn executable_filename(command: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("{command}.exe")
    } else {
        command.to_string()
    }
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
                if let Some(app_dir) = contents_dir.parent() {
                    push_root(app_dir.join("Resources"));
                    push_root(app_dir.join("Resources").join("bin"));
                }
            }
        }
    }

    if let Ok(cwd) = env::current_dir() {
        push_root(cwd.join("src-tauri").join("bin"));
        push_root(cwd.join("bin"));
        push_root(cwd);
    }

    roots
}

fn is_windows_unc_path(path: &Path) -> bool {
    #[cfg(target_os = "windows")]
    {
        matches!(
            path.components().next(),
            Some(Component::Prefix(prefix))
                if matches!(
                    prefix.kind(),
                    std::path::Prefix::UNC(_, _) | std::path::Prefix::VerbatimUNC(_, _)
                )
        )
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = path;
        false
    }
}

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

fn remove_file_path(path: &Path) -> Result<(), String> {
    retry_io_operation(format!("failed to remove {}", path.display()), || {
        fs::remove_file(path)
    })
}

fn create_dir_all_path(path: &Path) -> Result<(), String> {
    retry_io_operation(
        format!("failed to create directory {}", path.display()),
        || fs::create_dir_all(path),
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

fn open_file_path(path: &Path) -> Result<fs::File, String> {
    retry_io_operation(format!("failed to open {}", path.display()), || {
        fs::File::open(path)
    })
}

fn create_file_path(path: &Path) -> Result<fs::File, String> {
    retry_io_operation(format!("failed to create {}", path.display()), || {
        fs::File::create(path)
    })
}

fn read_dir_path(path: &Path) -> Result<fs::ReadDir, String> {
    retry_io_operation(
        format!("failed to read directory {}", path.display()),
        || fs::read_dir(path),
    )
}

fn preview_cache_root() -> Result<PathBuf, String> {
    let mut cache_root = std::env::temp_dir();
    cache_root.push("rekordport-preview-cache");
    create_dir_all_path(&cache_root)?;
    Ok(cache_root)
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
        return matches!(
            windows_preview_strategy(source),
            WindowsPreviewStrategy::TranscodeMp3
        );
    }

    #[cfg(all(not(target_os = "windows"), not(test)))]
    {
        let _ = source;
        false
    }
}

#[cfg(target_os = "windows")]
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
    ffmpeg.args(["-vn", "-ac", "2", "-ar", "44100", "-c:a", "libmp3lame", "-b:a", "192k"]);
    ffmpeg.arg(&cached);

    let output = ffmpeg.output().map_err(|e| {
        io_error_message(
            &format!("failed to run ffmpeg while preparing preview for {}", source.display()),
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

    if !is_windows_unc_path(&source) {
        return Ok(preview_path_string(&source));
    }

    let cached = preview_cache_path_for(&source)?;
    let source_meta = metadata_path(&source)?;
    let needs_refresh = match metadata_path(&cached) {
        Ok(meta) => meta.len() != source_meta.len(),
        Err(_) => true,
    };

    if needs_refresh {
        copy_path(&source, &cached).map_err(|e| {
            format!(
                "failed to cache network preview file locally ({} -> {}): {}",
                source.display(),
                cached.display(),
                e
            )
        })?;
    }

    Ok(preview_path_string(&cached))
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
            }
        }

        let sidecar = sidecar_filename(command);
        let plain = executable_filename(command);
        for root in candidate_search_roots() {
            for candidate in [root.join(&sidecar), root.join(&plain)] {
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
    let resolved = resolve_command(command)
        .ok_or_else(|| format!("{command} command not found in PATH or bundled sidecar"))?;
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
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    secs.to_string()
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

fn parse_number_before_marker(text: &str, marker: &str) -> Option<u32> {
    for (marker_index, _) in text.match_indices(marker) {
        let bytes = text.as_bytes();
        let mut end = marker_index;
        while end > 0 && bytes[end - 1].is_ascii_whitespace() {
            end -= 1;
        }

        let mut start = end;
        while start > 0 && bytes[start - 1].is_ascii_digit() {
            start -= 1;
        }

        if start < end {
            if let Ok(value) = text[start..end].parse::<u32>() {
                return Some(value);
            }
        }
    }

    None
}

fn parse_number_after_marker(text: &str, marker: &str) -> Option<u32> {
    let (marker_index, _) = text.match_indices(marker).next()?;
    let bytes = text.as_bytes();
    let mut start = marker_index + marker.len();
    while start < bytes.len() && bytes[start].is_ascii_whitespace() {
        start += 1;
    }

    let mut end = start;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }

    if start < end {
        text[start..end].parse::<u32>().ok()
    } else {
        None
    }
}

fn parse_audio_channels(text: &str) -> Option<u32> {
    let lower = text.to_ascii_lowercase();
    if let Some(value) = parse_number_before_marker(&lower, " channels") {
        return Some(value);
    }

    for (needle, channels) in [
        ("7.1", 8),
        ("6.1", 7),
        ("5.1", 6),
        ("5.0", 5),
        ("4.0", 4),
        ("stereo", 2),
        ("mono", 1),
    ] {
        if lower.contains(needle) {
            return Some(channels);
        }
    }

    None
}

fn parse_ffmpeg_audio_probe(text: &str) -> AudioProbe {
    let audio_line = text.lines().find(|line| line.contains("Audio:"));
    let probe_text = audio_line.unwrap_or(text);
    AudioProbe {
        sample_rate: parse_number_before_marker(probe_text, " Hz"),
        channels: audio_line.and_then(parse_audio_channels),
        bitrate_kbps: parse_number_before_marker(probe_text, " kb/s")
            .or_else(|| parse_number_after_marker(text, "bitrate:")),
        has_attached_pic: text.lines().any(|line| {
            let lower = line.to_ascii_lowercase();
            lower.contains("video:") && lower.contains("attached pic")
        }),
    }
}

fn probe_audio(path: &Path) -> Result<AudioProbe, String> {
    if !command_available("ffmpeg") {
        return Ok(AudioProbe::default());
    }

    let cache_key = path.to_path_buf();
    let cache = AUDIO_PROBE_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    {
        let guard = cache.lock().expect("audio probe cache lock poisoned");
        if let Some(value) = guard.get(&cache_key) {
            return Ok(value.clone());
        }
    }

    let mut ffmpeg = prepared_command("ffmpeg")?;
    ffmpeg.args(["-hide_banner", "-i"]);
    ffmpeg.arg(path);
    let output = ffmpeg.output().map_err(|e| {
        io_error_message(
            &format!("failed to run ffmpeg probe on {}", path.display()),
            &e,
        )
    })?;

    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    let probe = parse_ffmpeg_audio_probe(&text);

    let mut guard = cache.lock().expect("audio probe cache lock poisoned");
    guard.insert(cache_key, probe.clone());
    Ok(probe)
}

fn probe_channels(path: &Path) -> Result<u32, String> {
    Ok(probe_audio(path)?.channels.unwrap_or(2))
}

fn probe_sample_rate(path: &Path) -> Result<Option<u32>, String> {
    Ok(probe_audio(path)?.sample_rate)
}

fn probe_bitrate(path: &Path) -> Result<Option<u32>, String> {
    Ok(probe_audio(path)?.bitrate_kbps)
}

fn target_sample_rate_for_source(sample_rate: Option<u32>) -> u32 {
    let source = sample_rate.unwrap_or(44_100);
    match source {
        44_100 | 88_200 | 176_400 => 44_100,
        48_000 | 96_000 | 192_000 => 48_000,
        _ => {
            let diff_44 = source.abs_diff(44_100);
            let diff_48 = source.abs_diff(48_000);
            if diff_48 < diff_44 {
                48_000
            } else {
                44_100
            }
        }
    }
}

struct ConversionSpec {
    file_type: i32,
    extension: &'static str,
    ffmpeg_codec: &'static str,
    bit_depth: u32,
    bitrate_kbps: Option<u32>,
}

impl ConversionSpec {
    fn supports_embedded_artwork(&self) -> bool {
        matches!(self.extension, "mp3" | "m4a" | "aiff")
    }
}

fn preset_spec(preset: &str) -> Result<ConversionSpec, String> {
    match preset {
        "wav-auto" | "wav-44100" | "wav-48000" => Ok(ConversionSpec {
            file_type: 11,
            extension: "wav",
            ffmpeg_codec: "pcm_s16le",
            bit_depth: 16,
            bitrate_kbps: None,
        }),
        "aiff-auto" | "aiff-44100" | "aiff-48000" => Ok(ConversionSpec {
            file_type: 12,
            extension: "aiff",
            ffmpeg_codec: "pcm_s16be",
            bit_depth: 16,
            bitrate_kbps: None,
        }),
        "mp3-320" => Ok(ConversionSpec {
            file_type: 1,
            extension: "mp3",
            ffmpeg_codec: "libmp3lame",
            bit_depth: 16,
            bitrate_kbps: Some(320),
        }),
        "m4a-320" => Ok(ConversionSpec {
            file_type: 4,
            extension: "m4a",
            ffmpeg_codec: "aac_at",
            bit_depth: 16,
            bitrate_kbps: Some(320),
        }),
        _ => Err(format!("unsupported preset: {preset}")),
    }
}

fn source_handling_mode(value: &str) -> Result<SourceHandling, String> {
    match value {
        "rename" => Ok(SourceHandling::Rename),
        "trash" => Ok(SourceHandling::Trash),
        _ => Err(format!("unsupported source handling mode: {value}")),
    }
}

fn source_handling_name(mode: SourceHandling) -> &'static str {
    match mode {
        SourceHandling::Rename => "rename",
        SourceHandling::Trash => "trash",
    }
}

fn compute_pcm_bitrate(sample_rate: u32, channels: u32, bit_depth: u32) -> u32 {
    (((sample_rate as u64) * (channels as u64) * (bit_depth as u64)) / 1000) as u32
}

fn source_bitrate_kbps(track: &Track, source: &Path) -> Result<u32, String> {
    if let Some(value) = track.bitrate {
        if value > 0 {
            return Ok(value);
        }
    }

    if matches!(track.file_type.as_str(), "WAV" | "AIFF") {
        let sample_rate = probe_sample_rate(source)?
            .or(track.sample_rate)
            .unwrap_or(44_100);
        let channels = probe_channels(source)?;
        let bit_depth = track.bit_depth.unwrap_or(16);
        return Ok(compute_pcm_bitrate(sample_rate, channels, bit_depth));
    }

    if let Some(value) = probe_bitrate(source)? {
        return Ok(value);
    }

    Ok(track.bitrate.unwrap_or(0))
}

fn run_sqlcipher(db_path: &Path, key: &str, sql: &str) -> Result<String, String> {
    if !command_available("sqlcipher") {
        return Err("sqlcipher command not found in PATH or bundled sidecar".into());
    }

    let script = format!("PRAGMA key = '{key}';\nPRAGMA foreign_keys = ON;\n{sql}\n");

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

fn table_columns(db_path: &Path, key: &str, table: &str) -> Result<Vec<String>, String> {
    let sql = format!("PRAGMA table_info({table});");
    let lines = sqlcipher_lines(db_path, key, &sql)?;
    Ok(lines
        .into_iter()
        .filter_map(|line| line.split('|').nth(1).map(|value| value.to_string()))
        .collect())
}

fn parse_optional_u32(value: Option<&str>) -> Option<u32> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<u32>().ok())
}

fn sampler_path_predicate(column: &str) -> String {
    format!(r"REPLACE(COALESCE({column}, ''), '\', '/') NOT LIKE '%/Sampler/%'")
}

fn build_scan_query(min_bit_depth: u32, include_sampler: bool) -> String {
    let sampler_filter = if include_sampler {
        String::new()
    } else {
        format!("\n  AND {}", sampler_path_predicate("c.FolderPath"))
    };

    format!(
        ".headers on\n.mode csv\nSELECT\n  COALESCE(c.ID, '') AS id,\n  COALESCE(c.Title, '') AS title,\n  COALESCE(a.Name, c.SrcArtistName, '') AS artist,\n  c.FileType AS file_type,\n  c.BitDepth AS bit_depth,\n  c.SampleRate AS sample_rate,\n  c.BitRate AS bitrate,\n  COALESCE(c.FolderPath, '') AS full_path\nFROM djmdContent c\nLEFT JOIN djmdArtist a ON a.ID = c.ArtistID\nWHERE\n  (\n    c.FileType = 5\n    OR c.FileType = 6\n    OR (c.FileType IN (11, 12) AND COALESCE(c.BitDepth, 0) > {min_bit_depth})\n  ){sampler_filter}\nORDER BY\n  artist COLLATE NOCASE,\n  title COLLATE NOCASE,\n  full_path COLLATE NOCASE;"
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
        let codec_name = if row.file_type == 6 {
            stats.m4a_candidates += 1;
            Some("alac".to_string())
        } else {
            None
        };

        tracks.push(Track {
            id: row.id,
            source_id: None,
            analysis_state: None,
            analysis_note: None,
            title: row.title,
            artist: row.artist,
            file_type: file_type_name(row.file_type, codec_name.as_deref()),
            codec_name,
            bit_depth: row.bit_depth,
            sample_rate: row.sample_rate,
            bitrate: row.bitrate,
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

fn scan_tracks(
    db_path: &Path,
    key: &str,
    min_bit_depth: u32,
    include_sampler: bool,
) -> Result<Vec<Track>, String> {
    Ok(scan_tracks_with_progress(db_path, key, min_bit_depth, include_sampler, |_| {})?.tracks)
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

fn fetch_content_files(
    db_path: &Path,
    key: &str,
    content_id: &str,
) -> Result<Vec<ContentFileRef>, String> {
    let sql = format!(
    "SELECT ID, Path, COALESCE(rb_local_path, ''), UUID, COALESCE(Hash, ''), COALESCE(Size, '') FROM contentFile WHERE ContentID = {} ORDER BY ID;",
    sql_quote(content_id),
  );
    let mut files = Vec::new();
    for line in sqlcipher_lines(db_path, key, &sql)? {
        let mut parts = line.split('|');
        let id = parts.next().unwrap_or_default().to_string();
        let path = parts.next().unwrap_or_default().to_string();
        let rb_local_path = parts.next().unwrap_or_default().to_string();
        let uuid = parts.next().unwrap_or_default().to_string();
        let hash = parts.next().unwrap_or_default().to_string();
        let size = parts.next().unwrap_or_default().to_string();
        if id.is_empty() || path.is_empty() {
            continue;
        }
        files.push(ContentFileRef {
            id,
            path,
            rb_local_path: if rb_local_path.is_empty() {
                None
            } else {
                Some(rb_local_path)
            },
            uuid: if uuid.is_empty() { None } else { Some(uuid) },
            hash: if hash.is_empty() { None } else { Some(hash) },
            size: size.parse::<u64>().ok(),
        });
    }
    Ok(files)
}

fn copy_file_with_parent_dirs(source: &Path, destination: &Path) -> Result<(), String> {
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
    copy_path(source, destination)?;
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

fn csv_escape(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') || value.contains('\r') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn write_csv_export(path: &Path, tracks: &[Track]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        create_dir_all_path(parent)?;
    }

    let mut content = String::from(
        "id,title,artist,file_type,codec_name,bit_depth,sample_rate,bitrate,full_path\n",
    );
    for track in tracks {
        let row = [
            track.id.as_str(),
            track.title.as_str(),
            track.artist.as_str(),
            track.file_type.as_str(),
            track.codec_name.as_deref().unwrap_or(""),
            &track
                .bit_depth
                .map(|value| value.to_string())
                .unwrap_or_default(),
            &track
                .sample_rate
                .map(|value| value.to_string())
                .unwrap_or_default(),
            &track
                .bitrate
                .map(|value| value.to_string())
                .unwrap_or_default(),
            track.full_path.as_str(),
        ];
        content.push_str(
            &row.iter()
                .map(|value| csv_escape(value))
                .collect::<Vec<_>>()
                .join(","),
        );
        content.push('\n');
    }

    write_path(path, content)
}

fn excel_col_name(index: usize) -> String {
    let mut index = index + 1;
    let mut name = String::new();
    while index > 0 {
        let remainder = (index - 1) % 26;
        name.insert(0, char::from(b'A' + remainder as u8));
        index = (index - 1) / 26;
    }
    name
}

fn xlsx_cell(reference: &str, value: Option<&str>) -> String {
    match value {
        None => format!("<c r=\"{reference}\"/>"),
        Some(text) if text.parse::<f64>().is_ok() && !text.is_empty() => {
            format!("<c r=\"{reference}\"><v>{text}</v></c>")
        }
        Some(text) => format!(
            "<c r=\"{reference}\" t=\"inlineStr\"><is><t xml:space=\"preserve\">{}</t></is></c>",
            xml_escape(text)
        ),
    }
}

fn write_xlsx_export(path: &Path, tracks: &[Track]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        create_dir_all_path(parent)?;
    }

    let headers = [
        "id",
        "title",
        "artist",
        "file_type",
        "codec_name",
        "bit_depth",
        "sample_rate",
        "bitrate",
        "full_path",
    ];
    let mut rows = Vec::new();
    rows.push(
        headers
            .iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>(),
    );
    rows.extend(tracks.iter().map(|track| {
        vec![
            track.id.clone(),
            track.title.clone(),
            track.artist.clone(),
            track.file_type.clone(),
            track.codec_name.clone().unwrap_or_default(),
            track
                .bit_depth
                .map(|value| value.to_string())
                .unwrap_or_default(),
            track
                .sample_rate
                .map(|value| value.to_string())
                .unwrap_or_default(),
            track
                .bitrate
                .map(|value| value.to_string())
                .unwrap_or_default(),
            track.full_path.clone(),
        ]
    }));

    let rows_xml = rows
        .iter()
        .enumerate()
        .map(|(row_index, row)| {
            let cells = row
                .iter()
                .enumerate()
                .map(|(col_index, value)| {
                    let reference = format!("{}{}", excel_col_name(col_index), row_index + 1);
                    xlsx_cell(
                        &reference,
                        if value.is_empty() { None } else { Some(value) },
                    )
                })
                .collect::<Vec<_>>()
                .join("");
            format!("<row r=\"{}\">{cells}</row>", row_index + 1)
        })
        .collect::<Vec<_>>()
        .join("");

    let sheet_xml = format!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheetData>{rows_xml}</sheetData></worksheet>"
  );
    let workbook_xml = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><sheets><sheet name=\"Tracks\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>";
    let workbook_rels_xml = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/></Relationships>";
    let root_rels_xml = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>";
    let content_types_xml = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/><Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/></Types>";

    let file = create_file_path(path)?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zip.start_file("[Content_Types].xml", options)
        .map_err(|e| e.to_string())?;
    zip.write_all(content_types_xml.as_bytes())
        .map_err(|e| e.to_string())?;
    zip.start_file("_rels/.rels", options)
        .map_err(|e| e.to_string())?;
    zip.write_all(root_rels_xml.as_bytes())
        .map_err(|e| e.to_string())?;
    zip.start_file("xl/workbook.xml", options)
        .map_err(|e| e.to_string())?;
    zip.write_all(workbook_xml.as_bytes())
        .map_err(|e| e.to_string())?;
    zip.start_file("xl/_rels/workbook.xml.rels", options)
        .map_err(|e| e.to_string())?;
    zip.write_all(workbook_rels_xml.as_bytes())
        .map_err(|e| e.to_string())?;
    zip.start_file("xl/worksheets/sheet1.xml", options)
        .map_err(|e| e.to_string())?;
    zip.write_all(sheet_xml.as_bytes())
        .map_err(|e| e.to_string())?;
    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
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
            if expected_size == 0 || expected_size != actual_size {
                return Err(format!(
                    "analysis resource size mismatch for {} (db {}, disk {})",
                    source.display(),
                    expected_size,
                    actual_size
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

            let actual_hash = md5_hex(&source)?;
            if expected_hash != &actual_hash {
                return Err(format!(
                    "analysis resource hash mismatch for {} (db {}, disk {})",
                    source.display(),
                    expected_hash,
                    actual_hash
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

    for dir in &orphan_dirs {
        let relative = dir
            .strip_prefix(&analysis_root)
            .map_err(|e| e.to_string())?;
        let target = archive_root.join(relative);
        if let Some(parent) = target.parent() {
            create_dir_all_path(parent)?;
        }
        rename_path(dir, &target)?;
    }

    Ok(CleanupReport {
        archived_dirs: orphan_dirs.len(),
        archive_dir: Some(archive_root.to_string_lossy().to_string()),
    })
}

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

fn ensure_database_writable(db_path: &Path, key: &str) -> Result<(), String> {
    run_sqlcipher(db_path, key, "BEGIN IMMEDIATE;\nROLLBACK;")
        .map(|_| ())
        .map_err(|error| {
            format!(
                "database is not writable before conversion: {}. Close Rekordbox and any backup/sync tools that may be using master.db, then try again.",
                error
            )
        })
}

fn preflight_impl(req: PreflightRequest) -> PreflightResponse {
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
    let db_exists = !db_path.is_empty() && db_path_buf.exists();
    let db_readable = if db_exists {
        check_database_readable(&db_path_buf, DEFAULT_KEY)
    } else {
        false
    };

    let mut warnings = Vec::new();
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

    let scan_ready = sqlcipher_available && db_readable;
    let convert_ready = ffmpeg_available && sqlcipher_available && db_readable;

    PreflightResponse {
        os: platform_name(),
        sqlcipher_available,
        ffmpeg_available,
        sqlcipher_source,
        ffmpeg_source,
        m4a_encoder_available,
        db_path,
        db_exists,
        db_readable,
        scan_ready,
        convert_ready,
        warnings,
    }
}

fn backup_file_tree(source: &Path, backup_root: &Path) -> Result<PathBuf, String> {
    let relative = backup_relative_path(source);
    let target = backup_root.join(relative);
    if let Some(parent) = target.parent() {
        create_dir_all_path(parent)?;
    }
    copy_path(source, &target)?;
    Ok(target)
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
    backup_root: &Path,
    archive_conflict_resolution: ConflictResolution,
    output_conflict_resolution: ConflictResolution,
) -> Result<(Track, PathBuf, PathBuf), String> {
    let source = Path::new(&track.full_path);
    if !path_exists(source)? {
        return Err(format!("source file not found: {}", source.display()));
    }

    let source_sample_rate = probe_sample_rate(source)?.or(track.sample_rate);
    let target_sample_rate = target_sample_rate_for_source(source_sample_rate);
    let source_bitrate = source_bitrate_kbps(track, source)?;
    let mut archive_path =
        build_source_archive_path(source, source_bitrate, archive_conflict_resolution)?;
    let mut output_path = build_target_path(source, spec, output_conflict_resolution)?;

    backup_file_tree(source, backup_root)?;

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

    let conversion_result = (|| -> Result<(), String> {
        let cover_art_supported = spec.supports_embedded_artwork();
        let has_attached_pic = if cover_art_supported {
            probe_audio(&archive_path)?.has_attached_pic
        } else {
            false
        };

        let mut ffmpeg = prepared_command("ffmpeg")?;
        ffmpeg.args(["-hide_banner", "-loglevel", "error", "-y", "-i"]);
        ffmpeg.arg(&archive_path);
        ffmpeg.args(["-map", "0:a:0", "-map_metadata", "0", "-c:a", spec.ffmpeg_codec]);
        if has_attached_pic {
            ffmpeg.args([
                "-map",
                "0:v:0?",
                "-c:v",
                "png",
                "-disposition:v:0",
                "attached_pic",
            ]);
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
        let _ = remove_file_path(&temp_output_path);
        let _ = rename_path(&archive_path, source);
        return Err(error);
    }

    if path_exists(&output_path)? {
        match output_conflict_resolution {
            ConflictResolution::Error => {
                let _ = remove_file_path(&temp_output_path);
                let _ = rename_path(&archive_path, source);
                return Err(format!(
                    "target file already exists: {}",
                    output_path.display()
                ));
            }
            ConflictResolution::Overwrite => {
                if let Err(error) = remove_file_path(&output_path) {
                    let _ = remove_file_path(&temp_output_path);
                    let _ = rename_path(&archive_path, source);
                    return Err(error);
                }
            }
            ConflictResolution::Redirect => {
                output_path = match unique_redirect_path(&output_path) {
                    Ok(path) => path,
                    Err(error) => {
                        let _ = remove_file_path(&temp_output_path);
                        let _ = rename_path(&archive_path, source);
                        return Err(error);
                    }
                };
            }
        }
    }

    if let Err(error) = rename_path(&temp_output_path, &output_path) {
        let _ = remove_file_path(&temp_output_path);
        let _ = rename_path(&archive_path, source);
        return Err(error);
    }

    let channels = probe_channels(&archive_path)?;
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

    Ok((converted, output_path, archive_path))
}

fn migrate_tracks_in_db(
    db_path: &Path,
    tracks: &[Track],
    output_tracks: &[Track],
    key: &str,
    spec: &ConversionSpec,
) -> Result<Vec<Track>, String> {
    let content_columns = table_columns(db_path, key, "djmdContent")?;
    let insert_columns = content_columns;
    let now_expr = "strftime('%Y-%m-%d %H:%M:%f +00:00','now')";
    let mut copied_resources: Vec<PathBuf> = Vec::new();
    let result = (|| -> Result<Vec<Track>, String> {
        let mut sql = String::from("BEGIN IMMEDIATE;\n");
        sql.push_str(
            "CREATE TEMP TABLE IF NOT EXISTS migration_state (next_id INTEGER NOT NULL);\n",
        );
        sql.push_str("DELETE FROM migration_state;\n");
        sql.push_str("INSERT INTO migration_state (next_id) SELECT COALESCE(MAX(CAST(ID AS INTEGER)), 0) + 1 FROM djmdContent WHERE ID GLOB '[0-9]*';\n");
        sql.push_str("CREATE TEMP TABLE IF NOT EXISTS migration_results (source_id TEXT NOT NULL, new_id TEXT NOT NULL);\n");
        sql.push_str("DELETE FROM migration_results;\n");

        let new_content_id_expr = "(SELECT CAST(next_id AS TEXT) FROM migration_state LIMIT 1)";
        let new_content_id_text_expr =
            "(SELECT CAST(next_id AS TEXT) FROM migration_state LIMIT 1)";
        let mut analysis_summaries: Vec<(String, String)> = Vec::with_capacity(tracks.len());

        for (track, output_track) in tracks.iter().zip(output_tracks.iter()) {
            let output_path = Path::new(&output_track.full_path);
            let file_name = output_path
                .file_name()
                .ok_or_else(|| format!("missing file name for {}", output_path.display()))?
                .to_string_lossy()
                .to_string();
            let folder_path = output_path.to_string_lossy().to_string();
            let file_size = metadata_path(output_path)?.len();
            let old_uuid = sqlcipher_required_value(
                db_path,
                key,
                &format!(
                    "SELECT UUID FROM djmdContent WHERE ID = {} LIMIT 1;",
                    sql_quote(&track.id)
                ),
                &format!("missing UUID for source content {}", track.id),
            )?;
            let old_analysis_path = sqlcipher_single_value(
                db_path,
                key,
                &format!(
                    "SELECT AnalysisDataPath FROM djmdContent WHERE ID = {} LIMIT 1;",
                    sql_quote(&track.id)
                ),
            )?
            .unwrap_or_default();
            let content_files = fetch_content_files(db_path, key, &track.id)?;
            let content_uuid = Uuid::new_v4().to_string();
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

            match validate_analysis_resources(&content_files) {
                Ok(validated_files) => {
                    for file in validated_files {
                        let source_path =
                            file.original.rb_local_path.as_ref().ok_or_else(|| {
                                format!("analysis resource path missing for {}", file.original.id)
                            })?;
                        let destination_path = rewrite_analysis_resource_path(
                            source_path,
                            &old_uuid,
                            file.original.uuid.as_deref(),
                            &content_uuid,
                        );
                        let destination = PathBuf::from(&destination_path);
                        copy_file_with_parent_dirs(&file.source, &destination)?;
                        rewrite_anlz_ppth(&destination, &file_name)?;
                        copied_resources.push(destination.clone());
                        let size = metadata_path(&destination)?.len();
                        let hash = md5_hex(&destination)?;
                        let new_id = rewrite_analysis_resource_value(
                            &file.original.id,
                            &old_uuid,
                            file.original.uuid.as_deref(),
                            &content_uuid,
                        );
                        let new_path = rewrite_analysis_resource_path(
                            &file.original.path,
                            &old_uuid,
                            file.original.uuid.as_deref(),
                            &content_uuid,
                        );
                        let new_local_path = file.original.rb_local_path.as_ref().map(|path| {
                            rewrite_analysis_resource_path(
                                path,
                                &old_uuid,
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
                rewrite_analysis_resource_path(&old_analysis_path, &old_uuid, None, &content_uuid)
            };
            let new_analysis_path = if missing_analysis_resource {
                String::new()
            } else {
                new_analysis_path
            };

            sql.push_str(&format!(
        "UPDATE djmdContent SET FolderPath = {}, FileNameL = {}, FileNameS = {}, AnalysisDataPath = {}, FileType = {}, BitDepth = {}, BitRate = {}, SampleRate = {}, FileSize = {}, updated_at = {now_expr} WHERE ID = {new_content_id_expr};\n",
        sql_quote(&folder_path),
        sql_quote(&file_name),
        sql_quote(&file_name),
        sql_quote(&new_analysis_path),
        spec.file_type,
        spec.bit_depth,
        output_track.bitrate.unwrap_or(0),
        output_track.sample_rate.unwrap_or(44_100),
        file_size,
      ));

            sql.push_str(&format!(
        "UPDATE djmdCue SET ContentID = {new_content_id_expr}, ContentUUID = {}, updated_at = {now_expr} WHERE ContentID = {};\n",
        sql_quote(&content_uuid),
        sql_quote(&track.id),
      ));
            sql.push_str(&format!(
        "UPDATE contentActiveCensor SET ID = REPLACE(ID, {}, {}), ContentID = {new_content_id_expr}, updated_at = {now_expr} WHERE ContentID = {};\n",
        sql_quote(&old_uuid),
        sql_quote(&content_uuid),
        sql_quote(&track.id),
      ));
            sql.push_str(&format!(
        "UPDATE djmdActiveCensor SET ID = REPLACE(ID, {}, {}), ContentID = {new_content_id_expr}, ContentUUID = {}, updated_at = {now_expr} WHERE ContentID = {};\n",
        sql_quote(&old_uuid),
        sql_quote(&content_uuid),
        sql_quote(&content_uuid),
        sql_quote(&track.id),
      ));
            sql.push_str(&format!(
        "UPDATE djmdMixerParam SET ContentID = {new_content_id_expr}, updated_at = {now_expr} WHERE ContentID = {};\n",
        sql_quote(&track.id),
      ));
            sql.push_str(&format!(
        "UPDATE djmdSongPlaylist SET ContentID = {new_content_id_expr}, updated_at = {now_expr} WHERE ContentID = {} AND PlaylistID IN (SELECT ID FROM djmdPlaylist WHERE COALESCE(SmartList, '') = '');\n",
        sql_quote(&track.id),
      ));
            sql.push_str(&format!(
        "UPDATE djmdSongMyTag SET ContentID = {new_content_id_expr}, updated_at = {now_expr} WHERE ContentID = {};\n",
        sql_quote(&track.id),
      ));
            sql.push_str(&format!(
        "UPDATE djmdSongTagList SET ContentID = {new_content_id_expr}, updated_at = {now_expr} WHERE ContentID = {};\n",
        sql_quote(&track.id),
      ));
            sql.push_str(&format!(
        "UPDATE djmdSongHotCueBanklist SET ContentID = {new_content_id_expr}, updated_at = {now_expr} WHERE ContentID = {};\n",
        sql_quote(&track.id),
      ));
            sql.push_str(&format!(
        "UPDATE djmdSongHistory SET ContentID = {new_content_id_expr}, updated_at = {now_expr} WHERE ContentID = {};\n",
        sql_quote(&track.id),
      ));
            sql.push_str(&format!(
        "UPDATE djmdSongRelatedTracks SET ContentID = {new_content_id_expr}, updated_at = {now_expr} WHERE ContentID = {};\n",
        sql_quote(&track.id),
      ));
            sql.push_str(&format!(
        "UPDATE djmdSongSampler SET ContentID = {new_content_id_expr}, updated_at = {now_expr} WHERE ContentID = {};\n",
        sql_quote(&track.id),
      ));
            sql.push_str(&format!(
        "UPDATE djmdRecommendLike SET ContentID1 = CASE WHEN ContentID1 = {} THEN {new_content_id_expr} ELSE ContentID1 END, ContentID2 = CASE WHEN ContentID2 = {} THEN {new_content_id_expr} ELSE ContentID2 END, updated_at = {now_expr} WHERE ContentID1 = {} OR ContentID2 = {};\n",
        sql_quote(&track.id),
        sql_quote(&track.id),
        sql_quote(&track.id),
        sql_quote(&track.id),
      ));

            sql.push_str(&format!(
        "UPDATE contentCue SET ID = {}, ContentID = {new_content_id_expr}, Cues = REPLACE(REPLACE(Cues, {}, {}), {}, {}), rb_cue_count = COALESCE(json_array_length(REPLACE(REPLACE(Cues, {}, {}), {}, {})), 0), updated_at = {now_expr} WHERE ContentID = {};\n",
        sql_quote(&content_uuid),
        sql_quote(&track.id),
        new_content_id_text_expr,
        sql_quote(&old_uuid),
        sql_quote(&content_uuid),
        sql_quote(&track.id),
        new_content_id_text_expr,
        sql_quote(&old_uuid),
        sql_quote(&content_uuid),
        sql_quote(&track.id),
      ));

            for file in &migrated_content_files {
                let new_local_path = file.new_local_path.clone().unwrap_or_default();
                sql.push_str(&format!(
          "UPDATE contentFile SET ID = {}, ContentID = {new_content_id_expr}, UUID = {}, Path = {}, rb_local_path = {}, Hash = {}, Size = {}, updated_at = {now_expr} WHERE ID = {} AND ContentID = {};\n",
          sql_quote(&file.new_id),
          file
            .new_uuid
            .as_ref()
            .map(|uuid| sql_quote(uuid))
            .unwrap_or_else(|| "UUID".to_string()),
          sql_quote(&file.new_path),
          if new_local_path.is_empty() {
            "NULL".to_string()
          } else {
            sql_quote(&new_local_path)
          },
          sql_quote(&file.hash),
          file.size,
          sql_quote(&file.original.id),
          sql_quote(&track.id),
        ));
            }

            for file in &content_files {
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
        "INSERT INTO migration_results (source_id, new_id) VALUES ({}, {new_content_id_expr});\n",
        sql_quote(&track.id),
      ));
            sql.push_str("UPDATE migration_state SET next_id = next_id + 1;\n");
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
            migrated.source_id = Some(track.id.clone());
            migrated.analysis_state = Some(analysis_state);
            migrated.analysis_note = Some(analysis_note);
            migrated_tracks.push(migrated);
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

    if spec.extension == "m4a" && !ffmpeg_has_encoder("aac_at")? {
        return Err(
            "ffmpeg was built without Apple's aac_at encoder, so M4A 320kbps is unavailable".into(),
        );
    }
    ensure_database_writable(&db_path, DEFAULT_KEY)?;

    let timestamp = timestamp_token();
    let backup_root = db_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!("rkb-lossless-backup-{timestamp}"));
    create_dir_all_path(&backup_root)?;

    let db_backup = backup_root.join("master.db");
    copy_path(&db_path, &db_backup)?;

    let music_backup_root = backup_root.join("music");
    let mut session = conversion_session::ConversionSession::new();
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
            &music_backup_root,
            archive_conflict_resolution,
            output_conflict_resolution,
        ) {
            Ok((converted_track, output_path, archive_path)) => {
                session.push(track, converted_track, output_path, archive_path);
            }
            Err(error) => {
                session.rollback_all();
                return Err(error);
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
                session.rollback_all();
                return Err(error);
            }
        };

    let cleanup_report = cleanup_orphan_zero_analysis_dirs(&db_path, DEFAULT_KEY)?;
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

    let response = ConvertResponse {
        backup_dir: backup_root.to_string_lossy().to_string(),
        converted_count: migrated_tracks.len(),
        analysis_migrated_count,
        analysis_missing_count,
        source_cleanup_mode: source_handling_name(source_handling).to_string(),
        source_cleanup_failures,
        cleanup_archived_dirs: cleanup_report.archived_dirs,
        cleanup_archive_dir: cleanup_report.archive_dir,
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
        .filter(|track| matches!(track.file_type.as_str(), "WAV" | "AIFF"))
        .count();

    let response = ScanResponse {
        summary: ScanSummary {
            library_total,
            candidate_total: outcome.stats.candidate_total,
            total: tracks.len(),
            flac,
            alac,
            hi_res,
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
        message: if response.summary.total == 0 {
            "Scan complete. No tracks need processing.".to_string()
        } else {
            format!(
                "Scan complete. Found {} results from {} candidate tracks.",
                response.summary.total, response.summary.candidate_total
            )
        },
    });

    Ok(response)
}

#[allow(dead_code)]
fn scan_impl(req: ScanRequest) -> Result<ScanResponse, String> {
    scan_impl_with_progress(req, |_| {})
}

fn export_impl(req: ExportRequest) -> Result<ExportResponse, String> {
    let tracks = scan_tracks(
        Path::new(&req.db_path),
        DEFAULT_KEY,
        req.min_bit_depth,
        req.include_sampler,
    )?;
    let output_path = PathBuf::from(&req.output_path);
    match req.format.as_str() {
        "csv" => write_csv_export(&output_path, &tracks)?,
        "xlsx" => write_xlsx_export(&output_path, &tracks)?,
        other => return Err(format!("unsupported export format: {other}")),
    }
    Ok(ExportResponse {
        output_path: req.output_path,
        rows: tracks.len(),
    })
}

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
fn pick_export_path(format: String, suggested_name: String) -> Option<String> {
    let filter = if format.to_lowercase() == "xlsx" {
        ("Excel Workbook", vec!["xlsx"])
    } else {
        ("CSV", vec!["csv"])
    };

    rfd::FileDialog::new()
        .set_title("Choose export location")
        .add_filter(filter.0, &filter.1)
        .set_file_name(suggested_name)
        .save_file()
        .map(|path| path.to_string_lossy().to_string())
}

#[tauri::command]
fn prepare_preview_path(path: String) -> Result<String, String> {
    prepare_preview_path_impl(path)
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
            command.arg(format!("/select,{}", normalized));
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
        return Err(format!("failed to open url: {trimmed}"));
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

#[tauri::command]
async fn scan_library(app: tauri::AppHandle, req: ScanRequest) -> Result<ScanResponse, String> {
    tauri::async_runtime::spawn_blocking(move || {
        scan_impl_with_progress(req, |payload| {
            let _ = app.emit("scan-progress", payload);
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn export_tracks(req: ExportRequest) -> Result<ExportResponse, String> {
    tauri::async_runtime::spawn_blocking(move || export_impl(req))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn convert_tracks(
    app: tauri::AppHandle,
    req: ConvertRequest,
) -> Result<ConvertResponse, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let result = convert_impl_with_progress(req, |payload| {
            let _ = app.emit("convert-progress", payload);
        });

        if let Err(error) = &result {
            let _ = app.emit(
                "convert-progress",
                ScanProgressPayload {
                    phase: "error".to_string(),
                    current: 0,
                    total: 0,
                    message: error.clone(),
                },
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
            pick_export_path,
            prepare_preview_path,
            open_path_in_file_manager,
            open_external_url,
            scan_library,
            export_tracks,
            rekordbox_process_running,
            convert_tracks
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn normalize_path_separators(value: &str) -> String {
        if cfg!(target_os = "windows") {
            value.replace('/', "\\")
        } else {
            value.replace('\\', "/")
        }
    }

    #[test]
    fn normalizes_windows_path_strings() {
        assert_eq!(
            normalize_windows_path_string(r"D:/Music/Other\2 Unlimited,Remo-Conv - Twilight Zone.aiff"),
            r"D:\Music\Other\2 Unlimited,Remo-Conv - Twilight Zone.aiff"
        );
        assert_eq!(
            normalize_windows_path_string(r"\\?\D:\Music\Track.wav"),
            r"D:\Music\Track.wav"
        );
        assert_eq!(
            normalize_windows_path_string(r"\\?\UNC\server\share\Track.wav"),
            r"\\server\share\Track.wav"
        );
    }

    #[test]
    fn chooses_windows_preview_strategy() {
        assert_eq!(
            windows_preview_strategy(Path::new("track.mp3")),
            WindowsPreviewStrategy::CopyOriginal
        );
        assert_eq!(
            windows_preview_strategy(Path::new("track.m4a")),
            WindowsPreviewStrategy::CopyOriginal
        );
        assert_eq!(
            windows_preview_strategy(Path::new("track.aac")),
            WindowsPreviewStrategy::CopyOriginal
        );
        assert_eq!(
            windows_preview_strategy(Path::new("track.aiff")),
            WindowsPreviewStrategy::TranscodeMp3
        );
        assert_eq!(
            windows_preview_strategy(Path::new("track.flac")),
            WindowsPreviewStrategy::TranscodeMp3
        );
        assert_eq!(
            windows_preview_strategy(Path::new("track.wav")),
            WindowsPreviewStrategy::TranscodeMp3
        );
        assert_eq!(
            windows_preview_strategy(Path::new("track.ogg")),
            WindowsPreviewStrategy::TranscodeMp3
        );
    }

    #[test]
    fn parses_webview2_runtime_registry_version() {
        let output = format!(
            r#"
HKEY_CURRENT_USER\Software\Microsoft\EdgeUpdate\Clients\{WEBVIEW2_CLIENT_GUID}
    pv    REG_SZ    146.0.3856.109
"#
        );

        assert_eq!(
            parse_webview2_registry_version(&output),
            Some("146.0.3856.109".to_string())
        );
        assert_eq!(
            parse_webview2_registry_version("    pv    REG_SZ    0.0.0.0"),
            None
        );
        assert_eq!(parse_webview2_registry_version(""), None);
    }

    #[test]
    fn parses_ffmpeg_stereo_probe() {
        let probe = parse_ffmpeg_audio_probe(
            "Input #0, flac, from 'song.flac':\n  Duration: 00:03:00.00, bitrate: 2847 kb/s\n  Stream #0:0: Audio: flac, 96000 Hz, stereo, s24, 2847 kb/s",
        );

        assert_eq!(probe.sample_rate, Some(96_000));
        assert_eq!(probe.channels, Some(2));
        assert_eq!(probe.bitrate_kbps, Some(2847));
    }

    #[test]
    fn parses_ffmpeg_mono_and_surround_probe() {
        let mono = parse_ffmpeg_audio_probe(
            "Stream #0:0: Audio: pcm_s16le, 44100 Hz, mono, s16, 705 kb/s",
        );
        let surround = parse_ffmpeg_audio_probe(
            "Stream #0:0: Audio: alac, 48000 Hz, 5.1(side), s24p, 6912 kb/s",
        );

        assert_eq!(mono.channels, Some(1));
        assert_eq!(surround.channels, Some(6));
    }

    #[test]
    fn parses_container_bitrate_when_stream_bitrate_is_missing() {
        let probe = parse_ffmpeg_audio_probe(
            "Input #0, wav, from 'song.wav':\n  Duration: 00:01:00.00, bitrate: 1411 kb/s\n  Stream #0:0: Audio: pcm_s16le, 44100 Hz, stereo, s16",
        );

        assert_eq!(probe.sample_rate, Some(44_100));
        assert_eq!(probe.channels, Some(2));
        assert_eq!(probe.bitrate_kbps, Some(1411));
    }

    #[test]
    fn detects_attached_picture_in_ffmpeg_probe() {
        let probe = parse_ffmpeg_audio_probe(
            "Input #0, flac, from 'song.flac':\n  Metadata:\n    title           : demo\n  Stream #0:0: Audio: flac, 44100 Hz, stereo, s16\n  Stream #0:1: Video: png, rgb24(pc), 600x600, 90k tbr, 90k tbn (attached pic)",
        );

        assert!(probe.has_attached_pic);
        assert_eq!(probe.sample_rate, Some(44_100));
    }

    #[test]
    fn rewrites_uuid_paths_case_insensitively() {
        let rewritten = rewrite_uuid_in_path(
            "D:/PIONEER/Master/share/PIONEER/USBANLZ/A49/581BE-9886-4241-90C9-02B687C04804/ANLZ0000.DAT",
            "a49581be-9886-4241-90c9-02b687c04804",
            "11111111-2222-3333-4444-555555555555",
        );

        assert_eq!(
            rewritten,
            "D:/PIONEER/Master/share/PIONEER/USBANLZ/111/11111-2222-3333-4444-555555555555/ANLZ0000.DAT"
        );
    }

    #[test]
    fn rewrites_analysis_resource_paths_using_fallback_layout() {
        let rewritten = rewrite_analysis_resource_path(
            "D:/PIONEER/Master/share/PIONEER/USBANLZ/a49/581be-9886-4241-90c9-02b687c04804/ANLZ0000.2EX",
            "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
            None,
            "11111111-2222-3333-4444-555555555555",
        );

        assert_eq!(
            normalize_path_separators(&rewritten),
            normalize_path_separators(
                "D:/PIONEER/Master/share/PIONEER/USBANLZ/111/11111-2222-3333-4444-555555555555/ANLZ0000.2EX"
            )
        );
    }

    #[test]
    fn redirects_existing_target_file_names() {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        let source = dir.path().join("track.flac");
        fs::write(&source, b"source").expect("source fixture should be written");
        fs::write(dir.path().join("track.mp3"), b"existing").expect("existing target should exist");

        let spec = preset_spec("mp3-320").expect("preset should be valid");
        let redirected = build_target_path(&source, &spec, ConflictResolution::Redirect)
            .expect("redirected target path should be created");

        assert_eq!(
            redirected.file_name().and_then(|name| name.to_str()),
            Some("track (2).mp3")
        );
    }

    #[test]
    fn redirects_existing_archive_file_names() {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        let source = dir.path().join("track.flac");
        fs::write(&source, b"source").expect("source fixture should be written");
        fs::write(dir.path().join("track-1000kbps.flac"), b"existing")
            .expect("existing archive should exist");

        let redirected = build_source_archive_path(&source, 1000, ConflictResolution::Redirect)
            .expect("redirected archive path should be created");

        assert_eq!(
            redirected.file_name().and_then(|name| name.to_str()),
            Some("track-1000kbps (2).flac")
        );
    }

    #[test]
    #[ignore]
    fn migrate_real_master_db_track() {
        let db_path = "/Users/chuanpeng/Library/Pioneer/rekordbox/master.db".to_string();
        let scan = scan_impl(ScanRequest {
            db_path: db_path.clone(),
            min_bit_depth: 16,
            include_sampler: false,
        })
        .expect("scan should succeed");

        let track = scan
            .tracks
            .iter()
            .find(|candidate| candidate.id == "94106400")
            .cloned()
            .or_else(|| scan.tracks.first().cloned())
            .expect("expected at least one convertible track");

        let result = convert_impl_with_progress(
            ConvertRequest {
                db_path: db_path.clone(),
                preset: "mp3-320".to_string(),
                source_handling: "rename".to_string(),
                archive_conflict_resolution: None,
                output_conflict_resolution: None,
                tracks: vec![track.clone()],
            },
            |_| {},
        )
        .expect("conversion and migration should succeed");

        assert_eq!(result.converted_count, 1);
        let migrated = result
            .converted_tracks
            .first()
            .expect("expected one migrated track");
        assert_eq!(migrated.source_id.as_deref(), Some(track.id.as_str()));
        assert_ne!(migrated.id, track.id);
        assert_eq!(migrated.file_type, "MP3");

        let new_content_id = &migrated.id;
        let ordinary_playlist_count = sqlcipher_required_value(
      Path::new(&db_path),
      DEFAULT_KEY,
      &format!(
        "SELECT COUNT(*) FROM djmdSongPlaylist WHERE ContentID = {} AND PlaylistID IN (SELECT ID FROM djmdPlaylist WHERE COALESCE(SmartList, '') = '');",
        sql_quote(new_content_id)
      ),
      "expected playlist binding count",
    )
    .expect("playlist count query should succeed")
    .parse::<usize>()
    .expect("playlist count should parse");

        let old_playlist_count = sqlcipher_required_value(
      Path::new(&db_path),
      DEFAULT_KEY,
      &format!(
        "SELECT COUNT(*) FROM djmdSongPlaylist WHERE ContentID = {} AND PlaylistID IN (SELECT ID FROM djmdPlaylist WHERE COALESCE(SmartList, '') = '');",
        sql_quote(&track.id)
      ),
      "expected old playlist binding count",
    )
    .expect("old playlist count query should succeed")
    .parse::<usize>()
    .expect("old playlist count should parse");

        assert!(ordinary_playlist_count > 0);
        assert_eq!(old_playlist_count, 0);

        let old_content_exists = sqlcipher_required_value(
            Path::new(&db_path),
            DEFAULT_KEY,
            &format!(
                "SELECT COUNT(*) FROM djmdContent WHERE ID = {};",
                sql_quote(&track.id)
            ),
            "expected old content count",
        )
        .expect("old content count query should succeed")
        .parse::<usize>()
        .expect("old content count should parse");
        assert_eq!(old_content_exists, 0);

        let content_file_count = sqlcipher_required_value(
            Path::new(&db_path),
            DEFAULT_KEY,
            &format!(
                "SELECT COUNT(*) FROM contentFile WHERE ContentID = {};",
                sql_quote(new_content_id)
            ),
            "expected contentFile count",
        )
        .expect("contentFile count query should succeed")
        .parse::<usize>()
        .expect("contentFile count should parse");
        assert!(content_file_count > 0);
    }
}
