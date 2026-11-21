-- surtr: mpv launcher for the Surtr daemon.
-- Lua only launches (ADR 0001): it sets a per-process IPC pipe, spawns the
-- Rust daemon detached, and forwards config. All logic lives in Rust.

local mp = require 'mp'
local msg = require 'mp.msg'
local utils = require 'mp.utils'
local options = require 'mp.options'

local o = {
    anime_roots = '',
    client_id = '',
    binary_path = '',
    toggle_key = 'ctrl+shift+a',
    details_title_lang = 'english',
    overrides_path = '~~/script-opts/surtr-overrides.json',
    pause_timeout = '0',
}
options.read_options(o, 'surtr')

-- mp.options keeps whitespace around a value (`anime_roots= D:\Anime`);
-- Lua 5.1 (mpv's interpreter) has no string.trim, so spell it out.
local function trim(text)
    return (text:gsub('^%s+', ''):gsub('%s+$', ''))
end

-- mpv is routinely started with cwd `C:\Windows\System32`, so a relative path
-- is refused for both the daemon and the override file.
local function absolute_path(value)
    return value:match('^%a:[/\\]') or value:match('^~~')
        or value:match('^[\\/][\\/]')
end

-- The daemon's only route to the visible console: four IPC script messages,
-- one per level. Lua re-emits each through its matching `mp.msg` function;
-- mpv adds the `[surtr]` script prefix and applies its own message-level
-- filter; verbose lines need --msg-level=all=v.
local log_routes = {
    ['surtr-log'] = msg.info,
    ['surtr-log-warn'] = msg.warn,
    ['surtr-log-error'] = msg.error,
    ['surtr-log-verbose'] = msg.verbose,
}
for name, emit in pairs(log_routes) do
    mp.register_script_message(name, function(text)
        emit(tostring(text))
    end)
end

-- mpv >= 0.36 required: `user-data` is where the daemon later observes the
-- session Toggle (mpv 0.36+ per spec).
if mp.get_property_native('user-data') == nil then
    msg.error('Surtr needs mpv 0.36 or newer (found '
        .. tostring(mp.get_property('mpv-version')) .. ')')
    return
end

if o.anime_roots == '' then
    msg.warn('no anime_roots configured; every local file is eligible '
        .. '(script-opts/surtr.conf)')
end

-- client_id needs no warning: left empty, surtr.exe publishes through the
-- application id built into it. Only failures are worth console noise, and
-- the daemon's own diagnostics cover those.

-- Binary discovery: `binary_path` override expanded through expand-path
-- (supports `~~/`, `~~exe_dir/`), else `surtr.exe` beside this script.
local binary
local why = ''
if o.binary_path ~= '' then
    local override = trim(o.binary_path)
    if not absolute_path(override) then
        msg.error('binary_path "' .. override .. '" is not absolute; it '
            .. 'must start with a drive letter or ~~ (script-opts/surtr.conf). '
            .. 'Surtr disabled.')
        return
    end
    binary = mp.command_native({ 'expand-path', override })
    why = ' (binary_path in script-opts/surtr.conf)'
else
    local dir = mp.get_script_directory()
    binary = dir and (dir .. '/surtr.exe') or nil
end

local info = binary and utils.file_info(binary)
if not info or info.is_dir then
    msg.error('surtr.exe not found at "' .. tostring(binary) .. '"' .. why
        .. '; Surtr disabled. Re-extract the zip, or set binary_path in '
        .. 'script-opts/surtr.conf.')
    return
end

-- Overrides sit beside surtr.conf by default. The path follows binary_path's
-- rule: absolute, `~~/` or `~~exe_dir/`, never relative. A bad value disables
-- Overrides only; the daemon still runs. Empty means the default.
local overrides_arg
local overrides_setting = trim(o.overrides_path)
if overrides_setting == '' then
    overrides_setting = '~~/script-opts/surtr-overrides.json'
end
if not absolute_path(overrides_setting) then
    msg.error('overrides_path "' .. overrides_setting .. '" is not '
        .. 'absolute; it must start with a drive letter or ~~ '
        .. '(script-opts/surtr.conf). Overrides disabled.')
else
    overrides_arg = mp.command_native({ 'expand-path', overrides_setting })
    if not overrides_arg or overrides_arg == '' then
        msg.error('could not expand overrides_path "' .. overrides_setting
            .. '" (script-opts/surtr.conf). Overrides disabled.')
        overrides_arg = nil
    end
end

-- mpv creates its IPC listener asynchronously and with
-- FILE_FLAG_FIRST_PIPE_INSTANCE, so the pipe name is per process and the
-- daemon retries connecting for a few seconds (ADR 0001).
local pipe = string.format('\\\\.\\pipe\\surtr-%d', mp.get_property_number('pid'))
mp.set_property('input-ipc-server', pipe)

local args = { binary, '--mpv-pipe', pipe }
if o.client_id ~= '' then
    table.insert(args, '--client-id')
    table.insert(args, o.client_id)
end
if o.anime_roots ~= '' then
    table.insert(args, '--anime-roots')
    table.insert(args, o.anime_roots)
end
-- Title languages are normalized in Rust: a blank line or a typo there keeps
-- the built-in default, so only a set value is worth forwarding.
if o.details_title_lang ~= '' then
    table.insert(args, '--details-title-lang')
    table.insert(args, o.details_title_lang)
end
if overrides_arg then
    table.insert(args, '--overrides')
    table.insert(args, overrides_arg)
end
if o.pause_timeout ~= '' then
    table.insert(args, '--pause-timeout')
    table.insert(args, o.pause_timeout)
end

-- The session Toggle: a privacy switch for this mpv window. The daemon
-- observes `user-data/surtr/enabled` and clears/republishes Presence on the
-- change, so the binding itself only flips the shared flag. Session-scoped:
-- mpv forgets the flag when it exits.
local enabled = true
local function toggle()
    enabled = not enabled
    mp.set_property_bool('user-data/surtr/enabled', enabled)
    mp.osd_message('Surtr: presence ' .. (enabled and 'on' or 'off'))
end
mp.add_key_binding(o.toggle_key, 'surtr-toggle', toggle)

-- Fire and forget: mpv startup must never block on the daemon.
utils.subprocess_detached({ args = args, playback_only = false })
-- `info`, not `verbose`: mpv hides verbose lines unless the user starts it
-- with --msg-level, so a silent failure would otherwise print nothing at all.
msg.info('watching for playback (IPC ' .. pipe .. ')')
