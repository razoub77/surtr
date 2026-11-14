<#
.SYNOPSIS
    Installs, updates or uninstalls Surtr into mpv's configuration folder.

.DESCRIPTION
    One script for all three, so one URL serves them:

      install/update   irm https://github.com/razoub77/surtr/releases/latest/download/install.ps1 | iex
      uninstall        $env:SURTR_UNINSTALL = '1'; irm ... | iex
      (or)             powershell -File install.ps1 [-Uninstall]

    It detects mpv's configuration folder (MPV_HOME, %APPDATA%\mpv, the
    portable_config next to an mpv.exe on PATH, Scoop's layout), downloads the
    latest surtr.zip release asset, overwrites the plugin files, and rewrites
    script-opts/surtr.conf from the shipped template: surviving keys keep the
    user's value, keys that no longer exist are dropped, new keys arrive at
    their shipped defaults, and comments always match the shipped file.

    Nothing is written outside mpv's configuration folder and
    %LOCALAPPDATA%\surtr. State files (cache.json, overrides.json, owner.json)
    are never touched. A locked surtr.exe (mpv running) is a prompt to close
    mpv and press Enter, not a killed process.

    -SelfTest defines the functions and runs nothing: tests/install.tests.ps1
    dot-sources the script that way.

.EXAMPLE
    powershell -File scripts/install.ps1
    powershell -File scripts/install.ps1 -Uninstall
#>
param(
    [switch]$Uninstall,
    [switch]$SelfTest
)

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

$script:Repo = 'razoub77/surtr'
$script:LatestBase = "https://github.com/$script:Repo/releases/latest/download"
$script:ZipUrl = "$script:LatestBase/surtr.zip"

# --- conf sync ---------------------------------------------------------------

function Merge-SurtrConf {
    # The shipped template is the source of truth for the conf's shape:
    # its comments and key order are the result; existing confs only donate
    # values for keys the template still has. Returns CRLF-joined text with
    # one trailing newline, whatever line endings went in.
    param(
        [string]$Template,
        [string]$Existing
    )

    $user = @{}
    foreach ($line in ($Existing -split '\r?\n')) {
        if ($line -match '^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=(.*)$') {
            $user[$Matches[1]] = $Matches[2]
        }
    }

    $out = @()
    foreach ($line in ($Template -split '\r?\n')) {
        if ($line -match '^([A-Za-z_][A-Za-z0-9_]*)\s*=(.*)$') {
            $key = $Matches[1]
            if ($user.ContainsKey($key)) { $out += "$key=$($user[$key])" }
            else { $out += $line }
        }
        else { $out += $line }
    }

    # Exactly one trailing newline, whatever the template ended with.
    $text = ($out -join "`r`n")
    $text = $text.TrimEnd("`r", "`n")
    return ($text + "`r`n")
}

# --- mpv configuration folder detection --------------------------------------

function Find-SurtrMpvConfigs {
    # Every mpv configuration folder this machine shows. Keys are the folder
    # paths, values are where each was found. Nothing is a hard error: the
    # caller prompts when the result is empty or ambiguous.
    $found = [ordered]@{}

    if ($env:MPV_HOME -and (Test-Path -LiteralPath $env:MPV_HOME)) {
        $found[(Resolve-Path -LiteralPath $env:MPV_HOME).Path] = 'MPV_HOME'
    }

    $standard = if ($env:APPDATA) { Join-Path $env:APPDATA 'mpv' } else { $null }
    if ($standard -and (Test-Path -LiteralPath $standard)) {
        $found[$standard] = '%APPDATA%\mpv'
    }

    # Every mpv.exe on PATH, not just the first: Scoop puts a shim in ~\scoop\shims
    # and the real exe under apps\mpv\current, and portable_config sits beside
    # the real one.
    $mpvs = @(Get-Command mpv.exe -All -ErrorAction SilentlyContinue |
        Where-Object { $_.Source -and (Test-Path -LiteralPath $_.Source) })
    foreach ($mpv in $mpvs) {
        $portable = Join-Path (Split-Path -Parent $mpv.Source) 'portable_config'
        if (Test-Path -LiteralPath $portable) {
            $found[$portable] = 'portable_config (next to the mpv.exe on PATH)'
        }
    }

    $scoop = if ($env:USERPROFILE) { Join-Path $env:USERPROFILE 'scoop\apps\mpv\current\portable_config' } else { $null }
    if ($scoop -and (Test-Path -LiteralPath $scoop)) {
        $found[$scoop] = 'Scoop (scoop\apps\mpv\current\portable_config)'
    }

    return $found
}

function Test-SurtrConfigDir([string]$Path) {
    # A folder a stranger was told to type by hand still has to look like an
    # mpv configuration folder, not like C:\Windows.
    (Test-Path -LiteralPath (Join-Path $Path 'mpv.conf')) -or
    (Test-Path -LiteralPath (Join-Path $Path 'scripts')) -or
    (Test-Path -LiteralPath (Join-Path $Path 'script-opts')) -or
    ($Path -match 'portable_config$') -or
    ($Path -match '(?i)\bmpv$')
}

function Resolve-SurtrConfigDir {
    # One mpv configuration folder: silent when exactly one is found, a
    # numbered prompt when several, a typed path when none.
    $found = Find-SurtrMpvConfigs

    if ($found.Count -eq 1) {
        $path = @($found.Keys)[0]
        return @{ Path = $path; Source = $found[$path] }
    }

    if ($found.Count -gt 1) {
        Write-Host 'Surtr found several mpv configuration folders:'
        $i = 0
        foreach ($path in $found.Keys) {
            $i++
            Write-Host "  $i. $path  ($($found[$path]))"
        }
        while ($true) {
            $choice = (Read-Host "Install into which one? Type 1-$($found.Count), or a folder path").Trim()
            if ($choice -match '^\d+$' -and [int]$choice -ge 1 -and [int]$choice -le $found.Count) {
                $path = @($found.Keys)[[int]$choice - 1]
                return @{ Path = $path; Source = $found[$path] }
            }
            if ($choice -and (Test-Path -LiteralPath $choice)) {
                return @{ Path = (Resolve-Path -LiteralPath $choice).Path; Source = 'the folder you typed' }
            }
            Write-Host "  Not one of the folders above, and not a folder: '$choice'"
        }
    }

    while ($true) {
        $typed = (Read-Host "Surtr found no mpv configuration folder. Type mpv's configuration folder path (blank to quit)").Trim()
        if (-not $typed) { throw 'No mpv configuration folder given; nothing was installed.' }
        if (Test-Path -LiteralPath $typed) {
            if (Test-SurtrConfigDir $typed) {
                return @{ Path = (Resolve-Path -LiteralPath $typed).Path; Source = 'the folder you typed' }
            }
            Write-Host "  '$typed' has no mpv.conf, scripts\ or script-opts\ folder in it; that does not look like mpv's configuration folder."
            continue
        }
        Write-Host "  Not a folder: '$typed'"
    }
}

# --- copy with a close-mpv retry ---------------------------------------------

function Invoke-SurtrWithMpvRetry {
    # Run the scriptblock; if Windows refuses because the file is open (the
    # detached Daemon holds surtr.exe while mpv runs), ask the user to close
    # mpv and press Enter instead of killing the process under them.
    # Arguments reach the scriptblock positionally, so it never relies on
    # variable capture from the caller's scope.
    param([scriptblock]$Action, [string]$What, [object[]]$ArgumentList = @())

    while ($true) {
        try {
            & $Action @ArgumentList
            return
        }
        catch {
            # Windows reports a file open by another process differently by
            # API: UnauthorizedAccessException from Copy-Item, IOException from
            # [IO.File], or a wrapped RuntimeException whose message says so.
            # Walk the chain rather than trusting the outer type.
            $isLock = $false
            $ex = $_.Exception
            while ($null -ne $ex) {
                if ($ex -is [IO.IOException] -or
                    $ex -is [UnauthorizedAccessException] -or
                    $ex -is [System.IO.FileLoadException] -or
                    $ex.Message -match 'being used by another process|open by another process') {
                    $isLock = $true
                    break
                }
                $ex = $ex.InnerException
            }
            if (-not $isLock) { throw }
            $answer = (Read-Host 'Close mpv so Surtr can replace its files, then press Enter to retry (q to give up)').Trim()
            if ($answer -match '^q(uit)?$') { throw "Cancelled while replacing $What; the previous install was left in place." }
        }
    }
}

# --- install / uninstall ------------------------------------------------------

function Install-Surtr {
    $target = Resolve-SurtrConfigDir
    Write-Host ''
    Write-Host "Installing Surtr into $($target.Path)  ($($target.Source))"

    $work = Join-Path ([IO.Path]::GetTempPath()) ("surtr-install-" + [guid]::NewGuid())
    New-Item -ItemType Directory -Path $work | Out-Null
    try {
        Write-Host "Downloading the latest Surtr from $script:ZipUrl ..."
        $zip = Join-Path $work 'surtr.zip'
        Invoke-WebRequest -Uri $script:ZipUrl -OutFile $zip -UseBasicParsing
        Add-Type -AssemblyName System.IO.Compression.FileSystem
        [IO.Compression.ZipFile]::ExtractToDirectory($zip, $work)

        # Plugin files are replaced wholesale; the conf is synced, not replaced.
        $srcScripts = Join-Path $work 'scripts\surtr'
        $dstScripts = Join-Path $target.Path 'scripts\surtr'
        Invoke-SurtrWithMpvRetry -What 'surtr.exe' -ArgumentList @($srcScripts, $dstScripts) -Action {
            param($from, $to)
            New-Item -ItemType Directory -Force -Path $to | Out-Null
            foreach ($file in @('main.lua', 'surtr.exe', 'README.md')) {
                Copy-Item -LiteralPath (Join-Path $from $file) -Destination (Join-Path $to $file) -Force
            }
        }

        $confDir = Join-Path $target.Path 'script-opts'
        New-Item -ItemType Directory -Force -Path $confDir | Out-Null
        $confPath = Join-Path $confDir 'surtr.conf'
        $existing = if (Test-Path -LiteralPath $confPath) { [IO.File]::ReadAllText($confPath) } else { '' }
        $template = [IO.File]::ReadAllText((Join-Path $work 'script-opts\surtr.conf'))
        [IO.File]::WriteAllText($confPath, (Merge-SurtrConf -Template $template -Existing $existing),
            (New-Object System.Text.UTF8Encoding($false)))

        Write-Host ''
        Write-Host 'Done. Start Discord, then mpv.'
        Write-Host 'Point Surtr at your anime folders: open script-opts\surtr.conf in the'
        Write-Host 'configuration folder above and fill in, for example:'
        Write-Host '    anime_roots=D:\Anime;E:\Anime\Seasonal'
        Write-Host 'The first time surtr.exe starts, Windows SmartScreen may ask; choose'
        Write-Host 'More info, then Run anyway.'
    }
    finally {
        Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
    }
}

function Uninstall-Surtr {
    $found = Find-SurtrMpvConfigs
    if ($found.Count -gt 1) {
        Write-Host 'Surtr found several mpv configuration folders:'
        $i = 0
        foreach ($path in $found.Keys) { $i++; Write-Host "  $i. $path" }
        $targets = @($found.Keys)
    }
    elseif ($found.Count -eq 1) {
        $targets = @(@($found.Keys)[0])
    }
    else {
        $targets = @()
        Write-Host 'Surtr found no mpv configuration folder; only the Surtr data folder can be removed.'
    }

    foreach ($dir in $targets) {
        Invoke-SurtrWithMpvRetry -What 'surtr.exe' -ArgumentList @($dir) -Action {
            param($base)
            Remove-Item -LiteralPath (Join-Path $base 'scripts\surtr') -Recurse -Force -ErrorAction SilentlyContinue
            Remove-Item -LiteralPath (Join-Path $base 'script-opts\surtr.conf') -Force -ErrorAction SilentlyContinue
        }
        Write-Host "Removed Surtr from $dir"
    }

    $data = if ($env:LOCALAPPDATA) { Join-Path $env:LOCALAPPDATA 'surtr' } else { $null }
    if ($data -and (Test-Path -LiteralPath $data)) {
        Invoke-SurtrWithMpvRetry -What 'the Surtr data folder' -ArgumentList @($data) -Action {
            param($folder)
            Remove-Item -LiteralPath $folder -Recurse -Force
        }
        Write-Host "Removed $data (cache, overrides, ownership file)"
    }
    Write-Host 'Surtr is uninstalled. Nothing else was ever installed: no service, no registry entries, no scheduled task.'
}

# --- main ---------------------------------------------------------------------

if ($SelfTest) { return }

$doUninstall = [bool]$Uninstall -or ($env:SURTR_UNINSTALL -and $env:SURTR_UNINSTALL -ne '0')

Write-Host "Surtr install/update/uninstall  (github.com/$script:Repo)"
Write-Host ''
if ($doUninstall) { Uninstall-Surtr } else { Install-Surtr }
