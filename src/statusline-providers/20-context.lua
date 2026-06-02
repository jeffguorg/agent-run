-- Context window usage display.
local M = {}

-- Pick a foreground RGB for the given usage percentage.
-- Inline threshold table: 40 → yellow, 70 → orange, 90 → red; else green.
local function color_for(pct)
    if pct >= 90 then return { 255,  90,  90 } end
    if pct >= 70 then return { 255, 180,  60 } end
    if pct >= 40 then return { 240, 220,  90 } end
    return { 120, 200, 120 }
end

local function paint(pct)
    local c = color_for(pct)
    return string.format("\27[38;2;%d;%d;%dm%3.0f%%\27[0m", c[1], c[2], c[3], pct)
end

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

-- Optional colored variant. Rust picks this only when the terminal can
-- render color; otherwise it falls back to statusline_part.
function M.statusline_part_colored(_, ctx)
    local cw = (ctx.stdin or {}).context_window
    local pct = cw and cw.used_percentage
    if pct == nil then
        return " | ctx used ...%"
    end
    return " | ctx used " .. paint(pct)
end

return M
