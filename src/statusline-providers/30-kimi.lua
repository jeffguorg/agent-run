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

function M.statusline_part(_, ctx)
    local token = ctx.env.ANTHROPIC_AUTH_TOKEN or ctx.env.ANTHROPIC_API_KEY or ""
    if token == "" then return "" end

    local resp = http_get("https://api.kimi.com/coding/v1/usages", {
        Authorization = "Bearer " .. token,
    })
    if resp.status ~= 200 then return "" end

    local data = json_decode(resp.body)
    if not data then return "" end

    local slots = {}
    for _, limit in ipairs(data.limits or {}) do
        local detail = limit.detail or {}
        local pct = util.kimi_pct(detail)
        if pct then
            local secs = util.kimi_window_seconds(limit.window or {})
            local slot = secs and util.format_duration(secs) or nil
            if slot then
                local countdown = nil
                local reset_epoch = util.parse_reset_iso(detail.resetTime)
                if reset_epoch then countdown = util.format_countdown(reset_epoch - now) end
                slots[#slots + 1] = { label = slot, value = pct, countdown = countdown }
            end
        end
    end

    local usage = data.usage or {}
    local usage_pct = util.kimi_pct(usage)
    if usage_pct then
        local has_week = false
        for _, s in ipairs(slots) do
            if s.label == "week" then has_week = true; break end
        end
        if not has_week then
            local usage_countdown = nil
            local reset_epoch = util.parse_reset_iso(usage.resetTime)
            if reset_epoch then usage_countdown = util.format_countdown(reset_epoch - now) end
            slots[#slots + 1] = { label = "week", value = usage_pct, countdown = usage_countdown }
        end
    end

    if #slots == 0 then return " | kimi!" end

    return " | " .. util.build_summary(slots)
end

return M
