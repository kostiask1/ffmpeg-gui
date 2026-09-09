$ErrorActionPreference = "Stop"

$archiveName = "ffmpeg-n8.1.2-51-g7ba069f4f1-win64-lgpl-8.1.zip"
$archiveSha256 = "9bb4c17bf1e271e7944a61d900716e61c6f6b4ef556544f3907b131cbc0bfd41"
$releaseTag = "autobuild-2026-09-08-23-15"
$downloadUrl = "https://github.com/BtbN/FFmpeg-Builds/releases/download/$releaseTag/$archiveName"
$version = "n8.1.2-51-g7ba069f4f1-win64-lgpl"

$projectRoot = Split-Path -Parent $PSScriptRoot
$tauriRoot = Join-Path $projectRoot "src-tauri"
$binaryDirectory = Join-Path $tauriRoot "binaries"
$licenseDirectory = Join-Path $tauriRoot "third-party\ffmpeg"
$markerPath = Join-Path $binaryDirectory ".ffmpeg-version.json"
$targetTriples = @("x86_64-pc-windows-msvc", "x86_64-pc-windows-gnu")

$expectedFiles = foreach ($program in @("ffmpeg", "ffprobe")) {
    foreach ($targetTriple in $targetTriples) {
        Join-Path $binaryDirectory "$program-$targetTriple.exe"
    }
}

$prepared = $false
if (Test-Path -LiteralPath $markerPath) {
    try {
        $marker = Get-Content -Raw -Encoding UTF8 -LiteralPath $markerPath | ConvertFrom-Json
        $prepared = $marker.version -eq $version -and
            $marker.archiveSha256 -eq $archiveSha256 -and
            ($expectedFiles | Where-Object { -not (Test-Path -LiteralPath $_) }).Count -eq 0
    }
    catch {
        $prepared = $false
    }
}

if ($prepared) {
    Write-Host "Bundled FFmpeg $version is ready."
    exit 0
}

New-Item -ItemType Directory -Path $binaryDirectory -Force | Out-Null
New-Item -ItemType Directory -Path $licenseDirectory -Force | Out-Null
$temporaryDirectory = Join-Path ([System.IO.Path]::GetTempPath()) "audio-forge-ffmpeg-$([guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Path $temporaryDirectory | Out-Null

try {
    $archivePath = Join-Path $temporaryDirectory $archiveName
    $extractDirectory = Join-Path $temporaryDirectory "extracted"
    Write-Host "Downloading the pinned LGPL FFmpeg build..."
    Invoke-WebRequest -Uri $downloadUrl -OutFile $archivePath

    $actualSha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $archivePath).Hash.ToLowerInvariant()
    if ($actualSha256 -ne $archiveSha256) {
        throw "FFmpeg archive checksum mismatch. Expected $archiveSha256, received $actualSha256."
    }

    Expand-Archive -LiteralPath $archivePath -DestinationPath $extractDirectory
    $ffmpegSource = Get-ChildItem -LiteralPath $extractDirectory -Recurse -Filter "ffmpeg.exe" -File | Select-Object -First 1
    $ffprobeSource = Get-ChildItem -LiteralPath $extractDirectory -Recurse -Filter "ffprobe.exe" -File | Select-Object -First 1
    if (-not $ffmpegSource -or -not $ffprobeSource) {
        throw "The verified FFmpeg archive does not contain ffmpeg.exe and ffprobe.exe."
    }

    foreach ($targetTriple in $targetTriples) {
        Copy-Item -LiteralPath $ffmpegSource.FullName -Destination (Join-Path $binaryDirectory "ffmpeg-$targetTriple.exe") -Force
        Copy-Item -LiteralPath $ffprobeSource.FullName -Destination (Join-Path $binaryDirectory "ffprobe-$targetTriple.exe") -Force
    }

    $licenseSource = Get-ChildItem -LiteralPath $extractDirectory -Recurse -File |
        Where-Object { $_.Name -in @("LICENSE.txt", "COPYING.LGPLv2.1") } |
        Select-Object -First 1
    if (-not $licenseSource) {
        throw "The verified FFmpeg archive does not contain its license file."
    }
    Copy-Item -LiteralPath $licenseSource.FullName -Destination (Join-Path $licenseDirectory "LICENSE.txt") -Force

    $marker = [ordered]@{
        version = $version
        releaseTag = $releaseTag
        archive = $archiveName
        archiveSha256 = $archiveSha256
    }
    $marker | ConvertTo-Json | Set-Content -Encoding UTF8 -LiteralPath $markerPath

    & (Join-Path $binaryDirectory "ffmpeg-x86_64-pc-windows-msvc.exe") -version | Select-Object -First 1
    & (Join-Path $binaryDirectory "ffprobe-x86_64-pc-windows-msvc.exe") -version | Select-Object -First 1
    Write-Host "Bundled FFmpeg $version prepared successfully."
}
finally {
    if (Test-Path -LiteralPath $temporaryDirectory) {
        Remove-Item -LiteralPath $temporaryDirectory -Recurse -Force
    }
}
