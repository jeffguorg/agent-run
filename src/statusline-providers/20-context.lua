-- Context window usage display.
local M = {}

function M.matcher(_)
    return true
end

function M.statusline_part(_, ctx)
    local cw = (ctx.stdin or {}).context_window
    local pct = cw and cw.used_percentage
    if pct == nil then
        return " | ctx used ...%"
    end
    return string.format(" | ctx used %3.0f%%", pct)
end

return M
