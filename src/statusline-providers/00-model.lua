-- Model name display — always matches, display_index=0.
local M = {}

function M.matcher()
    return true
end

function M.statusline_part(_, ctx)
    local model = (ctx.stdin or {}).model or {}
    return model.display_name or model.id or "claude"
end

return M
