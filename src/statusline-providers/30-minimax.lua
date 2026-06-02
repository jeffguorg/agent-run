-- MiniMax quota display.
local util = require("_util")
local M = {}

function M.matcher(ctx)
    local base_url = (ctx.env or {}).ANTHROPIC_BASE_URL or ""
    return regex_match(base_url, "minimax\\.io|minimaxi\\.com")
end

function M.cache_hints()
    local env = util.default_anthropic_env()
    env[#env + 1] = "MINIMAX_GROUP_ID"
    return { key = "minimax", env = env }
end

function M.subscription_quota_usage(_, ctx)
    local token = ctx.env.ANTHROPIC_AUTH_TOKEN or ctx.env.ANTHROPIC_API_KEY or ""
    if token == "" then return nil end

    local group_id = ctx.env.MINIMAX_GROUP_ID or ""
    if group_id == "" then return nil end

    local base_url = ctx.env.ANTHROPIC_BASE_URL or ""
    local endpoint
    if base_url:find("minimaxi") then
        endpoint = "https://api.minimaxi.com/v1/api/openplatform/coding_plan/remains"
    else
        endpoint = "https://api.minimax.io/v1/api/openplatform/coding_plan/remains"
    end

    local resp = http_get(endpoint .. "?GroupId=" .. group_id, {
        Authorization = "Bearer " .. token,
    })
    if resp.status ~= 200 then return nil end

    local data = json_decode(resp.body)
    if not data then return nil end

    local base_resp = data.base_resp or data.baseResp or {}
    local status_code = base_resp.status_code
    if status_code and status_code ~= 0 then return nil end

    local target = nil
    for _, entry in ipairs(data.model_remains or {}) do
        if entry.model_name == "general" then
            target = entry
            break
        end
    end
    if not target then return nil end

    local result = {}
    -- Each row: { total_field, used_field, start_field, end_field, remaining_percent_field }.
    -- Some entries (e.g. `general`) don't populate the count fields, so fall back to
    -- the percent field with total=100.
    local intervals = {
        { "current_interval_total_count", "current_interval_usage_count", "start_time",            "end_time",            "current_interval_remaining_percent" },
        { "current_weekly_total_count",   "current_weekly_usage_count",   "weekly_start_time",     "weekly_end_time",     "current_weekly_remaining_percent" },
    }

    for _, intv in ipairs(intervals) do
        local start_t  = tonumber(target[intv[3]]) or 0
        local end_t    = tonumber(target[intv[4]]) or 0
        if end_t > start_t then
            local total, used = nil, nil
            local total_raw = tonumber(target[intv[1]])
            local used_raw  = tonumber(target[intv[2]])
            if total_raw and total_raw > 0 then
                total, used = total_raw, used_raw or 0
            else
                local remaining = tonumber(target[intv[5]])
                if remaining and remaining >= 0 and remaining <= 100 then
                    total = 100
                    used = math.floor(100 - remaining + 0.5)
                end
            end

            if total and total > 0 then
                local reset_in = nil
                if end_t > 0 then reset_in = end_t / 1000.0 - now end
                result[#result + 1] = {
                    window = math.floor((end_t - start_t) / 1000),
                    reset_in = reset_in,
                    total = total,
                    used = used,
                }
            end
        end
    end
    if #result == 0 then return nil end
    return result
end

function M.statusline_part(_, ctx)
    local windows = M.subscription_quota_usage(_, ctx)
    if not windows then
        local group_id = (ctx.env or {}).MINIMAX_GROUP_ID or ""
        if group_id == "" then return " | minimax: missing MINIMAX_GROUP_ID" end
        return ""
    end

    local slots = {}
    for _, w in ipairs(windows) do
        local slot = util.format_duration(w.window)
        if slot then
            local pct = 0
            if w.total and w.total > 0 then
                pct = math.floor(w.used / w.total * 100 + 0.5)
            end
            if pct > 0 then
                local countdown = nil
                if w.reset_in and w.reset_in > 0 then
                    countdown = util.format_countdown(w.reset_in)
                end
                slots[#slots + 1] = {
                    label = slot,
                    value = string.format("%d%%", pct),
                    countdown = countdown,
                }
            end
        end
    end

    if #slots == 0 then return " | minimax!" end
    return " | " .. util.build_summary(slots, "used")
end

return M
