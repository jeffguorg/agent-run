-- CWD + git branch display — always matches.
local M = {}

function M.matcher()
    return true
end

function M.statusline_part(_, ctx)
    local parts = { ctx.cwd_short or "?" }
    if ctx.git and ctx.git.branch then
        local s = ctx.git.branch
        if ctx.git.dirty then s = s .. "*" end
        parts[#parts + 1] = "(" .. s .. ")"
    end
    return " | " .. table.concat(parts, " ")
end

return M
