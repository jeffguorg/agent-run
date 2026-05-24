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

local UNIT_SECONDS = { [3] = 3600, [5] = 30 * 86400, [6] = 604800 }

function M.subscription_quota_usage(_, ctx)
    local token = ctx.env.ANTHROPIC_AUTH_TOKEN or ctx.env.ANTHROPIC_API_KEY or ""
    if token == "" then return nil end

    local base_url = ctx.env.ANTHROPIC_BASE_URL or ""
    local base_domain = base_url:match("^(https?://[^/]+)")
    if not base_domain then return nil end

    local resp = http_get(base_domain .. "/api/monitor/usage/quota/limit", {
        Authorization = token,
        ["Accept-Language"] = "en-US,en",
        ["Content-Type"] = "application/json",
    })
    if resp.status ~= 200 then return nil end

    local data = json_decode(resp.body)
    if not data then return nil end

    local limits = (data.data or data).limits or {}
    local result = {}

    for _, item in ipairs(limits) do
        if item.type == "TOKENS_LIMIT" and item.percentage ~= nil then
            local pct_val = tonumber(item.percentage) or 0
            local unit = tonumber(item.unit or 0)
            local number = tonumber(item.number or 0)
            local secs = number * (UNIT_SECONDS[unit] or 0)

            local reset_in = nil
            local next_reset_ms = tonumber(item.nextResetTime)
            if next_reset_ms and next_reset_ms > 0 then
                reset_in = next_reset_ms / 1000.0 - now
            end

            result[#result + 1] = {
                window = secs,
                reset_in = reset_in,
                total = 100,
                used = pct_val,
            }
        end
    end

    if #result == 0 then return nil end
    return result
end

function M.statusline_part(_, ctx)
    local windows = M.subscription_quota_usage(_, ctx)
    if not windows then return "" end

    local slots = {}
    for _, w in ipairs(windows) do
        local slot = util.format_duration(w.window)
        if slot then
            local countdown = nil
            if w.reset_in and w.reset_in > 0 then
                countdown = util.format_countdown(w.reset_in)
            end
            slots[#slots + 1] = {
                label = slot,
                value = string.format("%d%%", math.floor(w.used + 0.5)),
                countdown = countdown,
            }
        end
    end

    if #slots == 0 then return " | glm!" end
    return " | " .. util.build_summary(slots, "used")
end

return M
