$ErrorActionPreference = "Stop"

[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$modelId = "ihnatov-yuri/parakeet-tdt-0.6b-v3-uk"
$revision = "5c4bcfaca501587beaedfd23b72a678bf7194de1"
$baseUrl = "https://huggingface.co/$modelId/resolve/main"
$requiredFiles = @(
    "config.json",
    "generation_config.json",
    "processor_config.json",
    "tokenizer.json",
    "tokenizer_config.json",
    "model.safetensors"
)
$minWeightsBytes = 2000000000L

function Test-ModelDirectory {
    param([string]$DirPath)

    if (-not $DirPath -or -not (Test-Path -LiteralPath $DirPath -PathType Container)) {
        return $false
    }

    foreach ($fileName in $requiredFiles) {
        $filePath = Join-Path $DirPath $fileName
        if (-not (Test-Path -LiteralPath $filePath -PathType Leaf)) {
            return $false
        }
    }

    $weightsInfo = Get-Item -LiteralPath (Join-Path $DirPath "model.safetensors")
    return ($weightsInfo.Length -ge $minWeightsBytes)
}

function Find-InstalledModelDirectory {
    $candidates = New-Object System.Collections.Generic.List[string]

    if ($env:LOCALAPPDATA) {
        $candidates.Add((Join-Path $env:LOCALAPPDATA "Audio Forge\models\parakeet-tdt-0.6b-v3-uk"))
    }

    $hfRoots = New-Object System.Collections.Generic.List[string]
    if ($env:HF_HOME) {
        $hfRoots.Add((Join-Path $env:HF_HOME "hub"))
    }
    if ($env:USERPROFILE) {
        $hfRoots.Add((Join-Path $env:USERPROFILE ".cache\huggingface\hub"))
    }

    foreach ($hubRoot in $hfRoots) {
        $snapshotsDir = Join-Path $hubRoot "models--ihnatov-yuri--parakeet-tdt-0.6b-v3-uk\snapshots"
        if (Test-Path -LiteralPath $snapshotsDir -PathType Container) {
            Get-ChildItem -LiteralPath $snapshotsDir -Directory -ErrorAction SilentlyContinue | ForEach-Object {
                $candidates.Add($_.FullName)
            }
        }
    }

    foreach ($dir in $candidates) {
        if (Test-ModelDirectory -DirPath $dir) {
            return $dir
        }
    }
    return $null
}

function Find-PythonExecutable {
    $candidates = New-Object System.Collections.Generic.List[string]
    if ($env:LOCALAPPDATA) {
        $pyRoot = Join-Path $env:LOCALAPPDATA "Programs\Python"
        $candidates.Add((Join-Path $pyRoot "Python314\python.exe"))
        if (Test-Path -LiteralPath $pyRoot -PathType Container) {
            Get-ChildItem -LiteralPath $pyRoot -Directory -ErrorAction SilentlyContinue | ForEach-Object {
                $exe = Join-Path $_.FullName "python.exe"
                if (-not $candidates.Contains($exe)) {
                    $candidates.Add($exe)
                }
            }
        }
    }

    foreach ($exe in $candidates) {
        if (Test-Path -LiteralPath $exe -PathType Leaf) {
            try {
                & $exe --version *> $null
                if ($LASTEXITCODE -eq 0) {
                    return $exe
                }
            }
            catch {
            }
        }
    }

    $pathPython = Get-Command "python.exe" -ErrorAction SilentlyContinue
    if ($pathPython -and $pathPython.Source) {
        try {
            & $pathPython.Source --version *> $null
            if ($LASTEXITCODE -eq 0) {
                return $pathPython.Source
            }
        }
        catch {
        }
    }
    return $null
}

function Ensure-PythonDependencies {
    $pythonExe = Find-PythonExecutable
    if (-not $pythonExe) {
        Write-Host "Python 3 was not found; skipping Python package check."
        return
    }

    & $pythonExe -c "import transformers, accelerate, soundfile" *> $null
    if ($LASTEXITCODE -eq 0) {
        Write-Host "Python transcription packages (transformers, accelerate, soundfile) are already installed."
        return
    }

    Write-Host "Installing Python dependencies (transformers, accelerate, soundfile)..."
    & $pythonExe -m pip install --no-input transformers accelerate soundfile
    if ($LASTEXITCODE -ne 0) {
        Write-Warning "Failed to install Python packages automatically (exit code $LASTEXITCODE)."
    }
}

function Download-ModelFile {
    param(
        [string]$Url,
        [string]$Destination
    )

    $tmpPath = "$Destination.download"
    if (Test-Path -LiteralPath $tmpPath) {
        Remove-Item -LiteralPath $tmpPath -Force -ErrorAction SilentlyContinue
    }

    $curlCmd = Get-Command "curl.exe" -ErrorAction SilentlyContinue
    if ($curlCmd) {
        & $curlCmd.Source -L --fail --retry 3 --retry-delay 2 --silent --show-error -o $tmpPath $Url
        if ($LASTEXITCODE -eq 0 -and (Test-Path -LiteralPath $tmpPath)) {
            Move-Item -LiteralPath $tmpPath -Destination $Destination -Force
            return
        }
        if (Test-Path -LiteralPath $tmpPath) {
            Remove-Item -LiteralPath $tmpPath -Force -ErrorAction SilentlyContinue
        }
    }

    Invoke-WebRequest -Uri $Url -OutFile $tmpPath -UseBasicParsing
    Move-Item -LiteralPath $tmpPath -Destination $Destination -Force
}

$existingDir = Find-InstalledModelDirectory
if ($existingDir) {
    Write-Host "Parakeet model $modelId is already installed at $existingDir. Skipping download."
    exit 0
}

Ensure-PythonDependencies

$hfHubBase = if ($env:HF_HOME) {
    Join-Path $env:HF_HOME "hub"
}
else {
    Join-Path $env:USERPROFILE ".cache\huggingface\hub"
}

$repoCacheRoot = Join-Path $hfHubBase "models--ihnatov-yuri--parakeet-tdt-0.6b-v3-uk"
$snapshotDir = Join-Path $repoCacheRoot "snapshots\$revision"
$refsDir = Join-Path $repoCacheRoot "refs"

New-Item -ItemType Directory -Path $snapshotDir -Force | Out-Null
New-Item -ItemType Directory -Path $refsDir -Force | Out-Null

foreach ($fileName in $requiredFiles) {
    $destPath = Join-Path $snapshotDir $fileName
    if (Test-Path -LiteralPath $destPath -PathType Leaf) {
        if ($fileName -ne "model.safetensors" -or (Get-Item -LiteralPath $destPath).Length -ge $minWeightsBytes) {
            continue
        }
    }
    Write-Host "Downloading $modelId/$fileName..."
    Download-ModelFile -Url "$baseUrl/$fileName" -Destination $destPath
}

[System.IO.File]::WriteAllText(
    (Join-Path $refsDir "main"),
    $revision,
    [System.Text.UTF8Encoding]::new($false)
)

if (-not (Test-ModelDirectory -DirPath $snapshotDir)) {
    throw "Model verification failed after download in $snapshotDir."
}

Write-Host "Parakeet model $modelId installed successfully to $snapshotDir."
exit 0
