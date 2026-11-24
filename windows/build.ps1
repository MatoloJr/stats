# Build Stats for Windows and (optionally) install it to start with Windows.
#   .\build.ps1            build only
#   .\build.ps1 -Install   build, copy to %LOCALAPPDATA%\Programs\Stats, start it
param([switch]$Install)
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

# Keep the build itself light on a 16 GB machine.
$env:CARGO_BUILD_JOBS = "4"
cargo test --lib
cargo build --release
$exe = Join-Path $PSScriptRoot "target\release\stats.exe"
Write-Host "Built $exe ($([math]::Round((Get-Item $exe).Length / 1MB, 2)) MB)"

if ($Install) {
    $dest = Join-Path $env:LOCALAPPDATA "Programs\Stats"
    New-Item -ItemType Directory -Force $dest | Out-Null
    Get-Process stats -ErrorAction SilentlyContinue | Stop-Process -Force
    Copy-Item $exe $dest -Force
    $installed = Join-Path $dest "stats.exe"
    # Start with Windows (the tray menu toggles this too).
    Set-ItemProperty "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run" -Name "Stats" -Value "`"$installed`""
    Start-Process $installed
    Write-Host "Installed to $installed and started. It will start with Windows."
    # The release build output is about 1 GB of intermediate files; it is
    # safe to delete and is rebuilt next time.
    Write-Host "Tip: 'cargo clean' here frees the build folder now that it is installed."
}
