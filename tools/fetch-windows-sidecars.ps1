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

$ffmpegTargetName = switch ($targetTriple) {
  "aarch64-pc-windows-msvc" { "winarm64" }
  "x86_64-pc-windows-msvc" { "win64" }
  default { throw "Unsupported Windows target triple: $targetTriple" }
}

$ffmpegUrl = if ($env:RKB_FFMPEG_WINDOWS_URL) {
  $env:RKB_FFMPEG_WINDOWS_URL
} elseif ($env:RKB_FFMPEG_WINDOWS_DEFAULT_URL) {
  $env:RKB_FFMPEG_WINDOWS_DEFAULT_URL
} else {
  "https://github.com/BtbN/FFmpeg-Builds/releases/latest/download/ffmpeg-master-latest-$ffmpegTargetName-lgpl.zip"
}

$sqlcipherUrl = if ($env:RKB_SQLCIPHER_WINDOWS_URL) {
  $env:RKB_SQLCIPHER_WINDOWS_URL
} elseif ($targetTriple -eq "x86_64-pc-windows-msvc") {
  "https://raw.githubusercontent.com/Katecca/sqlcipher-static-binary/master/windows/x86_64/sqlcipher.exe"
} else {
  $null
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

New-Item -ItemType Directory -Path $binDir -Force | Out-Null
New-CleanDirectory $tempRoot
New-CleanDirectory $ffmpegExtractDir

Invoke-Download $ffmpegUrl $ffmpegZip
Assert-OptionalSha256 $ffmpegZip $env:RKB_FFMPEG_WINDOWS_SHA256 "ffmpeg archive"
Expand-Archive -LiteralPath $ffmpegZip -DestinationPath $ffmpegExtractDir -Force

$ffmpegExe = Get-ChildItem -Path $ffmpegExtractDir -Filter "ffmpeg.exe" -Recurse | Select-Object -First 1

if (-not $ffmpegExe) {
  throw "ffmpeg.exe was not found in $ffmpegUrl"
}

Copy-RequiredTool $ffmpegExe.FullName "ffmpeg-$targetTriple.exe"

if ([string]::IsNullOrWhiteSpace($sqlcipherUrl)) {
  throw "No default sqlcipher download is configured for $targetTriple. Set RKB_SQLCIPHER_WINDOWS_URL or place sqlcipher-$targetTriple.exe in src-tauri/bin."
}

Invoke-Download $sqlcipherUrl $sqlcipherTemp
Assert-OptionalSha256 $sqlcipherTemp $env:RKB_SQLCIPHER_WINDOWS_SHA256 "sqlcipher binary"
Copy-RequiredTool $sqlcipherTemp "sqlcipher-$targetTriple.exe"

Write-Host "Windows sidecars are ready in $binDir"
