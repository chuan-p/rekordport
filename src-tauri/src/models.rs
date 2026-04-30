const DEFAULT_KEY: &str = "402fd482c38817c35ffa8ffb8c7d93143b749e7d315df7a81732a1ff43608497";
const LATEST_RELEASE_URL: &str = "https://github.com/chuan-p/rekordport/releases/latest";
const UPDATE_CHECK_TIMEOUT: Duration = Duration::from_secs(10);
const PREVIEW_CACHE_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const PREVIEW_CACHE_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const HI_RES_SAMPLE_RATE_THRESHOLD: u32 = 48_000;
#[cfg(test)]
const WAV_FORMAT_TAG_PCM: u16 = 0x0001;
const WAV_FORMAT_TAG_EXTENSIBLE: u16 = 0xFFFE;
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
    #[serde(rename = "operationId", default)]
    operation_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct Track {
    id: String,
    #[serde(default)]
    source_id: Option<String>,
    #[serde(default)]
    scan_issue: Option<String>,
    #[serde(default)]
    scan_note: Option<String>,
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
    #[serde(rename = "operationId", default)]
    operation_id: Option<String>,
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
    wav_extensible: usize,
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

#[derive(Debug, Serialize, Clone)]
struct ProgressEventPayload {
    #[serde(rename = "operationId", skip_serializing_if = "Option::is_none")]
    operation_id: Option<String>,
    phase: String,
    current: usize,
    total: usize,
    message: String,
}

impl ProgressEventPayload {
    fn new(operation_id: Option<String>, payload: ScanProgressPayload) -> Self {
        Self {
            operation_id,
            phase: payload.phase,
            current: payload.current,
            total: payload.total,
            message: payload.message,
        }
    }
}

#[derive(Debug, Serialize)]
struct ConvertResponse {
    backup_dir: String,
    converted_count: usize,
    analysis_migrated_count: usize,
    analysis_missing_count: usize,
    verification_playlist_name: Option<String>,
    source_cleanup_mode: String,
    source_cleanup_failures: usize,
    cleanup_archived_dirs: usize,
    cleanup_archive_dir: Option<String>,
    warnings: Vec<String>,
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
    png_encoder_available: bool,
    db_path: String,
    db_exists: bool,
    db_readable: bool,
    scan_ready: bool,
    convert_ready: bool,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct LatestReleaseResponse {
    tag_name: String,
    html_url: String,
    changelog: Option<String>,
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
struct TrackMigrationSourceData {
    old_uuid: String,
    old_analysis_path: String,
    content_files: Vec<ContentFileRef>,
}

#[derive(Debug, Default)]
struct TrackMigrationSourceDataBuilder {
    old_uuid: Option<String>,
    old_analysis_path: Option<String>,
    content_files: Vec<ContentFileRef>,
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

#[derive(Debug, Clone)]
struct ContentCueRewrite {
    old_content_id: String,
    old_content_uuid: String,
    new_content_id: String,
    new_content_uuid: String,
    offset_ms: u32,
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
    warnings: Vec<String>,
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
    file_name_l: String,
    file_name_s: String,
}

#[derive(Debug, Default)]
struct ScanStats {
    candidate_total: usize,
    wav_extensible: usize,
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
    duration_seconds: Option<f64>,
    has_attached_pic: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AudioProbeCacheSignature {
    len: u64,
    modified: Option<SystemTime>,
}

#[derive(Debug, Clone)]
struct AudioProbeCacheEntry {
    signature: AudioProbeCacheSignature,
    probe: AudioProbe,
}

static COMMAND_CACHE: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
static COMMAND_PATH_CACHE: OnceLock<Mutex<HashMap<String, Option<PathBuf>>>> = OnceLock::new();
static ENCODER_CACHE: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
static AUDIO_PROBE_CACHE: OnceLock<Mutex<HashMap<PathBuf, AudioProbeCacheEntry>>> = OnceLock::new();
static CONVERSION_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

struct DatabaseConversionLock {
    path: PathBuf,
}

impl Drop for DatabaseConversionLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
static TIMESTAMP_COUNTER: AtomicU64 = AtomicU64::new(0);
