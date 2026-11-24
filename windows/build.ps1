# Build Stats for Windows and (optionally) install it.
#   .\build.ps1            test, build, and lay out ..\target\dist (stats.exe, uninstall.ps1, SHA256SUMS)
#   .\build.ps1 -Install   the same, then install from that folder with install.ps1 (the path users get)
param([switch]$Install)
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

# Keep the build itself light on a 16 GB machine.
$env:CARGO_BUILD_JOBS = "4"
cargo test -p stats-core
if ($LASTEXITCODE -ne 0) { throw "tests failed" }
cargo build --release -p stats-win
if ($LASTEXITCODE -ne 0) { throw "build failed" }

$root = Split-Path $PSScriptRoot -Parent
$exe = Join-Path $root "target\release\stats.exe"
$dist = Join-Path $root "target\dist"
New-Item -ItemType Directory -Force $dist | Out-Null
Copy-Item $exe, (Join-Path $PSScriptRoot "uninstall.ps1") $dist -Force
$sums = foreach ($name in "stats.exe", "uninstall.ps1") {
    "{0}  {1}" -f (Get-FileHash -Algorithm SHA256 (Join-Path $dist $name)).Hash.ToLowerInvariant(), $name
}
[IO.File]::WriteAllText((Join-Path $dist "SHA256SUMS"), (($sums -join "`n") + "`n"))
Write-Host "Built $exe ($([math]::Round((Get-Item $exe).Length / 1MB, 2)) MB); release files in $dist"

if ($Install) {
    $version = (Get-Content (Join-Path $root "VERSION") -Raw).Trim()
    & (Join-Path $PSScriptRoot "install.ps1") -Source $dist -Version "$version-dev"
    # The release build output is about 1 GB of intermediate files; it is
    # safe to delete and is rebuilt next time.
    Write-Host "Tip: 'cargo clean' in $root frees the build folder now that it is installed."
}
