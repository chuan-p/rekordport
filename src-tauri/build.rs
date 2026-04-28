use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn expected_windows_sidecar_sha256(command: &str) -> Option<&'static str> {
    match command {
        "ffmpeg" => Some("0c807b2a284a1b4c6c8f609ba90da7a4e623362313b18b2fe98d2cdb1535ea28"),
        "sqlcipher" => Some("19f16d2629adedc6ddc2aeebd2da165d61aa0d645a61d2de373396c04ad0031f"),
        _ => None,
    }
}

fn verify_windows_sidecar(command: &str, path: &Path) {
    let expected = expected_windows_sidecar_sha256(command)
        .unwrap_or_else(|| panic!("missing pinned SHA-256 for Windows sidecar {command}"));
    let bytes = fs::read(path).unwrap_or_else(|error| {
        panic!("failed to read Windows sidecar {}: {error}", path.display())
    });
    let actual = format!("{:x}", Sha256::digest(&bytes));
    if actual != expected {
        panic!(
            "refusing to embed Windows sidecar {}: SHA-256 mismatch for {command}. expected={expected} actual={actual}",
            path.display()
        );
    }
}

fn windows_sidecar_match_arm(command: &str, absolute_path: &Path) -> String {
    format!(
        "\"{command}\" => Some(include_bytes!(r#\"{}\"#) as &'static [u8]),",
        absolute_path.display()
    )
}

fn generate_embedded_windows_sidecars() {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR should exist"));
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR should exist"));
    let generated_path = out_dir.join("embedded_windows_sidecars.rs");
    let sidecars = [
        (
            "ffmpeg",
            manifest_dir.join("bin/ffmpeg-x86_64-pc-windows-msvc.exe"),
        ),
        (
            "sqlcipher",
            manifest_dir.join("bin/sqlcipher-x86_64-pc-windows-msvc.exe"),
        ),
    ];

    for (_, path) in &sidecars {
        println!("cargo:rerun-if-changed={}", path.display());
    }

    let match_arms = sidecars
        .iter()
        .filter_map(|(command, path)| {
            if path.exists() {
                if target_os == "windows" {
                    verify_windows_sidecar(command, path);
                }
                Some(windows_sidecar_match_arm(
                    command,
                    &path.canonicalize().unwrap_or_else(|_| path.to_path_buf()),
                ))
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n            ");

    let generated = format!(
        r#"#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
fn embedded_windows_sidecar_bytes(command: &str) -> Option<&'static [u8]> {{
    match command {{
            {match_arms}
        _ => None,
    }}
}}

#[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
fn embedded_windows_sidecar_bytes(_command: &str) -> Option<&'static [u8]> {{
    None
}}
"#
    );

    fs::write(&generated_path, generated)
        .expect("generated embedded_windows_sidecars.rs should be written");
}

fn main() {
    generate_embedded_windows_sidecars();
    tauri_build::build()
}
