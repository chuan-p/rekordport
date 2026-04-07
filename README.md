# rekordport

桌面版 rekordbox 曲库检查工具，目标是找出：

- 所有 FLAC
- 真正的 ALAC
- WAV / AIFF 中 bit depth 大于 16bit 的条目

现在仓库里同时保留两层能力：

- `rekordbox_lossless_scan.py` 作为底层扫描器和导出器
- Tauri 桌面壳负责界面、文件选择、扫描、导出和转换

## 快速开始

先装依赖：

```bash
npm install
```

Tauri 桌面壳还需要这些依赖来源之一：

- Rust toolchain
- 系统 PATH 里的 `sqlcipher` / `ffmpeg`
- 或放在 `src-tauri/bin` 里的 sidecar 二进制

`npm run tauri dev` 会在这套工具链齐备时启动。

启动桌面开发模式：

```bash
npm run tauri dev
```

如果你只想先看底层扫描器，也可以继续直接运行：

```bash
python3 rekordbox_lossless_scan.py --format table
```

## GUI 功能

- 选择 `master.db`
- 一键扫描
- 表格预览结果
- 默认忽略 Rekordbox 自带 sampler 文件
- ALAC 目前按 Rekordbox 数据库里的 `FileType=6` 识别
- 选择曲目后可一键转换成 `WAV` / `AIFF`，并按源采样率自动落到 `44.1k` 或 `48k`；也可转 `MP3 320kbps` 或 `M4A 320kbps`
- 可以选择把源文件重命名保留在原文件夹，或删除到回收站
- 转换前会先备份数据库和原文件
- 转换后的新文件默认保持原文件名；如果目标位置已有同名文件，会直接报错，不再自动加 `-1`
- 转换后会在 `master.db` 里克隆新条目、删除旧条目，并把普通 playlist 里的旧 `ContentID` 重绑到新条目
- GUI 启动后会自动做环境预检，把缺失依赖和兼容性问题提前显示出来
- 运行时会优先使用内置 sidecar，找不到时再退回系统 PATH
- 转换完成后会显示每首歌的分析迁移状态，并自动归档未被引用的空分析目录

## 约定

- GUI 里的扫描和导出现在都直接走 Rust 后端，不再依赖系统 Python；Python 脚本主要保留给 CLI 使用
- 默认使用 `~/Library/Pioneer/rekordbox/master.db`
- 转换后会更新 rekordbox 数据库里的文件路径和音频字段，并把播放相关关联迁移到新条目
- 底层扫描器导出的 `codec_name` 字段会在 ALAC 条目里显示为 `alac`
- Rekordbox 的 USB 同步仍可能因为 `ContentID` 规则保留旧文件，这是软件本身的同步机制限制
- Windows 版和 macOS 版现在共用同一套 Rust 后端；默认数据库路径会自动切到 `%USERPROFILE%/AppData/Roaming/Pioneer/rekordbox/master.db`
- `M4A 320kbps` 依赖 ffmpeg 提供 Apple 的 `aac_at` 编码器，通常只有 macOS 环境能直接用；别的机器上会在预检里显示为不可用
- 如果源条目的分析资源已经损坏或不一致，转换会直接中止，避免再生成一个“看起来成功但实际丢了 grid”的新条目
- 目前普通 playlist 会自动重绑到新 `ContentID`，smart playlist 先不改

## Sidecar 打包

如果你想把依赖跟应用一起分发，把对应平台的二进制放进 [src-tauri/bin/README.md](/Users/chuanpeng/Documents/rkb-lossless-process/src-tauri/bin/README.md) 里说明的文件名即可。实际打包时不会直接拿这些源文件入包，而是先在 `src-tauri/gen/` 里生成一份可分发副本，再交给 Tauri 收进去。

运行时查找顺序：

1. 环境变量覆盖
   - `RKB_SQLCIPHER_PATH`
   - `RKB_FFMPEG_PATH`
2. `src-tauri/bin` 或打包后的 app 资源目录
3. 系统 PATH

macOS 打包前还会额外做这一步：

- 把 `src-tauri/bin` 里的 sidecar 解引用成真实文件，避免把 Homebrew 符号链接直接塞进安装包
- 递归收集 sidecar 的非系统 `.dylib` 依赖，复制到应用资源目录
- 用 `install_name_tool` 把 sidecar 改写成只引用 app 内部资源，不再依赖 `/opt/homebrew/...`
- 对 macOS sidecar 额外检查最低系统版本，当前默认要求 `aarch64-apple-darwin <= 13.0`、`x86_64-apple-darwin <= 10.15`，防止误打进只能跑在新系统上的二进制

如果你需要重新生成 Apple Silicon 的兼容 sidecar，可以直接跑：

```bash
tools/build-macos13-arm64-sidecars.sh
```

这会重新编出一套 `macOS 13` 可用的 `sqlcipher` / `ffmpeg`，并覆盖到 `src-tauri/bin/`。

这意味着像 `sqlcipher -> /opt/homebrew/opt/openssl@3/lib/libcrypto.3.dylib` 这种链路，现在会在构建期被内置进 app，而不是留给用户机器去碰运气。

常见文件名示例：

- `src-tauri/bin/ffmpeg-aarch64-apple-darwin`
- `src-tauri/bin/sqlcipher-aarch64-apple-darwin`
- `src-tauri/bin/ffmpeg-x86_64-pc-windows-msvc.exe`
- `src-tauri/bin/sqlcipher-x86_64-pc-windows-msvc.exe`

## Windows 构建

如果你是在 Windows 机器上本地打包：

```bash
npm ci
npm run tauri build -- --bundles nsis
```

如果你需要在同一台机器上指定目标架构，比如在 Apple Silicon 上打 Intel 包，也可以直接透传 Tauri 的 `--target`，预处理脚本现在会跟着切到对应 triple：

```bash
npm run tauri build -- --target x86_64-apple-darwin --bundles app
```

如果你把仓库放到 GitHub 上，也可以直接用 [.github/workflows/windows-build.yml](/Users/chuanpeng/Documents/rkb-lossless-process/.github/workflows/windows-build.yml)：

- 手动点 `workflow_dispatch`，生成 Windows 便携版 `rekordport.exe` artifact，可以直接双击运行
- 推送到 `main` 时自动构建一次，方便持续检查 Windows 打包没有坏
- 推送 `v*` tag 时除了上传 artifact，还会把便携版 `.exe` 自动挂到 GitHub Release

构建时现在会自动只打包“当前目标平台实际存在”的 sidecar，并且先生成当前目标平台的可分发副本。Windows x64 版还会把 `ffmpeg` / `sqlcipher` 嵌进主程序，启动时释放到临时目录调用，所以 GitHub Actions 上传的 artifact 只有一个 `rekordport.exe`：

- 如果 `src-tauri/bin` 里已经有 `ffmpeg-x86_64-pc-windows-msvc.exe` / `sqlcipher-x86_64-pc-windows-msvc.exe`，它们会被自动收进安装包，并嵌入便携版主程序
- 如果这些 Windows sidecar 还没放进去，构建也不会失败；应用运行时会继续按环境变量和系统 `PATH` 查找依赖
- 因为大多数 Windows `ffmpeg` 不带 Apple 的 `aac_at` 编码器，`M4A 320kbps` 在 Windows 上通常不可用，预检里会直接提示
- 便携版不会安装 WebView2；少数没有 WebView2 Runtime 的机器需要先安装 Microsoft WebView2 Runtime，或改用安装包方案
- macOS 上如果 sidecar 来自 Homebrew 之类的动态链接构建，构建脚本会把它依赖的非系统 `.dylib` 一起收进 app 资源并改写加载路径，避免生成“只在开发机可运行”的假分发包

仓库还附带了一个 sidecar 下载脚本 [tools/fetch-windows-sidecars.ps1](/Users/chuanpeng/Documents/rkb-lossless-process/tools/fetch-windows-sidecars.ps1)：

- 默认下载 `BtbN/FFmpeg-Builds` 的 `win64-lgpl` 包，提取 `ffmpeg.exe`
- 默认下载 `Katecca/sqlcipher-static-binary` 提供的 `sqlcipher.exe`
- 可以用 `RKB_FFMPEG_WINDOWS_URL` 和 `RKB_SQLCIPHER_WINDOWS_URL` 覆盖下载地址
- 如果你想锁定供应链校验，还可以额外设置 `RKB_FFMPEG_WINDOWS_SHA256` 和 `RKB_SQLCIPHER_WINDOWS_SHA256`

注意：

- `ffmpeg` 默认来源是 BtbN 的公开构建
- `sqlcipher.exe` 默认来源是社区维护的预编译二进制，不是 SQLCipher 官方公开发布的 Windows CLI；如果你有更可信的内部制品库或自建构建产物，建议用 `RKB_SQLCIPHER_WINDOWS_URL` 覆盖

## macOS 正式签名与发布

仓库里的 `npm run tauri build` 现在分成两种模式：

- 默认模式：如果没有检测到 Apple 正式签名相关环境变量，构建完成后会对 `.app` 做一次 ad-hoc 签名，适合本机测试和未公证的临时包
- 正式发布模式：如果设置了 `APPLE_SIGNING_IDENTITY`，并且同时提供 notarization 所需的 Apple 凭据，构建脚本会保留 Tauri 生成的正式签名，不再用 ad-hoc 重签覆盖

如果你的 partner 要在他自己的 Mac 上正式发布这个应用，推荐流程是：

1. 在钥匙串里安装他的 `Developer ID Application` 证书，并确认签名身份名字：

```bash
security find-identity -v -p codesigning
```

2. 在仓库根目录安装依赖：

```bash
npm ci
```

3. 设置签名和 notarization 环境变量。下面示例走 App Store Connect API key，是 Tauri 官方推荐的自动化方式之一：

```bash
export APPLE_SIGNING_IDENTITY="Developer ID Application: <Name> (<TeamID>)"
export APPLE_API_ISSUER="<issuer-id>"
export APPLE_API_KEY="<key-id>"
export APPLE_API_KEY_PATH="/absolute/path/to/AuthKey_<key-id>.p8"
```

如果你更想用 Apple ID，也可以改用：

```bash
export APPLE_SIGNING_IDENTITY="Developer ID Application: <Name> (<TeamID>)"
export APPLE_ID="<apple-id-email>"
export APPLE_PASSWORD="<app-specific-password>"
export APPLE_TEAM_ID="<team-id>"
```

4. 构建正式发布包。Apple Silicon：

```bash
npm run tauri build -- --target aarch64-apple-darwin --bundles app,dmg
```

Intel：

```bash
npm run tauri build -- --target x86_64-apple-darwin --bundles app,dmg
```

产物会出现在：

- `src-tauri/target/aarch64-apple-darwin/release/bundle/`
- `src-tauri/target/x86_64-apple-darwin/release/bundle/`

补充说明：

- 如果你只是想继续生成测试包，不需要设置任何 Apple 环境变量
- 如果你想手动强制开关 ad-hoc 重签，可以设置 `RKB_AD_HOC_SIGN_BUNDLED_APPS=true` 或 `RKB_AD_HOC_SIGN_BUNDLED_APPS=false`
- 仓库里现有的 `release/` 目录仍然只是测试产物，不代表正式签名或 notarized 发布包

## CLI 仍然可用

```bash
python3 rekordbox_lossless_scan.py --format csv
python3 rekordbox_lossless_scan.py --format json
python3 rekordbox_lossless_scan.py --output report.csv
python3 rekordbox_lossless_scan.py --output report.xlsx
python3 rekordbox_lossless_scan.py --include-sampler
python3 rekordbox_lossless_scan.py --min-bit-depth 24
```

扫描器现在直接按 Rekordbox 数据库里的 `FileType=6` 识别 ALAC；转换阶段的音频探测复用已打包的 `ffmpeg`，不再额外依赖 `ffprobe`。
