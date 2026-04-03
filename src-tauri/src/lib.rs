use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::fs::File;
use std::io::Read;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::Emitter;
use tempfile::Builder as TempBuilder;
use uuid::Uuid;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const DEFAULT_KEY: &str = "402fd482c38817c35ffa8ffb8c7d93143b749e7d315df7a81732a1ff43608497";

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
    tracks: Vec<Track>,
}

#[derive(Debug, Deserialize)]
struct PreflightRequest {
    #[serde(rename = "dbPath")]
    db_path: Option<String>,
}

#[derive(Debug, Serialize)]
struct ScanSummary {
    total: usize,
    flac: usize,
    hi_res: usize,
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
    ffprobe_available: bool,
    sqlcipher_source: Option<String>,
    ffmpeg_source: Option<String>,
    ffprobe_source: Option<String>,
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
    hash: Option<String>,
    size: Option<u64>,
}

#[derive(Debug)]
struct MigratedContentFile {
    original: ContentFileRef,
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

#[derive(Debug, Clone, Copy)]
enum SourceHandling {
    Rename,
    Trash,
}

static COMMAND_CACHE: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
static COMMAND_PATH_CACHE: OnceLock<Mutex<HashMap<String, Option<PathBuf>>>> = OnceLock::new();
static ENCODER_CACHE: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();

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

fn executable_filename(command: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("{command}.exe")
    } else {
        command.to_string()
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

fn backup_relative_path(path: &Path) -> PathBuf {
    let mut rel = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => rel.push(prefix.as_os_str()),
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

fn probe_codec_name(path: &Path) -> Result<Option<String>, String> {
    if !command_available("ffprobe") {
        return Ok(None);
    }

    let mut ffprobe = prepared_command("ffprobe")?;
    ffprobe.args([
        "-v",
        "error",
        "-select_streams",
        "a:0",
        "-show_entries",
        "stream=codec_name",
        "-of",
        "default=noprint_wrappers=1:nokey=1",
    ]);
    ffprobe.arg(path);
    let output = ffprobe.output().map_err(|e| e.to_string())?;

    if !output.status.success() {
        return Ok(None);
    }

    let codec = String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_lowercase();
    if codec.is_empty() {
        Ok(None)
    } else {
        Ok(Some(codec))
    }
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
    let output = ffmpeg.output().map_err(|e| e.to_string())?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let available = stdout.lines().any(|line| line.contains(name));
    let mut guard = cache.lock().expect("encoder cache lock poisoned");
    guard.insert(name.to_string(), available);
    Ok(available)
}

fn probe_channels(path: &Path) -> Result<u32, String> {
    if !command_available("ffprobe") {
        return Ok(2);
    }

    let mut ffprobe = prepared_command("ffprobe")?;
    ffprobe.args([
        "-v",
        "error",
        "-select_streams",
        "a:0",
        "-show_entries",
        "stream=channels",
        "-of",
        "default=noprint_wrappers=1:nokey=1",
    ]);
    ffprobe.arg(path);
    let output = ffprobe.output().map_err(|e| e.to_string())?;

    if !output.status.success() {
        return Ok(2);
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(stdout.parse::<u32>().unwrap_or(2))
}

fn probe_sample_rate(path: &Path) -> Result<Option<u32>, String> {
    if !command_available("ffprobe") {
        return Ok(None);
    }

    let mut ffprobe = prepared_command("ffprobe")?;
    ffprobe.args([
        "-v",
        "error",
        "-select_streams",
        "a:0",
        "-show_entries",
        "stream=sample_rate",
        "-of",
        "default=noprint_wrappers=1:nokey=1",
    ]);
    ffprobe.arg(path);
    let output = ffprobe.output().map_err(|e| e.to_string())?;

    if !output.status.success() {
        return Ok(None);
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() {
        return Ok(None);
    }

    Ok(stdout.parse::<u32>().ok())
}

fn probe_bitrate(path: &Path) -> Result<Option<u32>, String> {
    if !command_available("ffprobe") {
        return Ok(None);
    }

    let mut ffprobe = prepared_command("ffprobe")?;
    ffprobe.args([
        "-v",
        "error",
        "-show_entries",
        "format=bit_rate",
        "-of",
        "default=noprint_wrappers=1:nokey=1",
    ]);
    ffprobe.arg(path);
    let output = ffprobe.output().map_err(|e| e.to_string())?;

    if !output.status.success() {
        return Ok(None);
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() {
        return Ok(None);
    }

    Ok(stdout.parse::<u32>().ok().map(|value| value / 1000))
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
        .map_err(|e| e.to_string())?;

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

fn build_scan_query(min_bit_depth: u32, include_sampler: bool) -> String {
    let sampler_filter = if include_sampler {
        String::new()
    } else {
        "\n  AND COALESCE(c.FolderPath, '') NOT LIKE '%/Sampler/%'".to_string()
    };

    format!(
    ".headers on\n.mode csv\nSELECT\n  COALESCE(c.ID, '') AS id,\n  COALESCE(c.Title, '') AS title,\n  COALESCE(a.Name, c.SrcArtistName, '') AS artist,\n  c.FileType AS file_type,\n  c.BitDepth AS bit_depth,\n  c.SampleRate AS sample_rate,\n  c.BitRate AS bitrate,\n  COALESCE(c.FolderPath, '') AS full_path\nFROM djmdContent c\nLEFT JOIN djmdArtist a ON a.ID = c.ArtistID\nWHERE\n  (\n    c.FileType = 5\n    OR c.FileType = 4\n    OR (c.FileType IN (11, 12) AND COALESCE(c.BitDepth, 0) > {min_bit_depth})\n  ){sampler_filter}\nORDER BY\n  artist COLLATE NOCASE,\n  title COLLATE NOCASE,\n  full_path COLLATE NOCASE;"
  )
}

fn scan_rows(
    db_path: &Path,
    key: &str,
    min_bit_depth: u32,
    include_sampler: bool,
) -> Result<Vec<ScanRow>, String> {
    if !db_path.exists() {
        return Err(format!("database file not found: {}", db_path.display()));
    }
    if !command_available("sqlcipher") {
        return Err("sqlcipher command not found in PATH or bundled sidecar".into());
    }
    if !command_available("ffprobe") {
        return Err(
            "ffprobe command not found in PATH or bundled sidecar (required to detect ALAC files)"
                .into(),
        );
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
) -> Result<Vec<Track>, String>
where
    F: FnMut(ScanProgressPayload),
{
    let rows = scan_rows(db_path, key, min_bit_depth, include_sampler)?;
    let total = rows.len();
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
        let mut codec_name = None;
        if row.file_type == 4 && !row.full_path.is_empty() {
            codec_name = probe_codec_name(Path::new(&row.full_path))?;
            if codec_name.as_deref() != Some("alac") {
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
        }

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

    Ok(tracks)
}

fn scan_tracks(
    db_path: &Path,
    key: &str,
    min_bit_depth: u32,
    include_sampler: bool,
) -> Result<Vec<Track>, String> {
    scan_tracks_with_progress(db_path, key, min_bit_depth, include_sampler, |_| {})
}

fn rewrite_uuid_in_path(path: &str, old_uuid: &str, new_uuid: &str) -> String {
    let old_split = format!("{}/{}", &old_uuid[..3], &old_uuid[3..]);
    let new_split = format!("{}/{}", &new_uuid[..3], &new_uuid[3..]);
    let old_split_encoded = format!("{}%2F{}", &old_uuid[..3], &old_uuid[3..]);
    let new_split_encoded = format!("{}%2F{}", &new_uuid[..3], &new_uuid[3..]);

    path.replace(old_uuid, new_uuid)
        .replace(&old_split, &new_split)
        .replace(&old_split_encoded, &new_split_encoded)
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
        let _uuid = parts.next().unwrap_or_default().to_string();
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
            hash: if hash.is_empty() { None } else { Some(hash) },
            size: size.parse::<u64>().ok(),
        });
    }
    Ok(files)
}

fn copy_file_with_parent_dirs(source: &Path, destination: &Path) -> Result<(), String> {
    if !source.exists() {
        return Err(format!("source resource not found: {}", source.display()));
    }
    if source == destination {
        return Err(format!(
            "refusing to copy analysis resource onto itself: {}",
            source.display()
        ));
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::copy(source, destination).map_err(|e| e.to_string())?;
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
    let mut bytes = fs::read(path).map_err(|e| e.to_string())?;
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

    fs::write(path, bytes).map_err(|e| e.to_string())
}

fn md5_hex(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut context = md5::Context::new();
    let mut buffer = [0_u8; 16 * 1024];

    loop {
        let bytes_read = file.read(&mut buffer).map_err(|e| e.to_string())?;
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
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
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

    fs::write(path, content).map_err(|e| e.to_string())
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
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
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

    let file = File::create(path).map_err(|e| e.to_string())?;
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
        if !source.exists() {
            return Err(format!("analysis resource missing: {}", source.display()));
        }

        let metadata = fs::metadata(&source).map_err(|e| e.to_string())?;
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
                hash: file.hash.clone(),
                size: file.size,
            },
            source,
        });
    }

    Ok(validated)
}

fn collect_anlz_dat_paths(base: &Path, paths: &mut Vec<PathBuf>) -> Result<(), String> {
    if !base.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(base).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.is_dir() {
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
    if !analysis_root.exists() {
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
            .filter_map(|path| fs::metadata(path).ok().map(|meta| meta.len()))
            .collect();

        if !existing_sizes.is_empty() && existing_sizes.iter().all(|size| *size == 0) {
            orphan_dirs.push(dir);
        }
    }

    if orphan_dirs.is_empty() {
        return Ok(CleanupReport::default());
    }

    let archive_root = rekordbox_root.join(format!("anlz-orphan-cleanup-{}", timestamp_token()));
    fs::create_dir_all(&archive_root).map_err(|e| e.to_string())?;

    for dir in &orphan_dirs {
        let relative = dir
            .strip_prefix(&analysis_root)
            .map_err(|e| e.to_string())?;
        let target = archive_root.join(relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::rename(dir, &target).map_err(|e| e.to_string())?;
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

fn preflight_impl(req: PreflightRequest) -> PreflightResponse {
    let db_path = req
        .db_path
        .filter(|value| !value.trim().is_empty())
        .or_else(default_database_path_value)
        .unwrap_or_default();
    let db_path_buf = PathBuf::from(&db_path);
    let sqlcipher_available = command_available("sqlcipher");
    let ffmpeg_available = command_available("ffmpeg");
    let ffprobe_available = command_available("ffprobe");
    let sqlcipher_source = command_source("sqlcipher");
    let ffmpeg_source = command_source("ffmpeg");
    let ffprobe_source = command_source("ffprobe");
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
    if !ffprobe_available {
        warnings.push("ffprobe was not found, so ALAC detection and some audio probing will fail. Add a bundled sidecar in src-tauri/bin or install it in the system PATH.".to_string());
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

    let scan_ready = sqlcipher_available && ffprobe_available && db_readable;
    let convert_ready = ffmpeg_available && sqlcipher_available && db_readable;

    PreflightResponse {
        os: platform_name(),
        sqlcipher_available,
        ffmpeg_available,
        ffprobe_available,
        sqlcipher_source,
        ffmpeg_source,
        ffprobe_source,
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
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::copy(source, &target).map_err(|e| e.to_string())?;
    Ok(target)
}

fn build_target_path(source: &Path, spec: &ConversionSpec) -> Result<PathBuf, String> {
    let parent = source
        .parent()
        .ok_or_else(|| format!("missing parent directory for {}", source.display()))?;
    let stem = source
        .file_stem()
        .ok_or_else(|| format!("missing file stem for {}", source.display()))?
        .to_string_lossy();
    Ok(parent.join(format!("{stem}.{}", spec.extension)))
}

fn build_source_archive_path(source: &Path, bitrate_kbps: u32) -> Result<PathBuf, String> {
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
    if candidate.exists() {
        return Err(format!(
            "source archive already exists, refusing to overwrite: {}",
            candidate.display()
        ));
    }
    Ok(candidate)
}

fn convert_one_track(
    track: &Track,
    spec: &ConversionSpec,
    backup_root: &Path,
) -> Result<(Track, PathBuf, PathBuf), String> {
    let source = Path::new(&track.full_path);
    if !source.exists() {
        return Err(format!("source file not found: {}", source.display()));
    }

    let source_sample_rate = probe_sample_rate(source)?.or(track.sample_rate);
    let target_sample_rate = target_sample_rate_for_source(source_sample_rate);
    let source_bitrate = source_bitrate_kbps(track, source)?;
    let archive_path = build_source_archive_path(source, source_bitrate)?;
    let output_path = build_target_path(source, spec)?;

    backup_file_tree(source, backup_root)?;

    fs::rename(source, &archive_path).map_err(|e| e.to_string())?;

    let output_parent = output_path
        .parent()
        .ok_or_else(|| format!("missing output parent for {}", output_path.display()))?;
    let temp_output = TempBuilder::new()
        .prefix(".rkb-lossless-")
        .suffix(&format!(".{}", spec.extension))
        .tempfile_in(output_parent)
        .map_err(|e| e.to_string())?;

    let conversion_result = (|| -> Result<(), String> {
        let mut ffmpeg = prepared_command("ffmpeg")?;
        ffmpeg.args(["-hide_banner", "-loglevel", "error", "-y", "-i"]);
        ffmpeg.arg(&archive_path);
        ffmpeg.args(["-vn", "-map_metadata", "0", "-c:a", spec.ffmpeg_codec]);
        if spec.extension == "wav" || spec.extension == "aiff" || spec.extension == "m4a" {
            ffmpeg.args(["-ar", &target_sample_rate.to_string()]);
        }
        if let Some(bitrate) = spec.bitrate_kbps {
            ffmpeg.args(["-b:a", &format!("{bitrate}k")]);
        }
        if spec.extension == "m4a" {
            ffmpeg.args(["-movflags", "+faststart"]);
        }
        ffmpeg.arg(temp_output.path());

        let output = ffmpeg.output().map_err(|e| e.to_string())?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            return Err(if !stderr.is_empty() { stderr } else { stdout });
        }

        Ok(())
    })();

    if let Err(error) = conversion_result {
        let _ = fs::rename(&archive_path, source);
        return Err(error);
    }

    if output_path.exists() {
        let _ = fs::remove_file(temp_output.path());
        let _ = fs::rename(&archive_path, source);
        return Err(format!(
            "target file already exists: {}",
            output_path.display()
        ));
    }

    let persisted = temp_output
        .persist(&output_path)
        .map_err(|e| e.error.to_string())?;
    drop(persisted);

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
            let file_size = fs::metadata(output_path).map_err(|e| e.to_string())?.len();
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
                        let destination = PathBuf::from(rewrite_uuid_in_path(
                            source_path,
                            &old_uuid,
                            &content_uuid,
                        ));
                        copy_file_with_parent_dirs(&file.source, &destination)?;
                        rewrite_anlz_ppth(&destination, &file_name)?;
                        copied_resources.push(destination.clone());
                        let size = fs::metadata(&destination).map_err(|e| e.to_string())?.len();
                        let hash = md5_hex(&destination)?;
                        let new_path =
                            rewrite_uuid_in_path(&file.original.path, &old_uuid, &content_uuid);
                        let new_local_path = file
                            .original
                            .rb_local_path
                            .as_ref()
                            .map(|path| rewrite_uuid_in_path(path, &old_uuid, &content_uuid));
                        migrated_content_files.push(MigratedContentFile {
                            original: file.original,
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
                rewrite_uuid_in_path(&old_analysis_path, &old_uuid, &content_uuid)
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
          "UPDATE contentFile SET ID = {}, ContentID = {new_content_id_expr}, Path = {}, rb_local_path = {}, Hash = {}, Size = {}, updated_at = {now_expr} WHERE ID = {} AND ContentID = {};\n",
          sql_quote(&rewrite_uuid_in_path(&file.original.id, &old_uuid, &content_uuid)),
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
            let _ = fs::remove_file(path);
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
    let db_path = PathBuf::from(&req.db_path);
    if !db_path.exists() {
        return Err(format!("database file not found: {}", db_path.display()));
    }

    if spec.extension == "m4a" && !ffmpeg_has_encoder("aac_at")? {
        return Err(
            "ffmpeg was built without Apple's aac_at encoder, so M4A 320kbps is unavailable".into(),
        );
    }

    let timestamp = timestamp_token();
    let backup_root = db_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!("rkb-lossless-backup-{timestamp}"));
    fs::create_dir_all(&backup_root).map_err(|e| e.to_string())?;

    let db_backup = backup_root.join("master.db");
    fs::copy(&db_path, &db_backup).map_err(|e| e.to_string())?;

    let music_backup_root = backup_root.join("music");
    let mut converted_tracks: Vec<Track> = Vec::with_capacity(req.tracks.len());
    let mut archive_paths: Vec<PathBuf> = Vec::new();
    let mut output_paths: Vec<PathBuf> = Vec::new();
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
        match convert_one_track(track, &spec, &music_backup_root) {
            Ok((converted_track, output_path, archive_path)) => {
                archive_paths.push(archive_path);
                output_paths.push(output_path);
                converted_tracks.push(converted_track);
            }
            Err(error) => {
                for path in output_paths {
                    let _ = fs::remove_file(path);
                }
                for archive_path in archive_paths {
                    let source_path = req
                        .tracks
                        .iter()
                        .find(|candidate| {
                            let current = Path::new(&candidate.full_path);
                            if let (Some(stem), Some(ext)) =
                                (current.file_stem(), current.extension())
                            {
                                archive_path.file_name().is_some_and(|name| {
                                    let expected_prefix = format!("{}-", stem.to_string_lossy());
                                    let expected_suffix = format!(".{}", ext.to_string_lossy());
                                    let file_name = name.to_string_lossy();
                                    file_name.starts_with(&expected_prefix)
                                        && file_name.ends_with(&expected_suffix)
                                })
                            } else {
                                false
                            }
                        })
                        .map(|candidate| PathBuf::from(&candidate.full_path))
                        .unwrap_or_else(|| archive_path.clone());
                    let _ = fs::rename(archive_path, source_path);
                }
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

    let migrated_tracks =
        match migrate_tracks_in_db(&db_path, &req.tracks, &converted_tracks, DEFAULT_KEY, &spec) {
            Ok(tracks) => tracks,
            Err(error) => {
                for output_path in &output_paths {
                    let _ = fs::remove_file(output_path);
                }
                for (track, archive_path) in req.tracks.iter().zip(archive_paths.iter()) {
                    let source_path = PathBuf::from(&track.full_path);
                    let _ = fs::rename(archive_path, source_path);
                }
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
        for archive_path in &archive_paths {
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

    let tracks = scan_tracks_with_progress(
        Path::new(&req.db_path),
        DEFAULT_KEY,
        req.min_bit_depth,
        req.include_sampler,
        |payload| on_progress(payload),
    )?;
    let flac = tracks
        .iter()
        .filter(|track| track.file_type == "FLAC")
        .count();
    let hi_res = tracks
        .iter()
        .filter(|track| matches!(track.file_type.as_str(), "WAV" | "AIFF"))
        .count();

    let response = ScanResponse {
        summary: ScanSummary {
            total: tracks.len(),
            flac,
            hi_res,
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
            format!("Scan complete. Found {} results.", response.summary.total)
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
fn open_path_in_finder(path: String) -> Result<(), String> {
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
        return Err(format!("failed to open path in Finder: {}", path.display()));
    }

    #[cfg(target_os = "windows")]
    {
        let mut command = Command::new("explorer");
        if path.is_dir() {
            command.arg(&path);
        } else {
            command.arg(format!("/select,{}", path.display().to_string()));
        }

        let status = command.status().map_err(|e| e.to_string())?;
        if status.success() {
            return Ok(());
        }
        return Err(format!(
            "failed to open path in Explorer: {}",
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

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            default_database_path,
            preflight_check,
            pick_database_path,
            pick_export_path,
            open_path_in_finder,
            scan_library,
            export_tracks,
            convert_tracks
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

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

        let result = convert_impl(ConvertRequest {
            db_path: db_path.clone(),
            preset: "mp3-320".to_string(),
            source_handling: "rename".to_string(),
            tracks: vec![track.clone()],
        })
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
