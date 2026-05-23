-- Context window usage display.
local M = {}

function M.matcher(ctx)
    local cw = (ctx.stdin or {}).context_window
    return cw ~= nil and cw.used_percentage ~= nil
end

function M.statusline_part(_, ctx)
    local pct = ctx.stdin.context_window.used_percentage
    return string.format(" | ctx %s%%", pct)
end

return M
