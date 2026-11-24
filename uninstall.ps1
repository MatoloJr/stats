# Remove Stats for Windows for the current user.
#
#   irm https://matolojr.github.io/stats/uninstall.ps1 | iex
#   %LOCALAPPDATA%\Programs\Stats\uninstall.ps1 [-Purge]
#
# Settings and the log in %APPDATA%\Stats are kept unless -Purge is given
# (or $env:STATS_PURGE = '1' with "irm | iex").

param([switch]$Purge = ($env:STATS_PURGE -eq '1'))

function Uninstall-Stats {
    param([bool]$Purge)

    $ErrorActionPreference = 'Stop'
    $installDir = Join-Path $env:LOCALAPPDATA 'Programs\Stats'
    $exePath = Join-Path $installDir 'stats.exe'

    Get-Process -Name stats -ErrorAction SilentlyContinue |
        Where-Object { $_.Path -and ($_.Path -ieq $exePath) } |
        ForEach-Object { $_ | Stop-Process -Force; $_.WaitForExit(5000) | Out-Null }

    # Only remove the autostart value if it is ours.
    $run = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
    $value = (Get-ItemProperty $run -Name 'Stats' -ErrorAction SilentlyContinue).Stats
    if ($value -and ($value.Trim('"') -ieq $exePath)) { Remove-ItemProperty $run -Name 'Stats' }

    Remove-Item -LiteralPath (Join-Path ([Environment]::GetFolderPath('Programs')) 'Stats.lnk') -Force -ErrorAction SilentlyContinue
    Remove-Item 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Stats' -Recurse -Force -ErrorAction SilentlyContinue

    if (Test-Path -LiteralPath $installDir) {
        for ($try = 0; Test-Path -LiteralPath $installDir; $try++) {
            try { Remove-Item -LiteralPath $installDir -Recurse -Force }
            catch { if ($try -ge 10) { throw "Could not remove $installDir : $($_.Exception.Message)" }; Start-Sleep -Milliseconds 500 }
        }
    }

    $settings = Join-Path $env:APPDATA 'Stats'
    if ($Purge) {
        Remove-Item -LiteralPath $settings -Recurse -Force -ErrorAction SilentlyContinue
        Write-Host 'Stats is removed, with its settings.'
    } else {
        Write-Host 'Stats is removed.'
        if (Test-Path -LiteralPath $settings) { Write-Host "Your settings are kept in $settings (delete that folder, or run with -Purge, to remove them)." }
    }
}

try {
    Uninstall-Stats -Purge ([bool]$Purge)
} catch {
    Write-Host "Stats could not be fully removed: $($_.Exception.Message)" -ForegroundColor Red
    if ($MyInvocation.MyCommand.Path) { exit 1 }
}
