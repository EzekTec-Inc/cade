<#
.SYNOPSIS
Installs CADE AI Coding Assistant on Windows.

.DESCRIPTION
This script detects the latest release from GitHub, downloads the Windows binaries (cade.exe and cade-server.exe),
extracts them to the user's Local AppData, adds the directory to the PATH, and starts CADE.
#>

$ErrorActionPreference = "Stop"

$Repo = "EzekTec-Inc/cade"
$InstallDir = "$env:LOCALAPPDATA\EzekTec\CADE\bin"

Write-Host "=========================================="
Write-Host "    Installing CADE AI Coding Assistant   "
Write-Host "=========================================="

# 1. Fetch Latest Release
Write-Host "[1/4] Fetching latest release info..."
$ApiUrl = "https://api.github.com/repos/$Repo/releases/latest"
$Release = Invoke-RestMethod -Uri $ApiUrl
$LatestVersion = $Release.tag_name

if ([string]::IsNullOrEmpty($LatestVersion)) {
    Write-Error "Could not determine latest release version."
    exit 1
}
Write-Host "Latest version: $LatestVersion"

$Target = "x86_64-pc-windows-msvc"
$AssetName = "cade-${Target}.zip"
$DownloadUrl = "https://github.com/$Repo/releases/download/$LatestVersion/$AssetName"

# 2. Setup Directories
if (-not (Test-Path $InstallDir)) {
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
}

$TmpFile = [System.IO.Path]::GetTempFileName() + ".zip"
$ExtractDir = [System.IO.Path]::Combine([System.IO.Path]::GetTempPath(), [guid]::NewGuid().ToString())
New-Item -ItemType Directory -Force -Path $ExtractDir | Out-Null

# 3. Download and Extract
Write-Host "[2/4] Downloading $AssetName..."
Invoke-WebRequest -Uri $DownloadUrl -OutFile $TmpFile

Write-Host "[3/4] Extracting binaries..."
Expand-Archive -Path $TmpFile -DestinationPath $ExtractDir -Force

# 4. Install Binaries
Write-Host "[4/4] Installing to $InstallDir..."
Move-Item -Path "$ExtractDir\cade.exe" -Destination "$InstallDir\cade.exe" -Force
Move-Item -Path "$ExtractDir\cade-server.exe" -Destination "$InstallDir\cade-server.exe" -Force

# Clean up
Remove-Item -Path $TmpFile -Force
Remove-Item -Path $ExtractDir -Recurse -Force

# 5. Add to PATH
$UserPath = [Environment]::GetEnvironmentVariable("PATH", "User")
if ($UserPath -notlike "*$InstallDir*") {
    Write-Host "Adding $InstallDir to user PATH..."
    $NewPath = "$UserPath;$InstallDir"
    [Environment]::SetEnvironmentVariable("PATH", $NewPath, "User")
    $env:PATH = "$env:PATH;$InstallDir"
}

Write-Host "=========================================="
Write-Host "    CADE successfully installed!          "
Write-Host "=========================================="
Write-Host "Starting CADE for the first time..."

# 6. Run CADE
Start-Process -NoNewWindow "$InstallDir\cade.exe"
