#!/usr/bin/env pwsh
# Single-file release packaging: build -> verify self-containment -> stage dist.
#
# The artifact is one exe with nothing beside it:
#   * icons + VERSIONINFO embedded by build.rs (winres)
#   * DPI manifest embedded by build.rs (embed-manifest)
#   * every dependency is a Rust static lib (rlib)
#   * the MSVC CRT is linked statically (.cargo/config.toml), so no
#     vcruntime140.dll / VC++ Redistributable is required on the target machine
# Config and logs are created at runtime under %APPDATA% / %LOCALAPPDATA%;
# nothing is read from the exe's own directory.
#
# dist/ holds exactly one release: the exe plus its sha256sum-format `.sha256`
# sidecar. Artifacts of other versions are pruned on every run.
#
# Run scripts/smoke.ps1 first: the three greens + clippy are the release gate.
$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
$budget = 800KB   # scheme §6 P0 single-exe budget (never loosen silently)

Push-Location $root
try {
    Write-Host "== Audio Switcher package =="

    $cargoToml = Get-Content "Cargo.toml" -Raw
    $version = [regex]::Match($cargoToml, '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
    if (-not $version) { throw "cannot read version from Cargo.toml" }
    Write-Host "[1] version $version"

    Write-Host "[2] cargo build --release --locked"
    cargo build --release --locked
    if ($LASTEXITCODE -ne 0) { throw "release build failed" }

    $exe = Join-Path $root "target\release\audio-switcher.exe"
    if (-not (Test-Path $exe)) { throw "missing $exe" }
    Write-Host "[3] built $(Split-Path -Leaf $exe)"

    # ---- self-containment: no non-OS runtime dependency ----
    $banned = @("vcruntime140", "msvcp140", "ucrtbase", "api-ms-win-crt", "concrt140", "libgcc", "libstdc++")
    $dumpbin = @(
        "$env:ProgramFiles\Microsoft Visual Studio\*\*\VC\Tools\MSVC\*\bin\Hostx64\x64\dumpbin.exe"
        "${env:ProgramFiles(x86)}\Microsoft Visual Studio\*\*\VC\Tools\MSVC\*\bin\Hostx64\x64\dumpbin.exe"
    ) | ForEach-Object { Get-ChildItem -Path $_ -ErrorAction SilentlyContinue } | Select-Object -First 1

    if ($dumpbin) {
        $deps = & $dumpbin.FullName /nologo /dependents $exe | Select-String -Pattern '^\s+\S+\.dll$' |
            ForEach-Object { $_.Line.Trim() } | Sort-Object -Unique
        Write-Host ("[4] imports: " + ($deps -join ", "))
        $bad = $deps | Where-Object { $dep = $_; $banned | Where-Object { $dep -like "*$_*" } }
    } else {
        # No MSVC toolchain visible: fall back to a byte scan for the DLL names
        # a dynamically linked CRT would have to import.
        Write-Host "[4] dumpbin not found; scanning image for runtime DLL references"
        $bytes = [System.IO.File]::ReadAllBytes($exe)
        $ascii = [System.Text.Encoding]::ASCII.GetString($bytes)
        $bad = $banned | Where-Object { $ascii -match [regex]::Escape($_) }
    }
    if ($bad) { throw "exe still depends on non-OS runtime libraries: $($bad -join ', ')" }

    Write-Host "[5] stage dist (current release only)"
    # Budget gate runs before staging: an oversized image must never reach dist/.
    $size = (Get-Item $exe).Length
    if ($size -gt $budget) { throw ("size {0:N0} exceeds the {1:N0}-byte budget (scheme §6)" -f $size, $budget) }

    $dist = Join-Path $root "dist"
    New-Item -ItemType Directory -Force -Path $dist | Out-Null
    $name = "audio-switcher-v$version-x64.exe"
    $checksum = "$name.sha256"
    Get-ChildItem -Path $dist -File |
        Where-Object { $_.Name -notin @($name, $checksum) } |
        ForEach-Object { Write-Host "      pruning $($_.Name)"; Remove-Item $_.FullName -Force }

    $out = Join-Path $dist $name
    Copy-Item $exe $out -Force
    # sha256sum-format sidecar: lowercase hash, two spaces, bare file name.
    $hash = (Get-FileHash $out -Algorithm SHA256).Hash.ToLowerInvariant()
    $sum = Join-Path $dist $checksum
    [System.IO.File]::WriteAllText($sum, "$hash  $name`n")

    Write-Host "[6] report"
    $info = (Get-Item $out).VersionInfo
    Write-Host ""
    Write-Host "artifact : $out"
    Write-Host ("size     : {0:N0} bytes ({1:N1} KB)" -f $size, ($size / 1KB))
    Write-Host ("sha256   : {0}" -f $hash)
    Write-Host "checksum : $sum"
    Write-Host ("version  : file={0} product={1}" -f $info.FileVersion, $info.ProductName)
    Write-Host ("budget   : within {0:N0} bytes" -f $budget)
    Write-Host "Packaging PASSED"
}
finally {
    Pop-Location
}
