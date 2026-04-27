$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$repoRoot = Split-Path -Parent $PSScriptRoot
$binDir = Join-Path $repoRoot "src-tauri/bin"
$tempRoot = Join-Path ([System.IO.Path]::GetTempPath()) "rekordport-windows-sidecars"
$ffmpegZip = Join-Path $tempRoot "ffmpeg.zip"
$ffmpegExtractDir = Join-Path $tempRoot "ffmpeg"
$sqlcipherTemp = Join-Path $tempRoot "sqlcipher.exe"
$targetTriple = if ($env:RKB_WINDOWS_TARGET_TRIPLE) {
  $env:RKB_WINDOWS_TARGET_TRIPLE
} else {
  "x86_64-pc-windows-msvc"
}

$allowUnverifiedDownloads = $env:RKB_ALLOW_UNVERIFIED_DOWNLOADS -eq "1"

$pinnedFfmpeg = @{
  "x86_64-pc-windows-msvc" = @{
    Url = "https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-04-26-13-08/ffmpeg-n8.1-10-g7f5c90f77e-win64-lgpl-8.1.zip"
    Sha256 = "d2bcaee1792a39e2bfd2c04a3d88daf53d4e857a6583fed68c03562106f745bd"
    Label = "BtbN FFmpeg win64 LGPL 8.1"
  }
  "aarch64-pc-windows-msvc" = @{
    Url = "https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-04-26-13-08/ffmpeg-n8.1-10-g7f5c90f77e-winarm64-lgpl-8.1.zip"
    Sha256 = "a29d83d01d3a07cfe060af439c803a082a508fd92c662a74d0ee946888ee4c1a"
    Label = "BtbN FFmpeg winarm64 LGPL 8.1"
  }
}

$pinnedSqlcipher = @{
  "x86_64-pc-windows-msvc" = @{
    Url = "https://raw.githubusercontent.com/Katecca/sqlcipher-static-binary/b7cb2d5dc1b6baee00e153ffbac8c6703f89da88/windows/x86_64/sqlcipher.exe"
    Sha256 = "19f16d2629adedc6ddc2aeebd2da165d61aa0d645a61d2de373396c04ad0031f"
    Label = "Katecca SQLCipher win64 static binary"
  }
}

if (-not $pinnedFfmpeg.ContainsKey($targetTriple)) {
  throw "Unsupported Windows target triple: $targetTriple"
}

function New-CleanDirectory([string]$Path) {
  if (Test-Path $Path) {
    Remove-Item -LiteralPath $Path -Recurse -Force
  }
  New-Item -ItemType Directory -Path $Path -Force | Out-Null
}

function Invoke-Download([string]$Url, [string]$OutFile) {
  Write-Host "Downloading $Url"
  Invoke-WebRequest -Uri $Url -Headers @{ "User-Agent" = "rekordport-build" } -OutFile $OutFile
}

function Assert-OptionalSha256([string]$FilePath, [string]$ExpectedHash, [string]$Label) {
  if ([string]::IsNullOrWhiteSpace($ExpectedHash)) {
    return
  }

  $actualHash = (Get-FileHash -Path $FilePath -Algorithm SHA256).Hash.ToLowerInvariant()
  if ($actualHash -ne $ExpectedHash.ToLowerInvariant()) {
    throw "$Label SHA256 mismatch. expected=$ExpectedHash actual=$actualHash"
  }
}

function Copy-RequiredTool([string]$SourcePath, [string]$TargetName) {
  $targetPath = Join-Path $binDir $TargetName
  Copy-Item -LiteralPath $SourcePath -Destination $targetPath -Force
  Write-Host "Prepared $targetPath"
}

function Resolve-DownloadSpec(
  [hashtable]$PinnedSpecs,
  [string]$TargetTriple,
  [string]$UrlEnvName,
  [string]$ShaEnvName,
  [string]$ToolLabel
) {
  $pinned = $null
  if ($PinnedSpecs.ContainsKey($TargetTriple)) {
    $pinned = $PinnedSpecs[$TargetTriple]
  }

  $url = [Environment]::GetEnvironmentVariable($UrlEnvName)
  $sha = [Environment]::GetEnvironmentVariable($ShaEnvName)

  if ([string]::IsNullOrWhiteSpace($url)) {
    if ($null -eq $pinned) {
      return $null
    }
    $url = $pinned.Url
  }

  if ([string]::IsNullOrWhiteSpace($sha)) {
    if ($null -ne $pinned -and $url -eq $pinned.Url) {
      $sha = $pinned.Sha256
    } elseif (-not $allowUnverifiedDownloads) {
      throw "Set $ShaEnvName when overriding $UrlEnvName for $ToolLabel, or set RKB_ALLOW_UNVERIFIED_DOWNLOADS=1 to bypass verification."
    } else {
      Write-Warning "Skipping SHA256 verification for $ToolLabel because RKB_ALLOW_UNVERIFIED_DOWNLOADS=1."
    }
  }

  $resolvedLabel = $ToolLabel
  if ($null -ne $pinned) {
    $resolvedLabel = $pinned.Label
  }

  return @{
    Url = $url
    Sha256 = $sha
    Label = $resolvedLabel
  }
}

New-Item -ItemType Directory -Path $binDir -Force | Out-Null
New-CleanDirectory $tempRoot
New-CleanDirectory $ffmpegExtractDir

$ffmpegSpec = Resolve-DownloadSpec $pinnedFfmpeg $targetTriple "RKB_FFMPEG_WINDOWS_URL" "RKB_FFMPEG_WINDOWS_SHA256" "ffmpeg archive"
Invoke-Download $ffmpegSpec.Url $ffmpegZip
Assert-OptionalSha256 $ffmpegZip $ffmpegSpec.Sha256 $ffmpegSpec.Label
Expand-Archive -LiteralPath $ffmpegZip -DestinationPath $ffmpegExtractDir -Force

$ffmpegExe = Get-ChildItem -Path $ffmpegExtractDir -Filter "ffmpeg.exe" -Recurse | Select-Object -First 1

if (-not $ffmpegExe) {
  throw "ffmpeg.exe was not found in $($ffmpegSpec.Url)"
}

Copy-RequiredTool $ffmpegExe.FullName "ffmpeg-$targetTriple.exe"

$sqlcipherSpec = Resolve-DownloadSpec $pinnedSqlcipher $targetTriple "RKB_SQLCIPHER_WINDOWS_URL" "RKB_SQLCIPHER_WINDOWS_SHA256" "sqlcipher binary"

if ($null -eq $sqlcipherSpec) {
  throw "No default sqlcipher download is configured for $targetTriple. Set RKB_SQLCIPHER_WINDOWS_URL and RKB_SQLCIPHER_WINDOWS_SHA256, or place sqlcipher-$targetTriple.exe in src-tauri/bin."
}

Invoke-Download $sqlcipherSpec.Url $sqlcipherTemp
Assert-OptionalSha256 $sqlcipherTemp $sqlcipherSpec.Sha256 $sqlcipherSpec.Label
Copy-RequiredTool $sqlcipherTemp "sqlcipher-$targetTriple.exe"

Write-Host "Windows sidecars are ready in $binDir"
