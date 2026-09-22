#!/usr/bin/env pwsh
param(
    # Skip the ignored integration tests: they move the real default recording
    # device and master volume. CI and the commit hook run with this switch, so
    # an unattended gate can never touch the machine's audio.
    [switch]$GateOnly
)
$ErrorActionPreference = "Stop"
Write-Host "== Audio Switcher Smoke =="

# 1. formatting first: it is the cheapest check and the only one whose failure a
#    later step can be blamed for (rustfmt is the single formatter; drift is a
#    review cost).
Write-Host "[1] cargo fmt --all -- --check"
cargo fmt --all -- --check
if ($LASTEXITCODE -ne 0) { throw "formatting drifted; run cargo fmt --all" }

# 2. the DPI manifest as checked in (build.rs embeds it; package.ps1 scans the
#    packaged exe for the same string).
Write-Host "[2] checking manifest PerMonitorV2"
if (!(Select-String -Path "audio-switcher.manifest" -Pattern "PerMonitorV2")) { throw "DPI manifest missing" }

# 3. unit tests. This compiles the lib, the bin and every test target, so it
#    replaces a separate `cargo build`.
Write-Host "[3] cargo test"
cargo test
if ($LASTEXITCODE -ne 0) { throw "tests failed" }

# 4. the ignored integration tests: skipped by the unattended gate, fatal when
#    run explicitly (a failure there is a real failure).
if ($GateOnly) {
    Write-Host "[4] skipped (-GateOnly): the ignored tests move real audio devices"
} else {
    Write-Host "[4] cargo test -- --ignored (integration)"
    cargo test -- --ignored
    if ($LASTEXITCODE -ne 0) { throw "integration tests failed" }
}

# 5. clippy (respects Cargo.toml lints; -D warnings so style/perf warnings cannot
#    accumulate silently: the Cargo.toml groups emit warnings, not errors)
Write-Host "[5] cargo clippy --all-targets -- -D warnings"
cargo clippy --all-targets -- -D warnings
if ($LASTEXITCODE -ne 0) { throw "clippy failed" }
Write-Host "Smoke PASSED"
