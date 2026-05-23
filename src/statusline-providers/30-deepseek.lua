-- DeepSeek balance display.
local M = {}

function M.cache_hints()
    return {
        key = "deepseek",
        env = { "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY", "ANTHROPIC_BASE_URL" },
    }
end

function M.matcher(ctx)
    local base_url = (ctx.env or {}).ANTHROPIC_BASE_URL or ""
    return regex_match(base_url, "deepseek\\.com")
end

function M.statusline_part(_, ctx)
    local token = ctx.env.ANTHROPIC_AUTH_TOKEN or ctx.env.ANTHROPIC_API_KEY or ""
    if token == "" then return "" end

    local resp = http_get("https://api.deepseek.com/user/balance", {
        Authorization = "Bearer " .. token,
    })
    if resp.status ~= 200 then return "" end

    local data = json_decode(resp.body)
    if not data then return "" end

    local info = (data.balance_infos or {})[1]
    if not info then return " | deepseek!" end

    local currency = info.currency or ""
    local sign = ({ CNY = "\u{FFE5}", USD = "$" })[currency] or ""
    local amount = tonumber(info.total_balance or "0")

    if sign == "" or not amount then return " | deepseek!" end

    local display = string.format("%.2f", amount):gsub("%.00$", ""):gsub("%.$", "")
    return " | " .. sign .. display
end

return M
