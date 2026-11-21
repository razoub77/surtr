<#
.SYNOPSIS
    Verifies a packaged surtr.zip against the README's install walkthrough.

.DESCRIPTION
    Extracts the archive into a fresh, empty mpv configuration directory (the
    same thing a stranger does by hand), then checks every install-time fact
    the README tells them about:

      * exactly the four install-layout files are present: three in
        scripts/surtr and script-opts/surtr.conf;
      * surtr.conf ships commented, with anime_roots=, binary_path=,
        overrides_path=~~/script-opts/surtr-overrides.json, client_id=,
        details_title_lang=, pause_timeout=0 and toggle_key=ctrl+shift+a;
      * README.md has exactly the eight sections, in order;
      * main.lua resolves the binary beside itself by default, expands
        binary_path through mpv's expand-path, forwards the default override
        file beside the conf, refuses a relative path, and spawns nothing when
        the binary is missing;
      * the daemon binary links the Windows GUI subsystem (a detached daemon
        must never open a console behind mpv).

    Needs `lua` on PATH for the launcher checks; pass -SkipLuaRun to skip
    them. Live mpv and Discord are not exercised: the README's verify step (a
    card appears) still needs a human, and this script says so.

.EXAMPLE
    powershell -File scripts/package.ps1
    powershell -File scripts/verify-package.ps1 -Zip dist/surtr.zip
#>
param(
    [string]$Zip = 'dist/surtr.zip',
    [switch]$SkipLuaRun
)

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$zipPath = if ([IO.Path]::IsPathRooted($Zip)) { $Zip } else { Join-Path $root $Zip }

$script:failures = 0
function Check([string]$name, [bool]$ok, [string]$detail = '') {
    if ($ok) { Write-Output "  ok    $name" }
    else {
        Write-Output "  FAIL  $name"
        if ($detail) { Write-Output "        $detail" }
        $script:failures++
    }
}

if (-not (Test-Path -LiteralPath $zipPath)) { throw "missing $zipPath; run scripts/package.ps1 first" }

# 1. A fresh mpv configuration folder, and the archive merged into it.
$config = Join-Path ([IO.Path]::GetTempPath()) ("surtr-mpv-config-" + [guid]::NewGuid())
New-Item -ItemType Directory -Force -Path $config | Out-Null
Write-Output "extracting $zipPath into a clean config folder: $config"
# Not Expand-Archive: Windows PowerShell 5.1's version cannot overwrite the
# hidden files a merged archive may contain.
Add-Type -AssemblyName System.IO.Compression.FileSystem
[IO.Compression.ZipFile]::ExtractToDirectory($zipPath, $config)

try {
    $scripts  = Join-Path $config 'scripts/surtr'
    $confPath = Join-Path $config 'script-opts/surtr.conf'
    $lua      = Join-Path $scripts 'main.lua'
    $exe      = Join-Path $scripts 'surtr.exe'
    $readme   = Join-Path $scripts 'README.md'

    # 2. The install layout, and nothing else.
    Write-Output 'layout'
    $expected = @('main.lua', 'surtr.exe', 'README.md') | Sort-Object
    $found = @(Get-ChildItem -LiteralPath $scripts -File | ForEach-Object Name) | Sort-Object
    Check 'scripts/surtr holds main.lua, surtr.exe, README.md' (
        (($found -join ',') -eq ($expected -join ','))) ("found: " + ($found -join ', '))
    Check 'script-opts/surtr.conf exists' (Test-Path -LiteralPath $confPath)
    $confDirs = @(Get-ChildItem -LiteralPath $config -Directory | ForEach-Object Name) | Sort-Object
    Check 'config folder holds only scripts/ and script-opts/' (
        (($confDirs -join ',') -eq 'script-opts,scripts')) ("found: " + ($confDirs -join ', '))

    # 3. The conf ships commented, with every key at its documented default.
    Write-Output 'configuration'
    $conf = Get-Content -LiteralPath $confPath -Raw
    $keys = @{}
    foreach ($line in ($conf -split "`r?`n")) {
        if ($line -match '^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=(.*)$') {
            $keys[$Matches[1]] = $Matches[2].Trim()
        }
    }
    Check 'anime_roots= ships blank' ($keys.ContainsKey('anime_roots') -and $keys['anime_roots'] -eq '')
    Check 'binary_path= ships blank' ($keys.ContainsKey('binary_path') -and $keys['binary_path'] -eq '')
    Check 'overrides_path= ships beside the conf' ($keys.ContainsKey('overrides_path') -and $keys['overrides_path'] -eq '~~/script-opts/surtr-overrides.json')
    Check 'toggle_key=ctrl+shift+a' ($keys.ContainsKey('toggle_key') -and $keys['toggle_key'] -eq 'ctrl+shift+a')
    Check 'client_id= ships blank' ($keys.ContainsKey('client_id') -and $keys['client_id'] -eq '')
    Check 'details_title_lang= ships blank' ($keys.ContainsKey('details_title_lang') -and $keys['details_title_lang'] -eq '')
    Check 'pause_timeout=0 ships the timeout off' ($keys.ContainsKey('pause_timeout') -and $keys['pause_timeout'] -eq '0')
    Check 'only those seven keys are active' ($keys.Count -eq 7) ("found: " + ($keys.Keys -join ', '))
    Check 'the file is commented (explains anime_roots)' ($conf -match '(?m)^#.*anime_roots')

    # 4. The README has exactly the eight sections, in order. Comparison drops
    # punctuation and case, so "Updating / Uninstalling" matches
    # "Updating/Uninstalling".
    Write-Output 'README'
    $headings = @()
    foreach ($line in (Get-Content -LiteralPath $readme)) {
        if ($line -match '^##\s+(?!#)(.+?)\s*$') {
            $headings += $Matches[1] -replace '^\d+\.\s*', ''
        }
    }
    $want = @('What it is', 'Install', 'Configure', 'Verify', 'Troubleshooting', 'Updating/Uninstalling', 'Known Limits', 'Overrides')
    $key = { param($s) ($s -replace '[^A-Za-z]', '').ToLowerInvariant() }
    $got = @($headings | ForEach-Object { & $key $_ })
    $wantKey = @($want | ForEach-Object { & $key $_ })
    $numbered = (($headings -join '') -match '^\d')
    Check 'exactly the eight sections, in order' (
        (($got -join '|') -eq ($wantKey -join '|')) -and -not $numbered) (
        "found: " + ($headings -join ' | '))
    $text = Get-Content -LiteralPath $readme -Raw
    foreach ($needle in @('%APPDATA%\mpv', 'portable_config', 'scripts/surtr', 'script-opts/surtr.conf',
            '%LOCALAPPDATA%\surtr', 'overrides.json', 'ctrl+shift+a', 'binary_path',
            'pause_timeout', 'releases/latest/download/install.ps1')) {
        Check "README names $needle" ($text.Contains($needle))
    }

    # 5. The daemon is a GUI-subsystem executable: no console flashes behind mpv.
    Write-Output 'daemon binary'
    $bytes = [IO.File]::ReadAllBytes($exe)
    $pe = [BitConverter]::ToInt32($bytes, 0x3C)
    $magic = [BitConverter]::ToUInt16($bytes, $pe + 24)
    $system = if ($magic -eq 0x20B -or $magic -eq 0x10B) {
        [BitConverter]::ToUInt16($bytes, $pe + 24 + 68)
    } else { 0 }
    Check 'release build links the Windows GUI subsystem (no console)' ($system -eq 2) (
        "subsystem field: $system (2 = GUI, 3 = console)")

    # 6. main.lua's startup gates, driven by a fake mp (no mpv needed).
    Write-Output 'launcher'
    if (-not $SkipLuaRun) {
        $luaExe = (Get-Command lua -ErrorAction SilentlyContinue).Source
        if ($null -eq $luaExe) {
            Write-Output '  skip  lua not found on PATH; launcher gates not run'
        }
        else {
            $harness = Join-Path ([IO.Path]::GetTempPath()) ("surtr-harness-" + [guid]::NewGuid() + '.lua')
            $results = Join-Path ([IO.Path]::GetTempPath()) ("surtr-harness-" + [guid]::NewGuid() + '.txt')
            $inputs = Join-Path ([IO.Path]::GetTempPath()) ("surtr-harness-" + [guid]::NewGuid() + '.txt')
            @'
-- Fake `mp` for scripts/surtr/main.lua: records what the launcher did.
-- Inputs come from the file named as the first argument, one KEY=VALUE per
-- line, so values keep their exact characters (`;`, trailing spaces, `\`).
local function read_inputs()
    local inputs = {}
    for line in io.lines(arg[1]) do
        local key, value = line:match('^([%w_]+)=(.*)$')
        if key then inputs[key] = value end
    end
    return inputs
end

local inputs = read_inputs()
local script_dir, conf = inputs.dir, inputs.conf
local user_data = inputs.userdata == '1'
local existing = inputs.existing
local absolute = inputs.absolute
local config = inputs.config
local function norm(path)
    if not path then return nil end
    return (path:lower():gsub('\\', '/'))
end

local missing = inputs.missing == '1'
local function same_path(a, b) return norm(a) == norm(b) end
local out = assert(io.open(inputs.log, 'w'))
-- Trailing spaces from empty optional arguments are trimmed: each recorded
-- line is exactly the text the launcher emitted.
local function note(...)
    local line = (table.concat({ ... }, ' '):gsub('%s+$', ''))
    out:write(line .. '\n')
end

local m = {}
local values = { anime_roots = '', binary_path = '', toggle_key = 'ctrl+shift+a' }
for line in io.lines(conf) do
    local k, v = line:match('^%s*([%w_]+)%s*=(.*)$')
    if k then values[k] = v:gsub('^%s+', ''):gsub('%s+$', '') end
end

package.preload['mp'] = function() return m end
package.preload['mp.msg'] = function()
    return { error = function(t) note('error', t) end, warn = function(t) note('warn', t) end,
             info = function(t) note('info', t) end, verbose = function(t) note('verbose', t) end }
end
local utils = {
    file_info = function(path)
        if not missing and same_path(path, existing) then return { is_dir = false } end
    end,
    subprocess_detached = function(args)
        note('spawn', table.concat(args.args, ' '))
    end,
}
-- Real mpv's `mp` table has no `utils` field: the module is only reachable
-- through `require 'mp.utils'`, which the daemon code must do.
package.preload['mp.utils'] = function() return utils end
package.preload['mp.options'] = function()
    return { read_options = function(t) for k, v in pairs(values) do t[k] = v end end }
end

function m.register_script_message() end
function m.get_property_native(name) if name == 'user-data' then return user_data and {} or nil end end
function m.get_property(name) return name == 'mpv-version' and 'mpv 0.42.0' or 'C:\\media\\ep.mkv' end
function m.get_property_number() return 4242 end
function m.get_script_directory() return script_dir end
function m.set_property() end
function m.set_property_bool() end
function m.osd_message() end
function m.add_key_binding(key) note('bind', key) end
function m.command_native(cmd)
    if cmd[1] == 'expand-path' then
        if cmd[2] == '~~/script-opts/surtr-overrides.json' then
            -- The override default is not part of the binary_path assertions.
            return config .. '/script-opts/surtr-overrides.json'
        end
        if cmd[2]:match('^%a:[/\\]') or cmd[2]:match('^[\\/][\\/]') then
            -- mpv returns an absolute expand-path argument unchanged.
            return cmd[2]
        end
    end
    note('expand-path', cmd[2])
    -- The exe named by binary_path, as mpv's expand-path would return it.
    return inputs.absolute
end

dofile(script_dir .. '/main.lua')
out:close()
'@ | Set-Content -LiteralPath $harness -Encoding ASCII

            $blankConf = Join-Path ([IO.Path]::GetTempPath()) ("surtr-conf-" + [guid]::NewGuid() + '.conf')
            Set-Content -LiteralPath $blankConf -Value "anime_roots=`nbinary_path=`ntoggle_key=ctrl+shift+a" -Encoding ASCII

            function New-Conf([string]$body) {
                $path = Join-Path ([IO.Path]::GetTempPath()) ("surtr-conf-" + [guid]::NewGuid() + '.conf')
                Set-Content -LiteralPath $path -Value $body -Encoding ASCII
                return $path
            }

            # Comparison form for paths: one case, one separator.
            function ConvertTo-PathKey([string]$path) {
                return ($path -replace '\\', '/').ToLowerInvariant()
            }

            function Invoke-Launcher([string]$label, [hashtable]$options) {
                if ($null -eq $options) { $options = @{} }
                # mpv's get_script_directory has no trailing separator; the
                # shipped launcher appends one itself. The harness gives Lua
                # the separator-free form so a doubled or missing `/` fails
                # the check instead of being hidden.
                $sd = if ($options.ContainsKey('script_dir')) { $options['script_dir'] } else { $scripts.TrimEnd('\', '/') }
                $cf = if ($options.ContainsKey('conf')) { $options['conf'] } else { $blankConf }
                $abs = if ($options.ContainsKey('absolute')) { $options['absolute'] } else { '' }
                $ud = if ($options.ContainsKey('user_data') -and $options['user_data']) { '1' } else { '0' }
                $ex = if ($options.ContainsKey('existing')) { $options['existing'] } else { $exe }
                $miss = if ($options.ContainsKey('missing') -and $options['missing']) { '1' } else { '0' }
                $lines = @("dir=$sd", "conf=$cf", "config=$config", "userdata=$ud", "existing=$ex", "missing=$miss", "absolute=$abs", "log=$results")
                [IO.File]::WriteAllLines($inputs, $lines, [Text.Encoding]::ASCII)
                if (Test-Path -LiteralPath $results) { Remove-Item -LiteralPath $results -Force }
                & $luaExe $harness $inputs | Out-Null
                if (Test-Path -LiteralPath $results) {
                    return @(Get-Content -LiteralPath $results)
                }
                return @()
            }

            $want = Join-Path $scripts 'surtr.exe'
            $wantKey = ConvertTo-PathKey $want
            $spawned = [regex]::new('^info watching for playback \(IPC \\\\\.\\pipe\\surtr-4242\)$')
            $spawnAny = [regex]::new('^spawn ')
            $bindAny = [regex]::new('^bind ')
            $expandAny = [regex]::new('^expand-path ')
            $errorAny = [regex]::new('^error ')

            # A clean extract with a filled-in conf: the shipped default.
            $log = Invoke-Launcher 'clean extract' @{ user_data = $true }
            $spawn = @($log | Where-Object { $spawnAny.IsMatch($_) })
            $spawnedExe = @($spawn | Where-Object { (ConvertTo-PathKey $_).Contains($wantKey) })
            Check 'default: spawns surtr.exe beside main.lua' (
                $spawnedExe.Length -eq 1 -and @($log | Where-Object { $spawned.IsMatch($_) }).Length -eq 1) (
                "log: " + ($log -join ' / '))
            Check 'default: no expand-path call, no error' (
                @($log | Where-Object { $expandAny.IsMatch($_) -or $errorAny.IsMatch($_) }).Length -eq 0) (
                "log: " + ($log -join ' / '))
            $wantOverrides = ConvertTo-PathKey (Join-Path $config 'script-opts\surtr-overrides.json')
            Check 'default: forwards the override file beside surtr.conf' (
                $spawn.Length -eq 1 -and
                (ConvertTo-PathKey $spawn[0]).Contains($wantOverrides)) ("log: " + ($log -join ' / '))

            # Empty anime_roots: one startup notice naming the conf, but the
            # daemon still spawns and the Toggle still binds.
            $log = Invoke-Launcher 'empty roots' @{ user_data = $true }
            Check 'empty anime_roots: one warning naming script-opts/surtr.conf' (
                @($log | Where-Object { $_ -like '*no anime_roots configured*script-opts/surtr.conf*' }).Length -eq 1) (
                "log: " + ($log -join ' / '))
            Check 'empty anime_roots: still spawns and binds the Toggle' (
                @($log | Where-Object { $spawnAny.IsMatch($_) -or $bindAny.IsMatch($_) }).Length -eq 2) (
                "log: " + ($log -join ' / '))

            # anime_roots is forwarded verbatim; Rust splits, trims and drops
            # empty segments, so the launcher does not touch it.
            $log = Invoke-Launcher 'roots' @{
                user_data = $true
                conf = (New-Conf "anime_roots=D:\Anime; E:\Anime2 ;`ntoggle_key=ctrl+shift+a")
            }
            $rootsLine = @($log | Where-Object { $spawnAny.IsMatch($_) })
            Check 'anime_roots is forwarded as one argv entry' (
                $rootsLine.Length -eq 1 -and
                $rootsLine[0].Contains(' --anime-roots D:\Anime; E:\Anime2 ;')) ("log: " + ($log -join ' / '))

            # pause_timeout goes to the daemon as seconds, verbatim.
            $log = Invoke-Launcher 'pause timeout' @{
                user_data = $true
                conf = (New-Conf "pause_timeout=300")
            }
            $timeoutLine = @($log | Where-Object { $spawnAny.IsMatch($_) })
            Check 'pause_timeout from the conf is forwarded to the daemon' (
                $timeoutLine.Length -eq 1 -and
                $timeoutLine[0].Contains(' --pause-timeout 300')) ("log: " + ($log -join ' / '))

            # Missing binary: one error naming the path, no spawn, no Toggle.
            $log = Invoke-Launcher 'missing exe' @{ user_data = $true; missing = $true }
            $errors = @($log | Where-Object { $errorAny.IsMatch($_) })
            Check 'missing surtr.exe: one console error' ($errors.Length -eq 1) ("log: " + ($log -join ' / '))
            Check 'missing surtr.exe: names the path attempted' (
                $errors.Length -eq 1 -and
                ((ConvertTo-PathKey $errors[0]).Contains($wantKey))) (
                "log: " + ($log -join ' / '))
            Check 'missing surtr.exe: no spawn, no Toggle binding' (
                @($log | Where-Object { $spawnAny.IsMatch($_) -or $bindAny.IsMatch($_) }).Length -eq 0) (
                "log: " + ($log -join ' / '))

            # mpv older than 0.36: one error naming the version, nothing else.
            $log = Invoke-Launcher 'missing user-data' @{ user_data = $false }
            Check 'missing user-data: one error naming mpv-version' (
                @($log | Where-Object { $_ -like 'error Surtr needs mpv 0.36 or newer*mpv 0.42.0*' }).Length -eq 1 -and
                @($log | Where-Object { $spawnAny.IsMatch($_) -or $bindAny.IsMatch($_) }).Length -eq 0) (
                "log: " + ($log -join ' / '))

            # binary_path: expanded through expand-path, relative refused.
            $log = Invoke-Launcher 'binary_path' @{
                user_data = $true
                absolute = 'C:\Tools\surtr\surtr.exe'
                existing = 'C:\Tools\surtr\surtr.exe'
                conf = (New-Conf "binary_path=~~exe_dir/surtr/surtr.exe`ntoggle_key=ctrl+shift+a")
            }
            Check 'binary_path=~~exe_dir/... goes through expand-path' (
                ($log -contains 'expand-path ~~exe_dir/surtr/surtr.exe') -and
                @($log | Where-Object { $spawnAny.IsMatch($_) -and $_.Contains('C:\Tools\surtr\surtr.exe') }).Length -eq 1 -and
                @($log | Where-Object { $spawned.IsMatch($_) }).Length -eq 1) ("log: " + ($log -join ' / '))
            $log = Invoke-Launcher 'relative' @{
                user_data = $true
                conf = (New-Conf "binary_path=bin/surtr.exe")
            }
            Check 'relative binary_path: one error, no spawn' (
                @($log | Where-Object { $_ -like 'error *not absolute*' }).Length -eq 1 -and
                @($log | Where-Object { $spawnAny.IsMatch($_) }).Length -eq 0) ("log: " + ($log -join ' / '))

            # overrides_path: the default sits beside the conf, any absolute
            # target is forwarded, and a relative target is refused.
            $log = Invoke-Launcher 'custom overrides_path' @{
                user_data = $true
                conf = (New-Conf "overrides_path=D:\Config\mine.json")
            }
            Check 'absolute overrides_path is forwarded verbatim' (
                @($log | Where-Object { $spawnAny.IsMatch($_) -and $_.Contains('D:\Config\mine.json') }).Length -eq 1) ("log: " + ($log -join ' / '))
            $log = Invoke-Launcher 'relative overrides_path' @{
                user_data = $true
                conf = (New-Conf "overrides_path=overrides\mine.json")
            }
            Check 'relative overrides_path: one error, no overrides argument' (
                @($log | Where-Object { $_ -like 'error *overrides_path*not absolute*' }).Length -eq 1 -and
                @($log | Where-Object { $spawnAny.IsMatch($_) }).Length -eq 1 -and
                @($log | Where-Object { $spawnAny.IsMatch($_) -and $_.Contains('--overrides') }).Length -eq 0) ("log: " + ($log -join ' / '))

            # The Toggle key from the conf is the one bound.
            $log = Invoke-Launcher 'toggle key' @{
                user_data = $true
                conf = (New-Conf "toggle_key=alt+p")
            }
            Check 'toggle_key from the conf is bound' ($log -contains 'bind alt+p')

            Remove-Item -LiteralPath $harness, $results, $inputs, $blankConf -Force -ErrorAction SilentlyContinue
        }
    }
}

finally {
    Remove-Item -LiteralPath $config -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Output ''
if ($script:failures -gt 0) {
    Write-Output "FAILED: $($script:failures) check(s)"
    Write-Output 'The README verify step (a card appears on Discord while an episode plays) needs live mpv and Discord.'
    exit 1
}
Write-Output 'All package checks passed.'
Write-Output 'Not covered here: live mpv + Discord (README section 4: play an episode under a root and see the card).'
