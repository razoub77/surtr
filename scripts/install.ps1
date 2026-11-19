<#
.SYNOPSIS
    Installs, updates or uninstalls Surtr in mpv's configuration folder.

.DESCRIPTION
    Opening it asks what to do: Install / Update or Uninstall. Arrow keys move,
    Enter chooses, Esc leaves. Without a console that can read keys the same
    choices are a numbered prompt.

      install/update   irm https://github.com/razoub77/surtr/releases/latest/download/install.ps1 | iex
      uninstall        same one-liner, pick Uninstall in the menu
      scripted        powershell -File install.ps1 [-Uninstall]   (skips the menu)

    With surtr.zip beside this script it installs from that file and never
    touches the network (offline install); otherwise it downloads the latest
    release asset.

    It detects mpv's configuration folders (MPV_HOME, %APPDATA%\mpv, the
    portable_config next to an mpv.exe on PATH, Scoop's layout) and asks which
    to use: Space ticks folders, Enter confirms, or type a folder path instead.
    Plugin files are overwritten and script-opts/surtr.conf is rewritten from
    the shipped template: surviving keys keep the user's value, keys that no
    longer exist are dropped, new keys arrive at their shipped defaults, and
    comments always match the shipped file. An empty
    script-opts/surtr-overrides.json is created only when the user has no
    override file there and no old one in %LOCALAPPDATA%\surtr; an existing
    override file is never overwritten.

    Uninstall removes the plugin from the ticked folders and asks before
    deleting %LOCALAPPDATA%\surtr (cache and ownership) - Enter keeps it. The
    override file in script-opts is left alone.

    Nothing is written outside mpv's configuration folder and
    %LOCALAPPDATA%\surtr. State files (cache.json, owner.json) are never
    touched unasked. A locked surtr.exe (mpv running) is a prompt to close mpv
    and press Enter, not a killed process.

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

# What surtr.zip must contain, relative to its root (scripts/package.ps1).
$script:Layout = @(
    'scripts/surtr/main.lua',
    'scripts/surtr/surtr.exe',
    'scripts/surtr/README.md',
    'script-opts/surtr.conf'
)

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

# The override-file path the shipped conf uses when the user has not moved it.
$script:DefaultOverridesPath = '~~/script-opts/surtr-overrides.json'

function Get-SurtrConfValue {
    # The last value for one key, trimmed. Absent or blank returns ''.
    param([string]$Existing, [string]$Key)
    $found = ''
    foreach ($line in ($Existing -split '\r?\n')) {
        if ($line -match '^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=(.*)$' -and $Matches[1] -eq $Key) {
            $found = $Matches[2].Trim()
        }
    }
    return $found
}

function New-SurtrOverridesFile {
    # Creates the default empty override file only when the user has none in
    # either place. An old appdata file is left for the daemon to move; a
    # custom overrides_path is the user's target, so the default stays absent.
    param([string]$ConfDir, [string]$ExistingConf, [string]$LegacyOverridesPath)

    $path = Join-Path $ConfDir 'surtr-overrides.json'
    if (Test-Path -LiteralPath $path) { return $false }
    if ($LegacyOverridesPath -and (Test-Path -LiteralPath $LegacyOverridesPath)) { return $false }
    $configured = Get-SurtrConfValue -Existing $ExistingConf -Key 'overrides_path'
    if ($configured -and $configured -ne $script:DefaultOverridesPath) { return $false }

    [IO.File]::WriteAllText($path, "{}`r`n", (New-Object System.Text.UTF8Encoding($false)))
    return $true
}

# --- console: colours, glyphs, one-key menus ---------------------------------

# ASCII until the console proves it can do better; the file stays ASCII-safe
# because every fancy glyph is a code point, not a literal.
$script:GlyphPointer = '>'
$script:GlyphSep = '-'
$script:GlyphArrows = 'Up/Down'
$script:GlyphDots = '...'
$script:UseUnicode = $false
$script:UseColor = $false
$script:UseVt = $false
# Set when the user backs out with Esc or Quit: the ending is theirs, so no
# "press Enter to close" afterwards.
$script:Cancelled = $false

# Surtr's red for focus, a darker danger red for errors, grey for detail.
$script:FocusColor = '38;2;255;77;77'
$script:DangerColor = '38;2;196;43;28'
$script:MutedColor = '90'

function Initialize-SurtrUi {
    # Once per run: decide colours and glyphs. A console that cannot take them
    # gets plain text, and the menus fall back to numbers (Test-SurtrKeys).
    try {
        $usable = ($Host.Name -eq 'ConsoleHost') -and
        -not [Console]::IsOutputRedirected -and
        [Environment]::UserInteractive -and
        -not $env:NO_COLOR -and -not $env:CI -and $env:TERM -ne 'dumb'
        if ($usable) {
            $script:UseColor = $true
            $script:UseVt = [bool]$Host.UI.SupportsVirtualTerminal
            $script:UseUnicode = ([Console]::OutputEncoding.CodePage -eq 65001) -or [bool]$env:WT_SESSION
        }
    }
    catch { }

    if ($script:UseUnicode) {
        $script:GlyphPointer = [string][char]0x203A
        $script:GlyphSep = [string][char]0x00B7
        $script:GlyphArrows = [string][char]0x2191 + '/' + [string][char]0x2193
        $script:GlyphDots = [string][char]0x2026
    }
}

function Write-SurtrStyle {
    # One piece of text in a style: 'plain', 'focus', 'danger' or 'muted'.
    # 24-bit colour where the console takes it, the nearest named colour
    # otherwise, nothing when colour is off.
    param([string]$Text, [string]$Style = 'plain', [switch]$NoNewline)

    if ($Style -eq 'plain' -or -not $script:UseColor) {
        Write-Host $Text -NoNewline:$NoNewline
        return
    }
    if ($script:UseVt) {
        $code = switch ($Style) {
            'focus' { $script:FocusColor }
            'danger' { $script:DangerColor }
            'muted' { $script:MutedColor }
            default { '' }
        }
        if (-not $code) { Write-Host $Text -NoNewline:$NoNewline; return }
        $esc = [string][char]27
        Write-Host ($esc + '[' + $code + 'm' + $Text + $esc + '[0m') -NoNewline:$NoNewline
        return
    }
    $named = switch ($Style) {
        'focus' { 'Red' }
        'danger' { 'DarkRed' }
        'muted' { 'DarkGray' }
        default { '' }
    }
    if (-not $named) { Write-Host $Text -NoNewline:$NoNewline; return }
    Write-Host $Text -NoNewline:$NoNewline -ForegroundColor $named
}

function Write-SurtrLine {
    param([string]$Text = '', [string]$Style = 'plain')
    Write-SurtrStyle -Text $Text -Style $Style
}

function Write-SurtrBanner {
    Write-Host '  ' -NoNewline
    Write-SurtrStyle 'Surtr' 'focus' -NoNewline
    Write-SurtrStyle ('  ' + $script:GlyphSep + ' github.com/' + $script:Repo) 'muted'
}

function Get-SurtrWidth {
    $width = 0
    try { $width = [Console]::WindowWidth } catch { }
    if ($width -lt 20) { $width = 80 }
    return $width
}

function Get-SurtrHelp {
    # The hint line under a menu, with the glyphs this console can draw.
    param([switch]$Multi, [switch]$Confirm)
    $sep = ' ' + $script:GlyphSep + ' '
    $parts = @()
    if ($Confirm) { $parts += 'Left/Right to choose' }
    else { $parts += ($script:GlyphArrows + ' to move') }
    if ($Multi) {
        $parts += 'Space to tick'
        $parts += 'Enter to confirm'
    }
    elseif (-not $Confirm) { $parts += 'Enter to choose' }
    $parts += 'Esc to exit'
    return '  ' + ($parts -join $sep)
}

function Format-SurtrFit {
    # Text shortened to $Max characters with an ellipsis, at the end or (paths)
    # in the middle. Never longer than $Max.
    param([string]$Text, [int]$Max, [switch]$Middle)

    if ($null -eq $Text -or $Max -lt 1) { return '' }
    if ($Text.Length -le $Max) { return $Text }
    $dots = $script:GlyphDots
    if ($Max -le $dots.Length) { return $Text.Substring(0, $Max) }
    $room = $Max - $dots.Length
    if ($Middle) {
        $head = [int][Math]::Floor($room / 2)
        $tail = $room - $head
        return $Text.Substring(0, $head) + $dots + $Text.Substring($Text.Length - $tail)
    }
    return $Text.Substring(0, $room) + $dots
}

function New-SurtrSegment {
    param([string]$Text, [string]$Style = 'plain')
    return @{ Text = $Text; Style = $Style }
}

function New-SurtrItem {
    # One row of a menu. Action rows are chosen with Enter instead of ticked;
    # Quit rows leave. Path is the folder the row stands for, if any.
    param(
        [string]$Label,
        [string[]]$Detail = @(),
        [bool]$Action = $false,
        [bool]$Quit = $false,
        [string]$Path = ''
    )
    return @{ Label = $Label; Detail = @($Detail); Action = $Action; Quit = $Quit; Path = $Path }
}

function Test-SurtrKeys {
    # Can this console hand us single keys? A redirected stdin cannot, and
    # reading is about to fail rather than answer - so ask before drawing.
    try {
        if ([Console]::IsInputRedirected) { return $false }
        $null = [Console]::KeyAvailable
        return $true
    }
    catch { return $false }
}

function Read-SurtrText {
    # Read-Host, trimmed and never $null: a closed stdin (piped, CI) reads as
    # an empty answer instead of failing on a method call.
    param([string]$Prompt)
    return ([string](Read-Host $Prompt)).Trim()
}

function Enter-SurtrMenuConsole {
    # While a menu is open: no cursor flicker, and Ctrl+C arrives as a key so
    # it exits the menu instead of killing a half-finished copy.
    $state = @{ Cursor = $true; CtrlC = $null }
    try { $state.Cursor = [Console]::CursorVisible } catch { }
    try { [Console]::CursorVisible = $false } catch { }
    try {
        $state.CtrlC = [Console]::TreatControlCAsInput
        [Console]::TreatControlCAsInput = $true
    }
    catch { }
    return $state
}

function Exit-SurtrMenuConsole {
    param($State)
    if ($null -eq $State) { return }
    if ($null -ne $State.CtrlC) {
        try { [Console]::TreatControlCAsInput = [bool]$State.CtrlC } catch { }
    }
    try { [Console]::CursorVisible = [bool]$State.Cursor } catch { }
}

function Clear-SurtrKeys {
    # Keys already waiting (a held arrow) must not answer the next menu.
    try {
        for ($i = 0; $i -lt 256 -and [Console]::KeyAvailable; $i++) { [void][Console]::ReadKey($true) }
    }
    catch { }
}

function Test-SurtrCancelKey {
    param($Key)
    return ($Key.Key -eq 'Escape') -or
    (($Key.Modifiers -band [ConsoleModifiers]::Control) -ne 0 -and $Key.Key -eq 'C')
}

function Step-SurtrItem {
    # Next row, wrapping at both ends.
    param([object[]]$Items, [int]$From, [int]$Step)
    $n = $Items.Count
    return (((($From + $Step) % $n) + $n) % $n)
}

function New-SurtrListFrame {
    # The menu as lines of coloured segments. Pure: same items, same text.
    param(
        [object[]]$Items,
        [int]$Current,
        [bool[]]$Checked,
        [bool]$Multi,
        [int]$Width,
        [string]$Help = ''
    )

    $lines = New-Object System.Collections.Generic.List[object]
    for ($i = 0; $i -lt $Items.Count; $i++) {
        $item = $Items[$i]
        $focused = ($i -eq $Current)
        $ticked = ($Multi -and $Checked[$i])
        $style = 'plain'
        if ($focused -or $ticked) { $style = 'focus' }
        $pointer = '  '
        if ($focused) { $pointer = $script:GlyphPointer + ' ' }
        $box = ''
        if ($Multi -and -not $item.Action -and -not $item.Quit) {
            if ($ticked) { $box = '[x] ' } else { $box = '[ ] ' }
        }
        $lines.Add([object[]]@(New-SurtrSegment ($pointer + $box + $item.Label) $style))
        $indent = ' ' * (2 + $box.Length)
        foreach ($detail in $item.Detail) {
            $room = $Width - 1 - $indent.Length
            $lines.Add([object[]]@(New-SurtrSegment ($indent + (Format-SurtrFit -Text $detail -Max $room -Middle)) 'muted'))
        }
    }
    if ($Help) {
        $lines.Add([object[]]@())
        $lines.Add([object[]]@(New-SurtrSegment $Help 'muted'))
    }
    return $lines.ToArray()
}

function Write-SurtrFrame {
    # Draw $Lines over the previous frame ($Previous lines tall): back up,
    # rewrite every line padded to the width (so nothing old sticks out), and
    # leave the cursor at the frame's top-left. Returns this frame's height.
    param([object[]]$Lines, [int]$Previous)

    $max = (Get-SurtrWidth) - 1
    if ($Previous -gt 0) {
        $top = [Console]::CursorTop - $Previous
        if ($top -lt 0) { $top = 0 }
        [Console]::SetCursorPosition(0, $top)
    }
    $count = $Lines.Count
    $total = [Math]::Max($count, $Previous)
    for ($i = 0; $i -lt $total; $i++) {
        $used = 0
        if ($i -lt $count) {
            foreach ($segment in $Lines[$i]) {
                $room = $max - $used
                if ($room -le 0) { break }
                $text = Format-SurtrFit -Text ([string]$segment.Text) -Max $room
                if ($text.Length -eq 0) { continue }
                Write-SurtrStyle -Text $text -Style $segment.Style -NoNewline
                $used += $text.Length
            }
        }
        # The padding ends the line: text is written without a newline, this
        # fills the rest of the row and breaks it, so a line is exactly $max
        # characters and the console never wraps it.
        Write-Host (' ' * [Math]::Max(0, $max - $used))
    }
    if ($total -gt $count) {
        [Console]::SetCursorPosition(0, [Math]::Max(0, [Console]::CursorTop - ($total - $count)))
    }
    return $count
}

function Read-SurtrMenuNumbers {
    # The same menu as a numbered prompt, for consoles that cannot read keys.
    # Single choice: one number. Multi: numbers separated by spaces.
    param([object[]]$Items, [bool]$Multi, [string]$Help)

    for ($i = 0; $i -lt $Items.Count; $i++) {
        $box = ''
        if ($Multi -and -not $Items[$i].Action -and -not $Items[$i].Quit) { $box = '[ ] ' }
        Write-Host ("  " + ($i + 1) + '. ' + $box + $Items[$i].Label)
        foreach ($detail in $Items[$i].Detail) { Write-SurtrLine ("      " + $detail) 'muted' }
    }
    Write-SurtrLine $Help 'muted'

    while ($true) {
        $hint = if ($Multi) { 'Numbers separated by spaces, then Enter' } else { 'Number, then Enter' }
        $line = Read-SurtrText ('  ' + $hint + ' (empty to quit)')
        if (-not $line) { return @{ Cancelled = $true; Index = -1; Ticked = @() } }
        $picked = @()
        $bad = $false
        foreach ($part in ($line -split '[\s,]+')) {
            if (-not $part) { continue }
            if ($part -notmatch '^\d+$') { $bad = $true; break }
            $n = [int]$part
            if ($n -lt 1 -or $n -gt $Items.Count) { $bad = $true; break }
            $picked += ($n - 1)
        }
        if ($bad -or $picked.Count -eq 0) {
            Write-SurtrLine '  Not one of the numbers above.' 'danger'
            continue
        }
        if ($picked.Count -eq 1) {
            $only = $Items[$picked[0]]
            if ($only.Action -or $only.Quit) {
                return @{ Cancelled = [bool]$only.Quit; Index = $picked[0]; Ticked = @() }
            }
        }
        if (-not $Multi) {
            return @{ Cancelled = $false; Index = $picked[0]; Ticked = @() }
        }
        return @{ Cancelled = $false; Index = -1; Ticked = $picked }
    }
}

function Read-SurtrMenu {
    # Arrow-key menu. Returns Cancelled, Index (row Enter was pressed on, -1
    # when rows were ticked) and Ticked (the ticked rows). Multi: Space ticks,
    # Enter confirms what is ticked, or the highlighted row when nothing is.
    param([object[]]$Items, [switch]$Multi, [string]$Help = '')

    $isMulti = [bool]$Multi
    if (-not $Help) { $Help = Get-SurtrHelp -Multi:$isMulti }
    if (-not (Test-SurtrKeys)) { return (Read-SurtrMenuNumbers -Items $Items -Multi $isMulti -Help $Help) }

    $checked = New-Object 'bool[]' $Items.Count
    $current = 0
    $drawn = 0
    $result = $null

    # A frame taller than the window cannot be redrawn in place: numbers instead,
    # decided before anything is written.
    try {
        $frame = @(New-SurtrListFrame -Items $Items -Current 0 -Checked $checked -Multi $isMulti -Width (Get-SurtrWidth) -Help $Help)
        $height = [Console]::WindowHeight
        if ($height -gt 0 -and $frame.Count -ge $height) { throw 'too tall' }
    }
    catch { return (Read-SurtrMenuNumbers -Items $Items -Multi $isMulti -Help $Help) }

    $console = Enter-SurtrMenuConsole
    try {
        Clear-SurtrKeys
        while ($null -eq $result) {
            $drawn = Write-SurtrFrame -Lines (New-SurtrListFrame -Items $Items -Current $current -Checked $checked -Multi $isMulti -Width (Get-SurtrWidth) -Help $Help) -Previous $drawn
            $key = [Console]::ReadKey($true)
            if (Test-SurtrCancelKey $key) {
                $result = @{ Cancelled = $true; Index = -1; Ticked = @() }
                continue
            }
            switch ($key.Key) {
                'UpArrow' { $current = Step-SurtrItem -Items $Items -From $current -Step -1 }
                'DownArrow' { $current = Step-SurtrItem -Items $Items -From $current -Step 1 }
                'Home' { $current = 0 }
                'End' { $current = $Items.Count - 1 }
                'Spacebar' {
                    if ($isMulti -and -not $Items[$current].Action) { $checked[$current] = -not $checked[$current] }
                }
                'Enter' {
                    $ticked = @(for ($i = 0; $i -lt $Items.Count; $i++) { if ($checked[$i]) { $i } })
                    $item = $Items[$current]
                    if ($isMulti -and -not $item.Action -and $ticked.Count -eq 0) { $ticked = @($current) }
                    if ($isMulti -and -not $item.Action) {
                        $result = @{ Cancelled = $false; Index = -1; Ticked = $ticked }
                    }
                    else {
                        $result = @{ Cancelled = $false; Index = $current; Ticked = $ticked }
                    }
                }
            }
        }
    }
    finally {
        # The menu becomes one line with the answer; nothing when cancelled.
        $summary = @()
        if ($null -ne $result -and -not $result.Cancelled) {
            $labels = @()
            if ($result.Index -ge 0) { $labels += $Items[$result.Index].Label }
            $labels += @($result.Ticked | ForEach-Object { $Items[$_].Label })
            if ($labels.Count -gt 0) {
                $summary = @(, [object[]]@(New-SurtrSegment ('  ' + $script:GlyphPointer + ' ' + ($labels -join ', ')) 'focus'))
            }
        }
        try { [void](Write-SurtrFrame -Lines $summary -Previous $drawn) } catch { }
        Exit-SurtrMenuConsole -State $console
    }
    return $result
}

function Read-SurtrConfirm {
    # Yes/No on one line: Left/Right (and Up/Down, Tab) choose, Enter answers,
    # Esc answers No - No is always the safe answer.
    param([string]$Question, [bool]$Default = $false)

    $delete = $Default
    $help = Get-SurtrHelp -Confirm
    $frame = {
        param($value)
        $keepPtr = '  '
        $deletePtr = '  '
        $keepStyle = 'plain'
        $deleteStyle = 'plain'
        if ($value) { $deletePtr = $script:GlyphPointer + ' '; $deleteStyle = 'focus' }
        else { $keepPtr = $script:GlyphPointer + ' '; $keepStyle = 'focus' }
        $segs = @(
            (New-SurtrSegment ($keepPtr + 'Keep') $keepStyle),
            (New-SurtrSegment '    '),
            (New-SurtrSegment ($deletePtr + 'Delete') $deleteStyle)
        )
        $lines = New-Object System.Collections.Generic.List[object]
        $lines.Add([object[]]$segs)
        $lines.Add([object[]]@())
        $lines.Add([object[]]@(New-SurtrSegment $help 'muted'))
        return $lines.ToArray()
    }

    if (-not (Test-SurtrKeys)) {
        Write-Host ('  ' + $Question)
        $answer = Read-SurtrText '  Keep it? Type y to delete, anything else keeps it'
        return ($answer -match '^(?i)y(es)?$')
    }

    $result = $null
    $drawn = 0
    $console = Enter-SurtrMenuConsole
    try {
        Write-Host ('  ' + $Question)
        Clear-SurtrKeys
        while ($null -eq $result) {
            $drawn = Write-SurtrFrame -Lines (& $frame $delete) -Previous $drawn
            $key = [Console]::ReadKey($true)
            if (Test-SurtrCancelKey $key) { $result = $false; continue }
            switch ($key.Key) {
                'LeftArrow' { $delete = $false }
                'RightArrow' { $delete = $true }
                'UpArrow' { $delete = -not $delete }
                'DownArrow' { $delete = -not $delete }
                'Tab' { $delete = -not $delete }
                'Enter' { $result = $delete }
            }
        }
    }
    finally {
        $label = 'Keep'
        $style = 'plain'
        if ($result) { $label = 'Delete'; $style = 'focus' }
        $summary = @(, [object[]]@(New-SurtrSegment ('  ' + $script:GlyphPointer + ' ' + $label) $style))
        try { [void](Write-SurtrFrame -Lines $summary -Previous $drawn) } catch { }
        Exit-SurtrMenuConsole -State $console
    }
    return $result
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

function Read-SurtrConfigDirPath {
    # A typed folder, for a machine detection cannot place. Returns '' when the
    # user presses Enter to go back.
    while ($true) {
        $typed = Read-SurtrText "  Type mpv's configuration folder path (Enter to go back)"
        if (-not $typed) { return '' }
        if (-not (Test-Path -LiteralPath $typed)) {
            Write-SurtrLine "  Not a folder: $typed" 'danger'
            continue
        }
        $full = (Resolve-Path -LiteralPath $typed).Path
        if (Test-SurtrConfigDir $full) { return $full }
        Write-SurtrLine "  $full has no mpv.conf, scripts or script-opts folder in it; that does not look like mpv's configuration folder." 'danger'
    }
}

function Read-SurtrConfigTargets {
    # One or more mpv configuration folders: a ticked list when any were found,
    # a typed path when none were. $null when the user backs out.
    $found = Find-SurtrMpvConfigs
    if ($found.Count -eq 0) {
        Write-SurtrLine '  Surtr found no mpv configuration folder.' 'muted'
        $typed = Read-SurtrConfigDirPath
        if (-not $typed) {
            $script:Cancelled = $true
            return $null
        }
        return @($typed)
    }

    $items = New-Object System.Collections.Generic.List[object]
    foreach ($path in $found.Keys) { $items.Add((New-SurtrItem -Label $found[$path] -Detail @($path) -Path $path)) }
    $otherIndex = $items.Count
    $items.Add((New-SurtrItem -Label ('Type a folder path' + $script:GlyphDots) -Action $true))
    $quitIndex = $items.Count
    $items.Add((New-SurtrItem -Label 'Quit' -Quit $true))

    Write-Host ''
    Write-SurtrLine '  Which mpv configuration folders?' 'plain'
    while ($true) {
        $menu = Read-SurtrMenu -Items $items.ToArray() -Multi
        if ($menu.Cancelled -or $menu.Index -eq $quitIndex) {
            $script:Cancelled = $true
            return $null
        }
        if ($menu.Index -eq $otherIndex) {
            $typed = Read-SurtrConfigDirPath
            if ($typed) { return @($typed) }
            continue
        }
        $picked = @($menu.Ticked | ForEach-Object { $items[$_].Path })
        if ($picked.Count -gt 0) { return $picked }
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
            $answer = Read-SurtrText 'Close mpv so Surtr can replace its files, then press Enter to retry (q to give up)'
            if ($answer -match '^q(uit)?$') { throw "Cancelled while replacing $What; the previous install was left in place." }
        }
    }
}

# --- package source ----------------------------------------------------------

function Get-SurtrMissingFiles {
    # What the package lacks, relative paths; empty when it is the install
    # layout. A release asset that came back as an error page fails here.
    param([string]$Root)
    return @($script:Layout | Where-Object { -not (Test-Path -LiteralPath (Join-Path $Root $_)) })
}

function Expand-SurtrSource {
    # The package tree to install from: surtr.zip beside this script when there
    # is one (offline, no network at all), else the latest release download.
    param([string]$Work)

    $localZip = $null
    if ($PSScriptRoot) { $localZip = Join-Path $PSScriptRoot 'surtr.zip' }
    if ($localZip -and (Test-Path -LiteralPath $localZip)) {
        Write-SurtrLine "  Using $localZip" 'muted'
    }
    else {
        $localZip = Join-Path $Work 'surtr.zip'
        Write-SurtrLine "  Downloading $script:ZipUrl" 'muted'
        Invoke-WebRequest -Uri $script:ZipUrl -OutFile $localZip -UseBasicParsing
    }

    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $root = Join-Path $Work 'package'
    New-Item -ItemType Directory -Force -Path $root | Out-Null
    [IO.Compression.ZipFile]::ExtractToDirectory($localZip, $root)

    $missing = @(Get-SurtrMissingFiles -Root $root)
    if ($missing.Count -gt 0) {
        # No silent fallback to the download: an offline run must stay offline,
        # and a wrong zip is the user's to fix, not to have papered over.
        throw ("$localZip is not a Surtr package; it lacks " + ($missing -join ', '))
    }
    return $root
}

# --- install / uninstall ------------------------------------------------------

function Set-SurtrPlugin {
    # Plugin files replaced wholesale; the conf is synced and the override file
    # is created only when the user has none in either place.
    param([string]$Source, [string]$ConfigDir)

    $from = Join-Path $Source 'scripts\surtr'
    $to = Join-Path $ConfigDir 'scripts\surtr'
    Invoke-SurtrWithMpvRetry -What 'surtr.exe' -ArgumentList @($from, $to) -Action {
        param($src, $dst)
        New-Item -ItemType Directory -Force -Path $dst | Out-Null
        foreach ($file in @('main.lua', 'surtr.exe', 'README.md')) {
            Copy-Item -LiteralPath (Join-Path $src $file) -Destination (Join-Path $dst $file) -Force
        }
    }

    $confDir = Join-Path $ConfigDir 'script-opts'
    New-Item -ItemType Directory -Force -Path $confDir | Out-Null
    $confPath = Join-Path $confDir 'surtr.conf'
    $existing = if (Test-Path -LiteralPath $confPath) { [IO.File]::ReadAllText($confPath) } else { '' }
    $template = [IO.File]::ReadAllText((Join-Path $Source 'script-opts\surtr.conf'))
    [IO.File]::WriteAllText($confPath, (Merge-SurtrConf -Template $template -Existing $existing),
        (New-Object System.Text.UTF8Encoding($false)))

    $oldOverrides = if ($env:LOCALAPPDATA) {
        Join-Path $env:LOCALAPPDATA 'surtr\overrides.json'
    } else { '' }
    [void](New-SurtrOverridesFile -ConfDir $confDir -ExistingConf $existing -LegacyOverridesPath $oldOverrides)
}

function Install-Surtr {
    $targets = Read-SurtrConfigTargets
    if ($null -eq $targets) {
        Write-Host '  Nothing was installed.'
        return
    }

    $work = Join-Path ([IO.Path]::GetTempPath()) ("surtr-install-" + [guid]::NewGuid())
    New-Item -ItemType Directory -Path $work | Out-Null
    try {
        $source = Expand-SurtrSource -Work $work
        Write-Host ''
        foreach ($dir in $targets) {
            Write-SurtrLine "  Installing Surtr into $dir" 'muted'
            Set-SurtrPlugin -Source $source -ConfigDir $dir
        }

        Write-Host ''
        Write-Host '  Done. Start Discord, then mpv.'
        Write-Host '  Point Surtr at your anime folders: open script-opts\surtr.conf in the'
        Write-Host '  configuration folder above and fill in, for example:'
        Write-Host '      anime_roots=D:\Anime;E:\Anime\Seasonal'
        Write-Host '  The first time surtr.exe starts, Windows SmartScreen may ask; choose'
        Write-Host '  More info, then Run anyway.'
    }
    finally {
        Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
    }
}

function Uninstall-Surtr {
    $targets = Read-SurtrConfigTargets
    if ($null -eq $targets) {
        Write-Host '  Nothing was removed.'
        return
    }

    foreach ($dir in $targets) {
        $plugin = Join-Path $dir 'scripts\surtr'
        if (-not (Test-Path -LiteralPath $plugin)) {
            Write-SurtrLine "  Surtr was not installed in $dir" 'muted'
            continue
        }
        Invoke-SurtrWithMpvRetry -What 'surtr.exe' -ArgumentList @($dir) -Action {
            param($base)
            Remove-Item -LiteralPath (Join-Path $base 'scripts\surtr') -Recurse -Force -ErrorAction SilentlyContinue
            Remove-Item -LiteralPath (Join-Path $base 'script-opts\surtr.conf') -Force -ErrorAction SilentlyContinue
        }
        Write-Host "  Removed Surtr from $dir"
    }

    $data = if ($env:LOCALAPPDATA) { Join-Path $env:LOCALAPPDATA 'surtr' } else { $null }
    if ($data -and (Test-Path -LiteralPath $data)) {
        Write-Host ''
        $delete = Read-SurtrConfirm -Question "Also delete $data (cache and ownership)?" -Default $false
        if ($delete) {
            Invoke-SurtrWithMpvRetry -What 'the Surtr data folder' -ArgumentList @($data) -Action {
                param($folder)
                Remove-Item -LiteralPath $folder -Recurse -Force
            }
            Write-Host "  Removed $data"
        }
        else {
            Write-SurtrLine "  Kept $data" 'muted'
        }
    }
    Write-Host '  Surtr is uninstalled. Nothing else was ever installed: no service, no registry entries, no scheduled task.'
}

function Test-SurtrExplorerConsole {
    # Started from Explorer ("Run with PowerShell", or a double-clicked .ps1):
    # that console closes with the script, taking the closing lines with it.
    # A terminal the user opened, or irm | iex, has no explorer parent.
    try {
        $me = Get-CimInstance Win32_Process -Filter "ProcessId=$PID"
        $parent = Get-Process -Id $me.ParentProcessId -ErrorAction SilentlyContinue
        return ($null -ne $parent -and $parent.ProcessName -eq 'explorer')
    }
    catch { return $false }
}

# --- main ---------------------------------------------------------------------

if ($SelfTest) { return }

$doUninstall = [bool]$Uninstall -or ($env:SURTR_UNINSTALL -and $env:SURTR_UNINSTALL -ne '0')

$failed = $false
try {
    Initialize-SurtrUi
    Write-SurtrBanner
    $run = $true
    if (-not $doUninstall) {
        Write-Host ''
        $choice = Read-SurtrMenu -Items @(
            (New-SurtrItem -Label 'Install / Update'),
            (New-SurtrItem -Label 'Uninstall')
        )
        $run = -not $choice.Cancelled
        if ($choice.Cancelled) { $script:Cancelled = $true }
        $doUninstall = ($choice.Index -eq 1)
    }
    if ($run) {
        if ($doUninstall) { Uninstall-Surtr } else { Install-Surtr }
    }
}
catch {
    Write-SurtrLine ('error: ' + $_.Exception.Message) 'danger'
    $failed = $true
}
finally {
    if (-not $script:Cancelled -and (Test-SurtrExplorerConsole)) {
        Write-Host ''
        [void](Read-Host 'Press Enter to close')
    }
}
if ($failed) { exit 1 }
