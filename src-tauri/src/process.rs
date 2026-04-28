use super::*;

fn process_output_contains(output: &[u8], needle: &str) -> bool {
    String::from_utf8_lossy(output).lines().any(|line| {
        let trimmed = line.trim();
        if trimmed.eq_ignore_ascii_case(needle) {
            return true;
        }

        Path::new(trimmed)
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case(needle))
    })
}

#[cfg(target_os = "windows")]
pub(super) fn rekordbox_process_running() -> Result<bool, String> {
    let output = hidden_windows_command("tasklist")
        .args(["/FI", "IMAGENAME eq rekordbox.exe"])
        .output()
        .map_err(|e| io_error_message("failed to check Windows process list", &e))?;
    Ok(output.status.success() && process_output_contains(&output.stdout, "rekordbox.exe"))
}

#[cfg(not(target_os = "windows"))]
pub(super) fn rekordbox_process_running() -> Result<bool, String> {
    match Command::new("pgrep").args(["-x", "rekordbox"]).status() {
        Ok(status) if status.success() => return Ok(true),
        Ok(_) => {}
        Err(_) => {}
    }

    let output = Command::new("ps")
        .args(["-axo", "comm="])
        .output()
        .map_err(|e| io_error_message("failed to check process list", &e))?;
    Ok(output.status.success() && process_output_contains(&output.stdout, "rekordbox"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_process_name_case_insensitively() {
        assert!(process_output_contains(
            b"/Applications/rekordbox.app/Contents/MacOS/rekordbox",
            "Rekordbox"
        ));
        assert!(!process_output_contains(
            b"/Applications/Other.app",
            "rekordbox"
        ));
        assert!(!process_output_contains(
            b"/Users/chuanpeng/Documents/rkb-lossless-process/target/debug/rekordport",
            "rekordbox"
        ));
        assert!(!process_output_contains(
            b"/Applications/rekordboxAgent.app/Contents/MacOS/rekordboxAgent",
            "rekordbox"
        ));
    }
}
