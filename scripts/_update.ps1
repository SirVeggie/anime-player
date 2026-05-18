# Do not run this file directly — use update.bat in the same folder.

$ErrorActionPreference = 'Stop'

$Repo = 'SirVeggie/anime-player'
$ExeName = 'anime-player.exe'
$HashAssetName = 'anime-player.exe.sha256'
$InstallDir = $PSScriptRoot
$ExePath = Join-Path $InstallDir $ExeName
$VersionPath = Join-Path $InstallDir 'VERSION.txt'
$DownloadPath = Join-Path $InstallDir "$ExeName.download"
$BackupPath = Join-Path $InstallDir "$ExeName.bak"

function Write-Info([string]$Message) {
  Write-Host $Message
}

function Write-Err([string]$Message) {
  Write-Host $Message -ForegroundColor Red
}

function Get-GitHubHeaders {
  $headers = @{
    Accept       = 'application/vnd.github+json'
    'User-Agent' = 'anime-player-updater'
  }
  if ($env:GITHUB_TOKEN) {
    $headers.Authorization = "Bearer $($env:GITHUB_TOKEN)"
  }
  return $headers
}

function Get-ResponseText($Content) {
  if ($null -eq $Content) {
    return ''
  }
  if ($Content -is [byte[]]) {
    return [System.Text.Encoding]::UTF8.GetString($Content)
  }
  return [string]$Content
}

function Get-ExpectedSha256([string]$HashText) {
  $line = ($HashText -split "`n" | Where-Object { $_.Trim() -ne '' } | Select-Object -First 1).Trim()
  if ($line -match '^([0-9A-Fa-f]{64})\s') {
    return $Matches[1].ToUpperInvariant()
  }
  if ($line -match '^([0-9A-Fa-f]{64})$') {
    return $Matches[1].ToUpperInvariant()
  }
  throw "Could not parse SHA256 from release asset ($HashAssetName)."
}

$running = Get-Process -Name 'anime-player' -ErrorAction SilentlyContinue
if ($running) {
  Write-Err 'Anime Player is still running. Close it and run update.bat again.'
  exit 1
}

Write-Info "Checking for updates ($Repo)..."

try {
  $release = Invoke-RestMethod `
    -Uri "https://api.github.com/repos/$Repo/releases/latest" `
    -Headers (Get-GitHubHeaders)
} catch {
  Write-Err "Failed to query GitHub releases: $($_.Exception.Message)"
  exit 1
}

$tag = [string]$release.tag_name
if ([string]::IsNullOrWhiteSpace($tag)) {
  Write-Err 'Latest release has no tag_name.'
  exit 1
}

$localVersion = $null
if (Test-Path -LiteralPath $VersionPath) {
  $localVersion = (Get-Content -LiteralPath $VersionPath -Raw).Trim()
}

if ($localVersion -eq $tag -and (Test-Path -LiteralPath $ExePath)) {
  Write-Info "Already on $tag. Nothing to do."
  exit 0
}

$asset = $release.assets | Where-Object { $_.name -eq $ExeName } | Select-Object -First 1
if (-not $asset -or -not $asset.browser_download_url) {
  Write-Err "Release $tag does not include a $ExeName asset."
  exit 1
}

$hashAsset = $release.assets | Where-Object { $_.name -eq $HashAssetName } | Select-Object -First 1

Write-Info "Downloading $ExeName from $tag..."

if (Test-Path -LiteralPath $DownloadPath) {
  Remove-Item -LiteralPath $DownloadPath -Force
}

try {
  Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $DownloadPath -UseBasicParsing
} catch {
  Write-Err "Download failed: $($_.Exception.Message)"
  if (Test-Path -LiteralPath $DownloadPath) {
    Remove-Item -LiteralPath $DownloadPath -Force -ErrorAction SilentlyContinue
  }
  exit 1
}

if ($hashAsset -and $hashAsset.browser_download_url) {
  Write-Info 'Verifying download hash...'
  try {
    $hashRaw = (Invoke-WebRequest -Uri $hashAsset.browser_download_url -UseBasicParsing).Content
    $expected = Get-ExpectedSha256 (Get-ResponseText $hashRaw)
    $actual = (Get-FileHash -LiteralPath $DownloadPath -Algorithm SHA256).Hash.ToUpperInvariant()
    if ($actual -ne $expected) {
      Write-Err "SHA256 mismatch. Expected $expected but got $actual."
      Remove-Item -LiteralPath $DownloadPath -Force
      exit 1
    }
  } catch {
    Write-Err "Hash verification failed: $($_.Exception.Message)"
    Remove-Item -LiteralPath $DownloadPath -Force -ErrorAction SilentlyContinue
    exit 1
  }
} else {
  Write-Info 'No SHA256 asset on this release; skipping hash check.'
}

if (Test-Path -LiteralPath $ExePath) {
  Copy-Item -LiteralPath $ExePath -Destination $BackupPath -Force
}

try {
  Move-Item -LiteralPath $DownloadPath -Destination $ExePath -Force
  Set-Content -LiteralPath $VersionPath -Value $tag -NoNewline -Encoding utf8
  if (Test-Path -LiteralPath $BackupPath) {
    Remove-Item -LiteralPath $BackupPath -Force
  }
} catch {
  Write-Err "Failed to install new executable: $($_.Exception.Message)"
  if ((Test-Path -LiteralPath $BackupPath) -and -not (Test-Path -LiteralPath $ExePath)) {
    Move-Item -LiteralPath $BackupPath -Destination $ExePath -Force
  }
  if (Test-Path -LiteralPath $DownloadPath) {
    Remove-Item -LiteralPath $DownloadPath -Force -ErrorAction SilentlyContinue
  }
  exit 1
}

Write-Info "Updated to $tag."
