# Install or upgrade Stats for Windows for the current user (no admin needed).
#
#   irm https://matolojr.github.io/stats/install.ps1 | iex
#
# Or from a checkout or a downloaded copy:
#   .\install.ps1                       latest release
#   .\install.ps1 -Version 0.2.0        a specific release
#   .\install.ps1 -Source C:\dist       a local folder holding stats.exe, uninstall.ps1, SHA256SUMS (testing)
#
# With "irm | iex" the same choices come from $env:STATS_VERSION and $env:STATS_RELEASE_BASE.
# Remove: irm https://matolojr.github.io/stats/uninstall.ps1 | iex
#     or: Settings > Apps > Installed apps > Stats > Uninstall

param(
    [string]$Version = $env:STATS_VERSION,
    [string]$Source = $env:STATS_RELEASE_BASE,
    [switch]$NoStart
)

# Everything is inside a function, so a download cut short never runs half a script.
function Install-Stats {
    param([string]$Version, [string]$Source, [bool]$NoStart)

    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue'   # Invoke-WebRequest is many times faster without the progress bar
    $repo = 'MatoloJr/stats'
    $installDir = Join-Path $env:LOCALAPPDATA 'Programs\Stats'
    $exePath = Join-Path $installDir 'stats.exe'
    $files = @('stats.exe', 'uninstall.ps1')

    function Step([int]$n, [string]$label) {
        $width = 20
        $filled = [int]($n * $width / 5)
        Write-Host ('  [' + ('#' * $filled) + ('-' * ($width - $filled)) + '] ' + $label)
    }

    # --- preflight -----------------------------------------------------------
    if ([Environment]::OSVersion.Version.Major -lt 10) { throw 'Stats needs Windows 10 or 11.' }
    if (-not [Environment]::Is64BitOperatingSystem) { throw 'Stats needs 64-bit Windows.' }
    if (-not $env:LOCALAPPDATA) { throw 'LOCALAPPDATA is not set; run this as a normal signed-in user.' }
    try {
        [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    } catch { }

    # --- where to download from ----------------------------------------------
    $Version = "$Version".Trim().TrimStart('v')
    $local = $false
    if ($Source) {
        $base = $Source.TrimEnd('/', '\')
        $local = Test-Path -LiteralPath $base -PathType Container
        if (-not $Version) { $Version = 'local' }
    } else {
        if (-not $Version) {
            try {
                $release = Invoke-RestMethod -UseBasicParsing -Headers @{ 'User-Agent' = 'stats-installer' } "https://api.github.com/repos/$repo/releases/latest"
                $Version = "$($release.tag_name)".TrimStart('v')
            } catch { }
        }
        if ($Version) { $base = "https://github.com/$repo/releases/download/v$Version" }
        else { $base = "https://github.com/$repo/releases/latest/download"; $Version = 'latest' }
    }

    $work = Join-Path ([IO.Path]::GetTempPath()) ("stats-install-" + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Force $work | Out-Null
    try {
        Step 1 "Downloading Stats $Version"
        foreach ($name in $files + 'SHA256SUMS') {
            $dest = Join-Path $work $name
            if ($local) {
                Copy-Item -LiteralPath (Join-Path $base $name) $dest
            } else {
                try { Invoke-WebRequest -UseBasicParsing -Uri "$base/$name" -OutFile $dest }
                catch { throw "Could not download $base/$name. Check your connection, or that release '$Version' exists at https://github.com/$repo/releases. ($($_.Exception.Message))" }
            }
        }

        Step 2 'Verifying checksums'
        $sums = @{}
        foreach ($line in Get-Content (Join-Path $work 'SHA256SUMS')) {
            $parts = $line.Trim() -split '\s+', 2
            if ($parts.Count -eq 2) { $sums[$parts[1].TrimStart('*')] = $parts[0].ToLowerInvariant() }
        }
        foreach ($name in $files) {
            if (-not $sums.ContainsKey($name)) { throw "SHA256SUMS has no entry for $name. Nothing was changed." }
            $got = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $work $name)).Hash.ToLowerInvariant()
            if ($got -ne $sums[$name]) { throw "Checksum mismatch for $name (expected $($sums[$name]), got $got). Nothing was changed." }
        }

        Step 3 'Stopping the running copy'
        # Only our own copy: another program could also be called stats.exe.
        Get-Process -Name stats -ErrorAction SilentlyContinue |
            Where-Object { $_.Path -and ($_.Path -ieq $exePath) } |
            ForEach-Object { $_ | Stop-Process -Force; $_.WaitForExit(5000) | Out-Null }

        Step 4 "Installing to $installDir"
        New-Item -ItemType Directory -Force $installDir | Out-Null
        foreach ($name in $files) {
            $target = Join-Path $installDir $name
            $done = $false
            for ($try = 0; -not $done; $try++) {
                try { Copy-Item -LiteralPath (Join-Path $work $name) $target -Force; $done = $true }
                catch { if ($try -ge 10) { throw "Could not replace $target (still in use?): $($_.Exception.Message)" }; Start-Sleep -Milliseconds 500 }
            }
        }
        Set-Content -LiteralPath (Join-Path $installDir 'version.txt') -Value $Version -Encoding ASCII

        # Start with Windows: same value the tray menu toggle writes.
        Set-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name 'Stats' -Value "`"$exePath`""

        # Start Menu shortcut.
        $programs = [Environment]::GetFolderPath('Programs')
        $shell = New-Object -ComObject WScript.Shell
        $lnk = $shell.CreateShortcut((Join-Path $programs 'Stats.lnk'))
        $lnk.TargetPath = $exePath
        $lnk.WorkingDirectory = $installDir
        $lnk.Description = 'Stats system monitor'
        $lnk.Save()

        # Settings > Apps entry, so it can be removed like any other app.
        $uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Stats'
        New-Item -Force $uninstallKey | Out-Null
        $uninstaller = Join-Path $installDir 'uninstall.ps1'
        $sizeKb = [int]((Get-ChildItem -LiteralPath $installDir -File | Measure-Object Length -Sum).Sum / 1KB)
        $entries = @{
            DisplayName     = 'Stats'
            DisplayVersion  = $Version
            Publisher       = 'Stats'
            DisplayIcon     = $exePath
            InstallLocation = $installDir
            URLInfoAbout    = 'https://matolojr.github.io/stats/'
            UninstallString = "powershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$uninstaller`""
        }
        foreach ($k in $entries.Keys) { Set-ItemProperty $uninstallKey -Name $k -Value $entries[$k] }
        foreach ($k in 'NoModify', 'NoRepair') { New-ItemProperty $uninstallKey -Name $k -Value 1 -PropertyType DWord -Force | Out-Null }
        New-ItemProperty $uninstallKey -Name 'EstimatedSize' -Value $sizeKb -PropertyType DWord -Force | Out-Null

        if ($NoStart) {
            Step 5 'Installed (not started)'
        } else {
            Step 5 'Starting Stats'
            Start-Process -FilePath $exePath -WorkingDirectory $installDir
        }
    } finally {
        Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
    }

    Write-Host ''
    Write-Host "Stats $Version is installed and starts with Windows."
    Write-Host 'Look for the bar above the taskbar and the tray icon. Settings: right-click the bar > Settings.'
    Write-Host 'Remove it from Settings > Apps > Installed apps, or: irm https://matolojr.github.io/stats/uninstall.ps1 | iex'
}

try {
    Install-Stats -Version $Version -Source $Source -NoStart ([bool]$NoStart)
} catch {
    Write-Host ''
    Write-Host "Stats was not installed: $($_.Exception.Message)" -ForegroundColor Red
    # Under "irm | iex" an exit would close the user's window, so only set the code when run as a file.
    if ($MyInvocation.MyCommand.Path) { exit 1 }
}
