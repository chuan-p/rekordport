# Sidecar binaries

把需要随应用一起打包的命令行工具放在这个目录里。

这里放的是“源 sidecar”，不是最终直接进安装包的文件。构建脚本会先把它们复制到 `src-tauri/gen/`：

- 所有平台都会先解引用成真实文件，避免把符号链接直接塞进包里
- macOS 会额外递归收集非系统 `.dylib` 依赖，并改写可执行文件的加载路径

当前项目会查找这两个工具：

- `sqlcipher`
- `ffmpeg`

Tauri `externalBin` 已经配置为：

- `bin/sqlcipher`
- `bin/ffmpeg`

所以源文件名需要带上目标平台 triple：

## macOS Apple Silicon

- `sqlcipher-aarch64-apple-darwin`
- `ffmpeg-aarch64-apple-darwin`

## macOS Intel

- `sqlcipher-x86_64-apple-darwin`
- `ffmpeg-x86_64-apple-darwin`

## Windows x64

- `sqlcipher-x86_64-pc-windows-msvc.exe`
- `ffmpeg-x86_64-pc-windows-msvc.exe`

## 运行时优先级

应用会按这个顺序查找工具：

1. 环境变量覆盖
   - `RKB_SQLCIPHER_PATH`
   - `RKB_FFMPEG_PATH`
2. 当前目录和打包后的资源目录里的 sidecar
3. 系统 PATH

## 注意

- `M4A 320kbps` 仍然依赖 ffmpeg 里存在 `aac_at` 编码器，通常只有 macOS 上的 ffmpeg 才会带这个能力。
- 不建议把大二进制直接提交进仓库；可以在本地或 CI 里按平台放进这个目录再打包。
- Windows 上建议优先用 [tools/fetch-windows-sidecars.ps1](../../tools/fetch-windows-sidecars.ps1) 拉取固定版本并校验 SHA-256，而不是依赖 `latest` 下载链接。
- macOS 上如果这里放的是 Homebrew 的 `sqlcipher` / `ffmpeg` 符号链接，也可以参与构建；预处理脚本会复制真实二进制并把非系统动态库一起打包。
- Apple Silicon 这组 sidecar 目前建议用 [tools/build-macos13-arm64-sidecars.sh](../../tools/build-macos13-arm64-sidecars.sh) 生成，默认把最低系统版本压到 macOS 13.0，并保留 `aac_at` / `libmp3lame`。
