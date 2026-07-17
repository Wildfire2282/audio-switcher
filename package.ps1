# Build and package audio-switcher
param(
    [switch]$SkipBuild = $false
)

$ErrorActionPreference = "Stop"
$ProjectRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$BuildDir = "$ProjectRoot\build"

Write-Host "=== Audio Switcher Package Script ===" -ForegroundColor Cyan

# 1. Setup VS environment
$vsPath = "C:\Program Files\Microsoft Visual Studio\18\Community"
$VsDevCmd = "$vsPath\Common7\Tools\VsDevCmd.bat"
if (-not (Test-Path $VsDevCmd)) {
    Write-Error "VS Dev Command Prompt not found at $VsDevCmd"
    exit 1
}

Write-Host "[1/3] Setting up Visual Studio environment..." -ForegroundColor Yellow
# Import VS environment variables from the VS dev cmd
cmd /c "`"$VsDevCmd`" -arch=amd64 -host_arch=amd64 2>&1 && set" | ForEach-Object {
    if ($_ -match '^(PATH|LIB|INCLUDE|VCToolsInstallDir|WindowsSDKVersion|WindowsSdkDir|VSCMD_ARG_.*|VisualStudioVersion|VSINSTALLDIR)=(.*)$') {
        Set-Item -Path "env:$($matches[1])" -Value $matches[2] -ErrorAction SilentlyContinue
    }
}

# Prepend MSVC bin path to PATH so the real MSVC link.exe is found before Git Bash's stub
$msvcBin = "$vsPath\VC\Tools\MSVC\14.52.36520\bin\Hostx64\x64"
$env:PATH = "$msvcBin;$env:PATH"

$linkExe = (Get-Command link.exe -ErrorAction SilentlyContinue).Source
Write-Host "  Linker: $linkExe" -ForegroundColor Gray

# 2. Build release
if (-not $SkipBuild) {
    Write-Host "[2/3] Building release binary..." -ForegroundColor Yellow
    Push-Location $ProjectRoot
    cargo build --release --bin audio-switcher
    if ($LASTEXITCODE -ne 0) {
        Write-Error "Build failed!"
        exit 1
    }
    Pop-Location
} else {
    Write-Host "[2/3] Skipping build (--SkipBuild)" -ForegroundColor Yellow
}

# 3. Package
Write-Host "[3/3] Packaging..." -ForegroundColor Yellow
$ReleaseExe = "$ProjectRoot\target\release\audio-switcher.exe"
if (-not (Test-Path $ReleaseExe)) {
    Write-Error "Release binary not found at $ReleaseExe"
    exit 1
}

if (Test-Path $BuildDir) {
    Remove-Item -Recurse -Force $BuildDir
}
New-Item -ItemType Directory -Force -Path $BuildDir | Out-Null

Copy-Item $ReleaseExe "$BuildDir\audio-switcher.exe" -Force

$AssetDir = "$BuildDir\assets"
New-Item -ItemType Directory -Force -Path $AssetDir | Out-Null
Copy-Item "$ProjectRoot\assets\icon_light.png" "$AssetDir\" -Force
Copy-Item "$ProjectRoot\assets\icon_dark.png" "$AssetDir\" -Force

Write-Host ""
Write-Host "Package created at: $BuildDir" -ForegroundColor Green
Write-Host "  audio-switcher.exe   $((Get-Item "$BuildDir\audio-switcher.exe").Length / 1KB) KB" -ForegroundColor Gray
Write-Host "  assets\icon_light.png" -ForegroundColor Gray
Write-Host "  assets\icon_dark.png" -ForegroundColor Gray
Write-Host "Done!" -ForegroundColor Green
