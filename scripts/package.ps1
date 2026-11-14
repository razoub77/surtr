<#
.SYNOPSIS
    Builds surtr.zip: the mpv config tree a stranger extracts and merges.

.DESCRIPTION
    The archive is exactly the install layout (issue 05):

        scripts/surtr/main.lua
        scripts/surtr/surtr.exe
        scripts/surtr/README.md
        script-opts/surtr.conf

    surtr.exe is the release build of this repo and carries no console window
    (Cargo.toml sets the windows subsystem for release): a detached daemon
    must not flash a console behind mpv.

.EXAMPLE
    powershell -File scripts/package.ps1
    powershell -File scripts/package.ps1 -SkipBuild -OutFile dist/surtr.zip
#>
param(
    [string]$OutFile = 'dist/surtr.zip',
    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $root 'target/release/surtr.exe'
$out = if ([IO.Path]::IsPathRooted($OutFile)) { $OutFile } else { Join-Path $root $OutFile }

if (-not $SkipBuild) {
    Push-Location $root
    try {
        & cargo build --release
        if ($LASTEXITCODE -ne 0) { throw "cargo build --release failed ($LASTEXITCODE)" }
    }
    finally { Pop-Location }
}
if (-not (Test-Path -LiteralPath $exe)) { throw "missing $exe; run without -SkipBuild" }

# Everything goes into a staging folder first, so the archive can only ever
# contain the install layout and nothing else from the repo.
$stage = Join-Path ([IO.Path]::GetTempPath()) ("surtr-zip-" + [guid]::NewGuid())
$files = [ordered]@{
    'scripts/surtr/main.lua'   = Join-Path $root 'scripts/surtr/main.lua'
    'scripts/surtr/surtr.exe'  = $exe
    'scripts/surtr/README.md'  = Join-Path $root 'scripts/surtr/README.md'
    'script-opts/surtr.conf'   = Join-Path $root 'script-opts/surtr.conf'
}
try {
    foreach ($name in $files.Keys) {
        $source = $files[$name]
        if (-not (Test-Path -LiteralPath $source)) { throw "missing $source" }
        $target = Join-Path $stage $name
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $target) | Out-Null
        Copy-Item -LiteralPath $source -Destination $target
    }
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $out) | Out-Null
    if (Test-Path -LiteralPath $out) { Remove-Item -LiteralPath $out -Force }
    Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $out
}
finally {
    if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
}

Write-Output "wrote $out"
Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [IO.Compression.ZipFile]::OpenRead($out)
try {
    $archive.Entries | Where-Object { $_.Name } | ForEach-Object { Write-Output ("  " + $_.FullName) }
}
finally { $archive.Dispose() }
