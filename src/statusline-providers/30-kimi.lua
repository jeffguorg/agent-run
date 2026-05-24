-- Kimi (Moonshot AI) quota display.
local util = require("_util")
local M = {}

function M.matcher(ctx)
    local base_url = (ctx.env or {}).ANTHROPIC_BASE_URL or ""
    return regex_match(base_url, "kimi\\.com|moonshot\\.cn")
end

function M.cache_hints()
    return {
        key = "kimi",
        env = util.default_anthropic_env(),
    }
end

function M.subscription_quota_usage(_, ctx)
    local token = ctx.env.ANTHROPIC_AUTH_TOKEN or ctx.env.ANTHROPIC_API_KEY or ""
    if token == "" then return nil end

    local resp = http_get("https://api.kimi.com/coding/v1/usages", {
        Authorization = "Bearer " .. token,
    })
    if resp.status ~= 200 then return nil end

    local data = json_decode(resp.body)
    if not data then return nil end

    local result = {}

    for _, limit in ipairs(data.limits or {}) do
        local detail = limit.detail or {}
        local total = tonumber(detail.limit)
        if total and total > 0 then
            local used = tonumber(detail.used)
            if not used then
                local rem = tonumber(detail.remaining)
                used = rem and (total - rem) or 0
            end

            local secs = util.kimi_window_seconds(limit.window or {})
            local reset_epoch = util.parse_reset_iso(detail.resetTime)
            local reset_in = reset_epoch and (reset_epoch - now) or nil

            if secs then
                result[#result + 1] = {
                    window = secs,
                    reset_in = reset_in,
                    total = total,
                    used = used,
                }
            end
        end
    end

    -- usage (weekly aggregate if not already covered by a limit window)
    local usage = data.usage or {}
    local usage_total = tonumber((usage or {}).limit)
    if usage_total and usage_total > 0 then
        local usage_used = tonumber(usage.used)
        if not usage_used then
            local rem = tonumber(usage.remaining)
            usage_used = rem and (usage_total - rem) or 0
        end

        local has_week = false
        for _, w in ipairs(result) do
            local label = util.format_duration(w.window)
            if label == "week" then has_week = true; break end
        end

        if not has_week then
            local reset_epoch = util.parse_reset_iso(usage.resetTime)
            local reset_in = reset_epoch and (reset_epoch - now) or nil
            result[#result + 1] = {
                window = 7 * 86400,
                reset_in = reset_in,
                total = usage_total,
                used = usage_used,
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
            local pct = math.floor(w.used / w.total * 100 + 0.5)
            slots[#slots + 1] = {
                label = slot,
                value = string.format("%d%%", pct),
                countdown = countdown,
            }
        end
    end

    if #slots == 0 then return " | kimi!" end
    return " | " .. util.build_summary(slots, "used")
end

return M
