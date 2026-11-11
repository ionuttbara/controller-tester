$ErrorActionPreference = "Stop"

Write-Host "Building IBBE Controller Manager (release)..." -ForegroundColor Cyan
cargo build --release

$exe = Join-Path $PSScriptRoot "target\release\IBBE-ControllerManager.exe"
if (-not (Test-Path $exe)) {
    throw "Build completed but executable was not found at $exe"
}

Write-Host "Build complete:" -ForegroundColor Green
Write-Host $exe
