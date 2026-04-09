use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn windows_sidecar_match_arm(command: &str, absolute_path: &Path) -> String {
    format!(
        "\"{command}\" => Some(include_bytes!(r#\"{}\"#) as &'static [u8]),",
        absolute_path.display()
    )
}

fn generate_embedded_windows_sidecars() {
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
