-- GLM (Zhipu AI) quota display.
local util = require("_util")
local M = {}

function M.matcher(ctx)
    local base_url = (ctx.env or {}).ANTHROPIC_BASE_URL or ""
    return regex_match(base_url, "z\\.ai|bigmodel\\.cn")
end

function M.cache_hints()
    return {
        key = "glm",
        env = util.default_anthropic_env(),
    }
end

function M.statusline_part(_, ctx)
    local token = ctx.env.ANTHROPIC_AUTH_TOKEN or ctx.env.ANTHROPIC_API_KEY or ""
    if token == "" then return "" end

    local base_url = ctx.env.ANTHROPIC_BASE_URL or ""
    local base_domain = base_url:match("^(https?://[^/]+)")
    if not base_domain then return "" end

    local resp = http_get(base_domain .. "/api/monitor/usage/quota/limit", {
        Authorization = token,
        ["Accept-Language"] = "en-US,en",
        ["Content-Type"] = "application/json",
    })
    if resp.status ~= 200 then return "" end

    local data = json_decode(resp.body)
    if not data then return "" end

    local UNIT_SECONDS = { [3] = 3600, [5] = 30 * 86400, [6] = 604800 }
    local limits = (data.data or data).limits or {}
    local slots = {}

    for _, item in ipairs(limits) do
        if item.type == "TOKENS_LIMIT" and item.percentage ~= nil then
            local pct_val = math.floor(tonumber(item.percentage) + 0.5)
            local unit = tonumber(item.unit or 0)
            local number = tonumber(item.number or 0)
            local secs = number * (UNIT_SECONDS[unit] or 0)
            local slot = util.format_duration(secs)
            if slot then
                local countdown = nil
                local next_reset_ms = tonumber(item.nextResetTime)
                if next_reset_ms and next_reset_ms > 0 then
                    countdown = util.format_countdown(next_reset_ms / 1000.0 - now)
                end
                slots[#slots + 1] = { label = slot, value = string.format("%d%%", pct_val), countdown = countdown }
            end
        end
    end

    if #slots == 0 then return " | glm!" end
    return " | " .. util.build_summary(slots)
end

return M
