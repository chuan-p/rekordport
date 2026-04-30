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

fn parse_ffmpeg_duration_seconds(text: &str) -> Option<f64> {
    let (_, rest) = text.split_once("Duration:")?;
    let value = rest.split(',').next()?.trim();
    if value.eq_ignore_ascii_case("N/A") {
        return None;
    }

    let mut parts = value.split(':');
    let hours = parts.next()?.parse::<f64>().ok()?;
    let minutes = parts.next()?.parse::<f64>().ok()?;
    let seconds = parts.next()?.parse::<f64>().ok()?;
    if parts.next().is_some() {
        return None;
    }

    let duration = (hours * 3600.0) + (minutes * 60.0) + seconds;
    (duration > 0.0).then_some(duration)
}

fn bitrate_kbps_from_size_and_duration(file_len: u64, duration_seconds: f64) -> Option<u32> {
    if file_len == 0 || !duration_seconds.is_finite() || duration_seconds <= 0.0 {
        return None;
    }

    let bitrate = (((file_len as f64) * 8.0) / duration_seconds / 1000.0).round();
    if bitrate > 0.0 && bitrate <= u32::MAX as f64 {
        Some(bitrate as u32)
    } else {
        None
    }
}

fn fill_audio_probe_bitrate_from_file_size(probe: &mut AudioProbe, file_len: u64) {
    if probe.bitrate_kbps.is_some() {
        return;
    }

    probe.bitrate_kbps = probe
        .duration_seconds
        .and_then(|duration| bitrate_kbps_from_size_and_duration(file_len, duration));
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
        duration_seconds: parse_ffmpeg_duration_seconds(text),
        has_attached_pic: text.lines().any(|line| {
            let lower = line.to_ascii_lowercase();
            lower.contains("video:") && lower.contains("attached pic")
        }),
    }
}

fn audio_probe_cache_signature(path: &Path) -> Result<AudioProbeCacheSignature, String> {
    let metadata = metadata_path(path)?;
    Ok(AudioProbeCacheSignature {
        len: metadata.len(),
        modified: metadata.modified().ok(),
    })
}

fn probe_audio(path: &Path) -> Result<AudioProbe, String> {
    if !command_available("ffmpeg") {
        return Ok(AudioProbe::default());
    }

    let signature = audio_probe_cache_signature(path)?;
    let cache_key = path.to_path_buf();
    let cache = AUDIO_PROBE_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    {
        let guard = cache.lock().expect("audio probe cache lock poisoned");
        if let Some(entry) = guard.get(&cache_key) {
            if entry.signature == signature {
                return Ok(entry.probe.clone());
            }
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
    let mut probe = parse_ffmpeg_audio_probe(&text);
    fill_audio_probe_bitrate_from_file_size(&mut probe, signature.len);

    let mut guard = cache.lock().expect("audio probe cache lock poisoned");
    guard.insert(
        cache_key,
        AudioProbeCacheEntry {
            signature,
            probe: probe.clone(),
        },
    );
    Ok(probe)
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

fn source_bitrate_kbps(track: &Track, source_probe: &AudioProbe) -> u32 {
    if let Some(value) = track.bitrate {
        if value > 0 {
            return value;
        }
    }

    if matches!(track.file_type.as_str(), "WAV" | "AIFF") {
        let sample_rate = source_probe
            .sample_rate
            .or(track.sample_rate)
            .unwrap_or(44_100);
        let channels = source_probe.channels.unwrap_or(2);
        let bit_depth = track.bit_depth.unwrap_or(16);
        return compute_pcm_bitrate(sample_rate, channels, bit_depth);
    }

    if let Some(value) = source_probe.bitrate_kbps {
        return value;
    }

    track.bitrate.unwrap_or(0)
}
