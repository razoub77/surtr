<#
.SYNOPSIS
    Tests scripts/install.ps1's pure seams: the conf sync (Merge-SurtrConf:
    template conf + existing conf text -> merged conf text), the package check
    (Get-SurtrMissingFiles) and the menu frame (New-SurtrListFrame).

.DESCRIPTION
    Dependency-free: dot-sources the installer with -SelfTest (which defines
    its functions and runs nothing) and asserts on plain strings. The
    expected values are literals below, not recomputed from the code.

    Run:  powershell -File tests/install.tests.ps1
#>
Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

$root = $PSScriptRoot
$pkg = [IO.Path]::GetDirectoryName($root)
. (Join-Path $pkg 'scripts/install.ps1') -SelfTest

if (-not (Test-Path variable:failures)) { $failures = 0 }
function Check([string]$name, [bool]$ok, [string]$detail = '') {
    if ($ok) { Write-Output "  ok    $name" }
    else {
        Write-Output "  FAIL  $name"
        if ($detail) { Write-Output "        $detail" }
        $script:failures++
    }
}

# A representative shipped template: comments first, active keys last.
$template = @(
    '# Surtr configuration (mpv script-opts).',
    '# anime_roots: semicolon-separated folders holding anime.',
    'anime_roots=',
    '# client_id: Discord application id; empty uses the shipped one.',
    'client_id=',
    '# details_title_lang: english or romaji.',
    'details_title_lang=',
    'binary_path=',
    'toggle_key=ctrl+shift+a'
) -join "`r`n"

# A pre-one-liner conf a manual installer might leave behind: a stale key,
# a filled-in key, a customized toggle, and comments that no longer match.
$oldConf = @(
    '# my personal surtr notes',
    '# hover shows the title here',
    'hover_title_lang=romaji',
    'anime_roots=D:\Anime ;  E:\Anime\Seasonal ;',
    'toggle_key=alt+p'
) -join "`r`n"

$merged = Merge-SurtrConf -Template $template -Existing $oldConf
$lines = ($merged.TrimEnd("`r", "`n")) -split '\r?\n'
$templateLines = $template -split '\r?\n'
Check 'stale key hover_title_lang is dropped' (
    -not ($merged -match 'hover_title_lang'))

Check 'stale comments are replaced by the shipped ones' (
    -not ($merged -match 'my personal surtr notes') -and
    -not ($merged -match 'hover shows the title here') -and
    ($merged -match 'shipped one'))

Check 'surviving keys keep the user value verbatim' (
    ($lines -contains 'anime_roots=D:\Anime ;  E:\Anime\Seasonal ;') -and
    ($lines -contains 'toggle_key=alt+p'))

Check 'keys new to this version arrive at their shipped default' (
    ($lines -contains 'details_title_lang='))

# Shape, not text: a line's key (or its whole comment) must match the template
# at the same position, but the values are the user's by design.
$shape = {
    param($line)
    if ($line -match '^([A-Za-z_][A-Za-z0-9_]*)\s*=') { "key:" + $Matches[1] } else { "text:$line" }
}
# ForEach-Object <scriptblock> does not bind the item to a `param()` block, so
# call it explicitly; otherwise every line classifies as the same text.
$mergedShape = @($lines | ForEach-Object { & $shape $_ })
$templateShape = @($templateLines | ForEach-Object { & $shape $_ })
$shapeMismatch = -1
if ($mergedShape.Count -ne $templateShape.Count) {
    $shapeMismatch = [Math]::Min($mergedShape.Count, $templateShape.Count)
}
else {
    for ($i = 0; $i -lt $templateShape.Count; $i++) {
        if ($mergedShape[$i] -cne $templateShape[$i]) { $shapeMismatch = $i; break }
    }
}
Check 'every template line survives, in template order' ($shapeMismatch -lt 0) (
    "first mismatch at line $shapeMismatch; merged has $($mergedShape.Count) lines, template $($templateShape.Count)")

# A fresh install has no existing conf: the template passes through whole.
$fresh = Merge-SurtrConf -Template $template -Existing ''
Check 'no existing conf: the template is the result' (
    ((($fresh.TrimEnd("`r", "`n")) -split '\r?\n') -join '|') -eq ($templateLines -join '|'))

# A key the user set that also exists blank in the template: the user value wins.
$override = Merge-SurtrConf -Template $template -Existing "binary_path=D:\moved\surtr.exe"
Check 'user binary_path survives the sync' (
    ((($override.TrimEnd("`r", "`n")) -split '\r?\n') -contains 'binary_path=D:\moved\surtr.exe'))

# CRLF and LF existing files both merge; values keep their exact characters.
$lfConf = "anime_roots=E:\Anime;F:\More`ntoggle_key=F8"
$lf = Merge-SurtrConf -Template $template -Existing $lfConf
$lfLines = ($lf.TrimEnd("`r", "`n")) -split '\r?\n'
Check 'LF-ended existing conf merges the same way' (
    ($lfLines -contains 'anime_roots=E:\Anime;F:\More') -and
    ($lfLines -contains 'toggle_key=F8'))

# --- override file creation --------------------------------------------------
$overrideStage = Join-Path ([IO.Path]::GetTempPath()) ("surtr-override-test-" + [guid]::NewGuid())
try {
    New-Item -ItemType Directory -Force -Path $overrideStage | Out-Null
    $target = Join-Path $overrideStage 'surtr-overrides.json'
    $old = Join-Path $overrideStage 'old-overrides.json'

    Check 'no override file anywhere: the default file is created' (
        (New-SurtrOverridesFile -ConfDir $overrideStage -ExistingConf '' -LegacyOverridesPath $old) -and
        (Test-Path -LiteralPath $target) -and
        ((Get-Content -LiteralPath $target -Raw) -eq "{}`r`n"))

    Set-Content -LiteralPath $target -Value '{"anime a": 1}' -Encoding ASCII
    Check 'an existing surtr-overrides.json is never overwritten' (
        -not (New-SurtrOverridesFile -ConfDir $overrideStage -ExistingConf '' -LegacyOverridesPath $old) -and
        ((Get-Content -LiteralPath $target -Raw).Contains('anime a')))

    Remove-Item -LiteralPath $target
    Set-Content -LiteralPath $old -Value '{"anime b": 2}' -Encoding ASCII
    Check 'an old overrides.json leaves the move to the daemon' (
        -not (New-SurtrOverridesFile -ConfDir $overrideStage -ExistingConf '' -LegacyOverridesPath $old) -and
        -not (Test-Path -LiteralPath $target))

    $custom = "overrides_path=D:\Config\mine.json"
    Check 'a custom overrides_path creates no default file' (
        -not (New-SurtrOverridesFile -ConfDir $overrideStage -ExistingConf $custom -LegacyOverridesPath '') -and
        -not (Test-Path -LiteralPath $target))

    $explicitDefault = "overrides_path=$($script:DefaultOverridesPath)"
    Check 'the explicit default overrides_path still creates the file' (
        (New-SurtrOverridesFile -ConfDir $overrideStage -ExistingConf $explicitDefault -LegacyOverridesPath '') -and
        (Test-Path -LiteralPath $target))
}
finally { Remove-Item -LiteralPath $overrideStage -Recurse -Force -ErrorAction SilentlyContinue }

# --- package layout ----------------------------------------------------------
# A surtr.zip is trusted only if the install layout is inside it; a downloaded
# error page, or a truncated archive, must be reported by relative path.
$stage = Join-Path ([IO.Path]::GetTempPath()) ("surtr-installer-test-" + [guid]::NewGuid())
try {
    foreach ($rel in @('scripts/surtr/main.lua', 'scripts/surtr/surtr.exe',
            'scripts/surtr/README.md', 'script-opts/surtr.conf')) {
        $target = Join-Path $stage $rel
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $target) | Out-Null
        Set-Content -LiteralPath $target -Value ''
    }
    Check 'a full install layout is a package' (@(Get-SurtrMissingFiles -Root $stage).Count -eq 0)
    Remove-Item -LiteralPath (Join-Path $stage 'scripts/surtr/README.md')
    Check 'the missing file is named by its path inside the zip' (
        ((@(Get-SurtrMissingFiles -Root $stage) -join ',') -eq 'scripts/surtr/README.md'))
    Check 'nothing at all is not a package: all four files reported' (
        (@(Get-SurtrMissingFiles -Root (Join-Path $stage 'absent')).Count -eq 4))
}
finally { Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue }

# --- menu frame --------------------------------------------------------------
# The frame is text plus styles; the text is what the user reads, so assert on it.
function Get-FrameLines($frame) {
    return @($frame | ForEach-Object { (@($_) | ForEach-Object { $_.Text }) -join '' })
}

$targets = @(
    (New-SurtrItem -Label 'Install / Update'),
    (New-SurtrItem -Label 'Uninstall'),
    (New-SurtrItem -Label 'Type a folder path...' -Action $true)
)
$one = Get-FrameLines (New-SurtrListFrame -Items $targets -Current 0 -Checked ([bool[]]@($false, $false, $false)) `
        -Multi $false -Width 80 -Help (Get-SurtrHelp))
Check 'the focused row carries the pointer' ($one[0] -eq '> Install / Update') ("found: $($one[0])")
Check 'other rows stay aligned under it' ($one[1] -eq '  Uninstall') ("found: $($one[1])")
Check 'the hint line says Up/Down, Enter and Esc' (
    $one -contains '  Up/Down to move - Enter to choose - Esc to exit') ("found: $($one -join ' | ')")

$picked = Get-FrameLines (New-SurtrListFrame -Items $targets -Current 1 -Checked ([bool[]]@($false, $true, $false)) `
        -Multi $true -Width 80 -Help (Get-SurtrHelp -Multi))
Check 'a ticked row shows [x]' ($picked -contains '> [x] Uninstall') ("found: $($picked -join ' | ')")
Check 'an unticked row shows [ ]' ($picked -contains '  [ ] Install / Update') ("found: $($picked -join ' | ')")
Check 'an action row has no tick box' ($picked -contains '  Type a folder path...') ("found: $($picked -join ' | ')")
Check 'the hint line of a tick menu mentions Space' ($picked -contains '  Up/Down to move - Space to tick - Enter to confirm - Esc to exit') ("found: $($picked -join ' | ')")

$detected = @((New-SurtrItem -Label '%APPDATA%\mpv' -Detail @('C:\Users\me\AppData\Roaming\mpv') -Path 'C:\Users\me\AppData\Roaming\mpv'))
$withDetail = Get-FrameLines (New-SurtrListFrame -Items $detected -Current 0 -Checked ([bool[]]@($false)) `
        -Multi $true -Width 80 -Help '')
Check 'a detected folder shows its path under the label' (
    ($withDetail[1] -eq '      C:\Users\me\AppData\Roaming\mpv')) ("found: $($withDetail -join ' | ')")

Check 'a long path is shortened in the middle, never past the width' (
    (Format-SurtrFit -Text 'C:\Users\me\AppData\Roaming\mpv' -Max 20 -Middle).Length -le 20 -and
    (Format-SurtrFit -Text 'C:\Users\me\AppData\Roaming\mpv' -Max 20 -Middle).Contains('...'))

Write-Output ''
if ($failures -gt 0) {
    Write-Output "FAILED: $failures check(s)"
    exit 1
}
Write-Output 'All installer conf-sync checks passed.'