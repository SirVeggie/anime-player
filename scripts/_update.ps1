# Do not run this file directly — use update.bat in the same folder.

$ErrorActionPreference = 'Stop'

$Repo = 'SirVeggie/anime-player'
$ManifestUrl = "https://github.com/$Repo/releases/latest/download/manifest.json"
$InstallDir = $PSScriptRoot
$ExeName = 'anime-player.exe'
$VersionPath = Join-Path $InstallDir 'VERSION.txt'
$PendingDir = Join-Path $InstallDir '_pending'
$AllowedFiles = @(
  'anime-player.exe',
  'libmpv-2.dll',
  'ffmpeg.exe',
  'ffprobe.exe',
  'fpcalc.exe',
  'update.bat',
  '_update.ps1'
)

function Write-Info([string]$Message) {
  Write-Host $Message
}

function Write-Err([string]$Message) {
  Write-Host $Message -ForegroundColor Red
}

function Get-GitHubHeaders {
  return @{
    Accept       = 'application/json, application/octet-stream'
    'User-Agent' = 'anime-player-updater'
  }
}

function Test-AllowedFileName([string]$Name) {
  if ($AllowedFiles -notcontains $Name) {
    return $false
  }
  if ($Name -match '[\\/]' -or $Name -like '*..*') {
    return $false
  }
  return $true
}

function Get-FileSha256Lower([string]$Path) {
  return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

$running = Get-Process -Name 'anime-player' -ErrorAction SilentlyContinue
if ($running) {
  Write-Err 'Anime Player is still running. Close it and run update.bat again.'
  exit 1
}

Write-Info "Checking for updates ($Repo)..."

try {
  $manifest = Invoke-RestMethod -Uri $ManifestUrl -Headers (Get-GitHubHeaders)
} catch {
  Write-Err "Failed to download update manifest: $($_.Exception.Message)"
  exit 1
}

if ($null -eq $manifest -or [string]::IsNullOrWhiteSpace([string]$manifest.version) -or -not $manifest.files) {
  Write-Err 'Update manifest is missing a version or file list.'
  exit 1
}

$tag = [string]$manifest.version
$needed = @()
foreach ($file in $manifest.files) {
  $name = [string]$file.name
  if (-not (Test-AllowedFileName $name)) {
    Write-Err "Manifest listed a disallowed file: $name"
    exit 1
  }
  $expected = ([string]$file.sha256).Trim().ToLowerInvariant()
  if ($expected -notmatch '^[0-9a-f]{64}$') {
    Write-Err "Manifest hash for $name is not a SHA256 digest."
    exit 1
  }
  $url = [string]$file.url
  if ($url -notmatch '^https://') {
    Write-Err "Manifest url for $name must be https."
    exit 1
  }
  $localPath = Join-Path $InstallDir $name
  $needsDownload = $true
  if (Test-Path -LiteralPath $localPath) {
    $actual = Get-FileSha256Lower $localPath
    if ($actual -eq $expected) {
      $needsDownload = $false
    }
  }
  if ($needsDownload) {
    $needed += [pscustomobject]@{
      Name     = $name
      Sha256   = $expected
      Url      = $url
      Size     = [int64]$file.size
    }
  }
}

$localVersion = $null
if (Test-Path -LiteralPath $VersionPath) {
  $localVersion = (Get-Content -LiteralPath $VersionPath -Raw).Trim()
}

if ($needed.Count -eq 0 -and $localVersion -eq $tag) {
  Write-Info "Already on $tag. Nothing to do."
  exit 0
}

if ($needed.Count -eq 0) {
  Set-Content -LiteralPath $VersionPath -Value $tag -NoNewline -Encoding utf8
  Write-Info "Already have the $tag files. Updated VERSION.txt."
  exit 0
}

$downloadBytes = ($needed | Measure-Object -Property Size -Sum).Sum
Write-Info "Downloading $($needed.Count) file(s) for $tag ($([math]::Round($downloadBytes / 1MB, 1)) MB)..."

if (Test-Path -LiteralPath $PendingDir) {
  Remove-Item -LiteralPath $PendingDir -Recurse -Force
}
New-Item -ItemType Directory -Path $PendingDir | Out-Null

try {
  foreach ($file in $needed) {
    $dest = Join-Path $PendingDir $file.Name
    Write-Info "Downloading $($file.Name)..."
    Invoke-WebRequest -Uri $file.Url -OutFile $dest -UseBasicParsing -Headers (Get-GitHubHeaders)
    $actual = Get-FileSha256Lower $dest
    if ($actual -ne $file.Sha256) {
      throw "SHA256 mismatch for $($file.Name). Expected $($file.Sha256) but got $actual."
    }
  }

  foreach ($file in $needed) {
    $source = Join-Path $PendingDir $file.Name
    $dest = Join-Path $InstallDir $file.Name
    Copy-Item -LiteralPath $source -Destination $dest -Force
  }
  Set-Content -LiteralPath $VersionPath -Value $tag -NoNewline -Encoding utf8
} catch {
  Write-Err "Update failed: $($_.Exception.Message)"
  if (Test-Path -LiteralPath $PendingDir) {
    Remove-Item -LiteralPath $PendingDir -Recurse -Force -ErrorAction SilentlyContinue
  }
  exit 1
}

Remove-Item -LiteralPath $PendingDir -Recurse -Force -ErrorAction SilentlyContinue
Write-Info "Updated to $tag."
