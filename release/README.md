# Release Artifacts

当前目录包含两个 macOS 测试包：

- `rekordbox lossless scan-macos-arm64.app`
  - 适用于 Apple Silicon Mac
  - 已内置 `ffmpeg` / `ffprobe` / `sqlcipher`
- `rekordbox lossless scan-macos-x86_64.app`
  - 适用于 Intel Mac
  - 已内置 `ffmpeg` / `ffprobe` / `sqlcipher`

注意：

- 这两个 `.app` 都是未签名、未 notarize 的测试构建。
- 第一次在别的 Mac 上打开时，可能需要右键选择 `Open`，或者到“系统设置 -> 隐私与安全性”里允许打开。
- Intel 包体更大，是因为打包进去了真正的 x86_64 sidecar 二进制。
