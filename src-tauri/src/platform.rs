fn refresh_command_discovery_caches() {
    if let Some(cache) = COMMAND_CACHE.get() {
        cache.lock().expect("command cache lock poisoned").clear();
    }
    if let Some(cache) = COMMAND_PATH_CACHE.get() {
        cache
            .lock()
            .expect("command path cache lock poisoned")
            .clear();
    }
    if let Some(cache) = ENCODER_CACHE.get() {
        cache.lock().expect("encoder cache lock poisoned").clear();
    }
}

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
