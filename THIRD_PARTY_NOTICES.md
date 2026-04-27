# Third-Party Notices

This repository is MIT-licensed, but some build helpers download or build
third-party tools that have their own licenses and redistribution terms.

## Windows Sidecar Sources

The helper script [`tools/fetch-windows-sidecars.ps1`](tools/fetch-windows-sidecars.ps1)
uses pinned upstream artifacts and verifies their SHA-256 digests by default.

| Component | Target | Source | Pinned object | SHA-256 |
| --- | --- | --- | --- | --- |
| FFmpeg | `x86_64-pc-windows-msvc` | `BtbN/FFmpeg-Builds` | `autobuild-2026-04-26-13-08 / ffmpeg-n8.1-10-g7f5c90f77e-win64-lgpl-8.1.zip` | `d2bcaee1792a39e2bfd2c04a3d88daf53d4e857a6583fed68c03562106f745bd` |
| FFmpeg | `aarch64-pc-windows-msvc` | `BtbN/FFmpeg-Builds` | `autobuild-2026-04-26-13-08 / ffmpeg-n8.1-10-g7f5c90f77e-winarm64-lgpl-8.1.zip` | `a29d83d01d3a07cfe060af439c803a082a508fd92c662a74d0ee946888ee4c1a` |
| SQLCipher | `x86_64-pc-windows-msvc` | `Katecca/sqlcipher-static-binary` | `commit b7cb2d5dc1b6baee00e153ffbac8c6703f89da88 / windows/x86_64/sqlcipher.exe` | `19f16d2629adedc6ddc2aeebd2da165d61aa0d645a61d2de373396c04ad0031f` |

The Windows SQLCipher binary is a third-party prebuilt artifact rather than an
official upstream release. The script keeps this reproducible by pinning both
the exact object and its SHA-256 digest, and it allows maintainers to override
the source only when a matching hash is provided.

## macOS Sidecar Sources

[`tools/build-macos13-arm64-sidecars.sh`](tools/build-macos13-arm64-sidecars.sh)
builds these tools from source:

- SQLCipher `v4.13.0`
- FFmpeg `8.1`
- LAME `3.100`

## Maintainer Notes

- Review upstream licenses before redistributing bundled binaries.
- Keep pinned URLs and SHA-256 digests in sync with release notes or workflow
  changes.
- Prefer exact versioned artifacts over `latest` URLs for CI and release builds.
