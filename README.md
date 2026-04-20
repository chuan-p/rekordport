# rekordport

<p align="center">
  <img src="src-tauri/icons/128x128.png" alt="rekordport icon" width="128" />
</p>

rekordport is a tool for scanning a rekordbox library, finding hi-res files CDJs don't support, and converting selected entries without losing rekordbox metadata. Made by [chuan](https://www.instagram.com/chuan_p/) with heavy use of Codex.

<p align="center">
  <img src="docs/assets/readme-scan-preview.png" alt="rekordport desktop screenshot" />
</p>

## What It Does

- Scans `master.db` for:
  - FLAC
  - ALAC
  - WAV files with `WAVE_FORMAT_EXTENSIBLE` headers that some CDJ/XDJ units reject
  - WAV / AIFF above 16-bit or above 48kHz
- Lets you preview results in a desktop UI
- Converts selected tracks to `WAV`, `AIFF`, `MP3 320kbps`, or `M4A 320kbps`
- Backs up the source file and database before conversion
- Rewrites rekordbox paths and rebinds standard playlists to the new entry


## Safety Notes

- `rekordport` modifies your rekordbox database during conversion.
- The app creates backups first, but you should still keep your own library backup.
- Smart playlists are not rewritten yet.
- `MP3` and `M4A` now preserve embedded cover art when the source contains attached artwork.
- `AIFF` attempts to preserve embedded artwork on a best-effort basis.
- `WAV` output does not preserve embedded cover art.

## Platform Status

| Platform | Status |
| --- | --- |
| macOS | Primary development target |
| Windows | Supported via the shared Rust backend and GitHub Actions build |

## Quick Start

Install frontend dependencies:

```bash
npm install
```

Run the desktop app in development mode:

```bash
npm run tauri dev
```

The desktop app requires one of these setups:

- Rust toolchain installed locally
- `sqlcipher` and `ffmpeg` available in your system `PATH`
- Or platform sidecars placed in `src-tauri/bin`

## CLI

The original Python scanner is still available:

```bash
python3 rekordbox_lossless_scan.py --format table
python3 rekordbox_lossless_scan.py --format csv
python3 rekordbox_lossless_scan.py --output report.xlsx
python3 rekordbox_lossless_scan.py --include-sampler
python3 rekordbox_lossless_scan.py --min-bit-depth 24
```

## Build And Checks

Run the standard project checks:

```bash
npm run check
```

Build the frontend:

```bash
npm run build
```

Build the desktop app:

```bash
npm run tauri build
```

## Dependencies

The app looks for conversion dependencies in this order:

1. Environment variable overrides
   - `RKB_SQLCIPHER_PATH`
   - `RKB_FFMPEG_PATH`
2. Bundled sidecars in `src-tauri/bin`
3. System `PATH`

See [src-tauri/bin/README.md](src-tauri/bin/README.md) for sidecar naming and packaging notes.

## Open Source Project Health

- Contribution guide: [CONTRIBUTING.md](CONTRIBUTING.md)
- Code of conduct: [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)
- Security reporting: [SECURITY.md](SECURITY.md)
- Third-party build notices: [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)

## Format Notes

- `M4A 320kbps` requires the Apple `aac_at` encoder and is usually only available on macOS.
- The app uses `ffmpeg` for probing, preview generation, and conversion.
- The app uses `sqlcipher` for direct rekordbox database access.
- Some otherwise compatible `WAV` files still fail on standalone CDJ/XDJ units because they use a `WAVE_FORMAT_EXTENSIBLE` header instead of standard PCM. rekordport now flags those files during scan so you can convert them into a safer output.

## Current Limitations

- Smart playlist migration is not implemented yet.
- rekordbox USB export behavior may still keep old files depending on rekordbox's own sync rules.
- If analysis resources are already broken or inconsistent, conversion stops instead of trying to guess.
- `WAV` cover art is not preserved.

## Windows Build

There is a Windows portable build workflow in [.github/workflows/windows-build.yml](.github/workflows/windows-build.yml).

Local Windows build:

```bash
npm ci
npm run tauri build -- --bundles nsis
```

The helper script [tools/fetch-windows-sidecars.ps1](tools/fetch-windows-sidecars.ps1) downloads pinned Windows sidecars for `ffmpeg` and `sqlcipher` and verifies SHA-256 digests by default.

If you override the upstream URLs, also provide matching hashes:

```powershell
$env:RKB_FFMPEG_WINDOWS_URL = "https://example.invalid/ffmpeg.zip"
$env:RKB_FFMPEG_WINDOWS_SHA256 = "<sha256>"
$env:RKB_SQLCIPHER_WINDOWS_URL = "https://example.invalid/sqlcipher.exe"
$env:RKB_SQLCIPHER_WINDOWS_SHA256 = "<sha256>"
./tools/fetch-windows-sidecars.ps1
```

See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) for the currently pinned artifacts.

## Repository Layout

- `src/`: frontend UI
- `src-tauri/`: Rust backend and packaging config
- `rekordbox_lossless_scan.py`: original CLI scanner
- `tools/`: helper scripts for local builds and sidecar preparation

## Contributing

Small bug reports, platform notes, and focused pull requests are welcome. Start with [CONTRIBUTING.md](CONTRIBUTING.md), use the GitHub issue templates for bugs and feature requests, and check [SECURITY.md](SECURITY.md) before reporting sensitive issues.

## License

[MIT](LICENSE)
