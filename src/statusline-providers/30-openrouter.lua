-- OpenRouter credits display.
local M = {}

function M.cache_hints()
    return {
        key = "openrouter",
        env = { "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY", "ANTHROPIC_BASE_URL" },
    }
end

function M.matcher(ctx)
    local base_url = (ctx.env or {}).ANTHROPIC_BASE_URL or ""
    return regex_match(base_url, "openrouter\\.ai")
end

function M.statusline_part(_, ctx)
    local token = ctx.env.ANTHROPIC_AUTH_TOKEN or ctx.env.ANTHROPIC_API_KEY or ""
    if token == "" then return "" end

    local resp = http_get("https://openrouter.ai/api/v1/credits", {
        Authorization = "Bearer " .. token,
    })
    if resp.status ~= 200 then return "" end

    local data = json_decode(resp.body)
    if not data then return "" end

    local credits = data.data or {}
    local total = tonumber(credits.total_credits or 0)
    local usage = tonumber(credits.total_usage or 0)
    local remaining = total - usage

    local display = string.format("%.2f", remaining):gsub("%.00$", ""):gsub("%.$", "")
    return " | $" .. display
end

return M
