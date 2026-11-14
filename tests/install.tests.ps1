<#
.SYNOPSIS
    Tests scripts/install.ps1's conf sync (Merge-SurtrConf) at its seam:
    template conf + existing conf text -> merged conf text.

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

Write-Output ''
if ($failures -gt 0) {
    Write-Output "FAILED: $failures check(s)"
    exit 1
}
Write-Output 'All installer conf-sync checks passed.'
