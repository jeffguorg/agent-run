-- Shared utilities for statusline provider scripts.

local M = {}

local UNIT_SECONDS = {
    minute = 60, hour = 3600, day = 86400, week = 604800, month = 30 * 86400,
}

function M.format_duration(seconds)
    if not seconds or seconds <= 0 then return nil end
    local MONTH = 30 * 86400
    local WEEK  = 7  * 86400
    local DAY   =      86400
    local HOUR  =      3600

    local function fmt(value, no_num, with_num)
        if value >= 0.9 and value <= 1.1 then return no_num end
        local s = string.format("%.1f", value):gsub("%.0$", ""):gsub("%.$", "")
        return s .. with_num
    end

    if seconds >= 0.5 * MONTH then return fmt(seconds / MONTH, "mon", "mon") end
    if seconds >= 0.5 * WEEK  then return fmt(seconds / WEEK,  "week", "w") end
    if seconds >= 0.5 * DAY   then return fmt(seconds / DAY,   "day", "d") end
    return fmt(seconds / HOUR, "hour", "h")
end

function M.format_countdown(seconds_remaining)
    if not seconds_remaining or seconds_remaining <= 0 then return nil end
    local MINUTE, HOUR, DAY = 60, 3600, 86400
    if seconds_remaining >= DAY then
        return string.format("%.1f", seconds_remaining / DAY):gsub("%.0$", ""):gsub("%.$", "") .. "d"
    end
    if seconds_remaining >= HOUR then
        return string.format("%.1f", seconds_remaining / HOUR):gsub("%.0$", ""):gsub("%.$", "") .. "h"
    end
    return string.format("%.0f", seconds_remaining / MINUTE) .. "m"
end

function M.parse_reset_iso(iso_str)
    if not iso_str or type(iso_str) ~= "string" then return nil end
    local y, mo, d, h, mi, s = iso_str:match("^(%d+)%-(%d+)%-(%d+)T(%d+):(%d+):(%d+)")
    if not y then return nil end
    local days_from_epoch = M._days_since_epoch(tonumber(y), tonumber(mo), tonumber(d))
    return days_from_epoch * 86400 + tonumber(h) * 3600 + tonumber(mi) * 60 + tonumber(s)
end

function M._days_since_epoch(y, m, d)
    local a = math.floor((14 - m) / 12)
    local y1 = y + 4800 - a
    local m1 = m + 12 * a - 3
    return d + math.floor((153 * m1 + 2) / 5) + 365 * y1 + math.floor(y1 / 4)
        - math.floor(y1 / 100) + math.floor(y1 / 400) - 2472633
end

function M.build_summary(slots, prefix)
    local parts = {}
    for _, slot in ipairs(slots) do
        if slot.countdown then
            table.insert(parts, string.format("%s(in %s) %s", slot.label, slot.countdown, slot.value))
        else
            table.insert(parts, string.format("%s %s", slot.label, slot.value))
        end
    end
    local body = table.concat(parts, " ")
    if prefix then return prefix .. " " .. body end
    return body
end

function M.kimi_window_seconds(window)
    if not window then return nil end
    local d = tonumber(window.duration or 0)
    if not d or d <= 0 then return nil end
    local unit = window.timeUnit or ""
    if type(unit) == "string" then
        unit = unit:gsub("^TIME_UNIT_", ""):lower()
    end
    local s = UNIT_SECONDS[unit]
    if s then return d * s end
    return nil
end

function M.kimi_pct(detail)
    if not detail then return nil end
    local total = tonumber(detail.limit)
    if not total or total <= 0 then return nil end
    local used = tonumber(detail.used)
    if not used then
        local remaining = tonumber(detail.remaining)
        if not remaining then return nil end
        used = total - remaining
    end
    return string.format("%d%%", math.floor(used / total * 100 + 0.5))
end

function M.default_anthropic_env()
    return { "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY", "ANTHROPIC_BASE_URL" }
end

return M
