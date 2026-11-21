-- Launcher IPC log routing, driven by a fake mp (no mpv needed).
-- Run with: lua tests/launcher.tests.lua
local registered, calls = {}, {}

local mp = {}
function mp.register_script_message(name, fn) registered[name] = fn end
function mp.get_property_native(name) if name == 'user-data' then return {} end end
function mp.get_property(name)
    return name == 'mpv-version' and 'mpv 0.42.0' or 'C:\\Anime\\ep.mkv'
end
function mp.get_property_number() return 4242 end
function mp.get_script_directory() return 'scripts/surtr' end
function mp.set_property() end
function mp.set_property_bool() end
function mp.osd_message() end
function mp.add_key_binding() end
function mp.command_native(cmd) if cmd[1] == 'expand-path' then return cmd[2] end end

local levels = {}
for _, level in ipairs({ 'info', 'warn', 'error', 'verbose' }) do
    levels[level] = function(text) calls[#calls + 1] = { level, text } end
end

package.preload['mp'] = function() return mp end
package.preload['mp.msg'] = function() return levels end
package.preload['mp.utils'] = function()
    return { file_info = function() return {} end, subprocess_detached = function() end }
end
package.preload['mp.options'] = function() return { read_options = function() end } end

dofile(arg and arg[1] or 'scripts/surtr/main.lua')
calls = {} -- the launcher's own startup line is not an IPC Diagnostic

local routes = {
    { 'surtr-log', 'info' },
    { 'surtr-log-warn', 'warn' },
    { 'surtr-log-error', 'error' },
    { 'surtr-log-verbose', 'verbose' },
}
local wanted = {}
for i, route in ipairs(routes) do
    local name, level = route[1], route[2]
    wanted[name] = true
    local fn = assert(registered[name], 'launcher registers ' .. name)
    local text = 'line ' .. i
    fn(text)
    assert(calls[i][1] == level, name .. ' uses mp.msg.' .. level)
    assert(calls[i][2] == text, name .. ' forwards the text unchanged')
end
for name in pairs(registered) do
    assert(wanted[name], 'no unexpected script message: ' .. name)
end
assert(#calls == #routes, 'one console line per script message')
print('launcher IPC log routes: ok')
