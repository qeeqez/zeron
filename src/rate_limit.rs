//! Rate-limit / quota signals from agent backends.
//!
//! Three sources fold into one `RateLimit` snapshot on `ChatUsage`:
//! - codex app-server `account/rateLimits/updated` notifications (and the
//!   `rate_limits` field on `codex exec` `token_count` events) carry quota
//!   windows — `usedPercent`, `windowDurationMins`, `resetsAt`;
//! - claude's `rate_limit_event` stream frame carries `status`/`resetsAt`;
//! - any backend's `AgentEvent::Error` text mentioning 429/rate-limit/quota
//!   marks the chat limited, with the reset time scraped from the message
//!   when it phrases one ("try again at 3:04 PM", "resets in 35 seconds").

/// One quota window from a backend snapshot — codex reports a primary and
/// an optional secondary (e.g. 5-hour and weekly) window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RateWindow {
    /// Percent of the window consumed, 0–100.
    pub used_percent: f64,
    /// Rolling window length in minutes, when reported.
    pub window_mins: Option<u64>,
    /// Unix seconds when the window resets, when reported.
    pub resets_at: Option<u64>,
}

/// The account's rate-limit state as last reported by the backend.
/// `limited` means a turn was throttled; the windows carry quota detail
/// for the usage popover even when nothing is currently blocked.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RateLimit {
    /// The backend refused or throttled a turn — the banner shows until
    /// the next turn succeeds or a fresh snapshot clears the flag.
    pub limited: bool,
    /// Reset time as the backend phrased it in an error message
    /// ("3:04 PM", "in 35 seconds") — preferred over `resets_at` since it
    /// keeps the server's wording.
    pub reset_hint: Option<String>,
    pub primary: Option<RateWindow>,
    pub secondary: Option<RateWindow>,
}

/// A window at or above this fill warns "approaching limit".
const APPROACHING: f64 = 90.;

impl RateLimit {
    /// Codex's `RateLimitSnapshot` — app-server sends it camelCase under
    /// `params.rateLimits`, `codex exec` carries it snake_case as
    /// `rate_limits` on `token_count`. Both spellings parse.
    pub fn from_codex(v: &serde_json::Value) -> Option<Self> {
        let snap = v.get("rateLimits").unwrap_or(v);
        if !snap.is_object() {
            return None;
        }
        let limited = snap["rateLimitReachedType"].is_string()
            || snap["rate_limit_reached_type"].is_string()
            || snap["spendControlReached"]
                .as_bool()
                .or_else(|| snap["spend_control_reached"].as_bool())
                .unwrap_or(false);
        Some(Self {
            limited,
            reset_hint: None,
            primary: window(&snap["primary"]),
            secondary: window(&snap["secondary"]),
        })
    }

    /// Claude's `rate_limit_event` frame (`rate_limit_info`): `status` is
    /// "allowed"/"rejected", `resetsAt` is epoch seconds, `rateLimitType`
    /// names the window ("five_hour", "seven_day").
    pub fn from_claude(info: &serde_json::Value) -> Option<Self> {
        if !info.is_object() {
            return None;
        }
        let status = info["status"].as_str();
        let limited = status == Some("rejected") || info["overageStatus"].as_str() == Some("rejected");
        // A window with no fields at all is noise — keep it out of the
        // popover's quota rows.
        let primary = window(info).filter(|w| w.used_percent > 0. || w.window_mins.is_some() || w.resets_at.is_some());
        // Status-only frames still emit — an "allowed" clears a prior limit.
        (status.is_some() || limited || primary.is_some()).then_some(Self { limited, reset_hint: None, primary, secondary: None })
    }

    /// An `AgentEvent::Error` mentioning throttling. The message doubles as
    /// the reset-time source when it phrases one.
    pub fn from_error(msg: &str) -> Option<Self> {
        let lower = msg.to_lowercase();
        let hit = ["rate limit", "rate_limit", "ratelimit", "usage limit", "spend cap", "too many requests"]
            .iter()
            .any(|k| lower.contains(k))
            // "quota exceeded"/"insufficient_quota" — but not disk quota.
            || (lower.contains("quota") && !lower.contains("disk"))
            || msg.split(|c: char| !c.is_ascii_alphanumeric()).any(|t| t == "429");
        hit.then(|| Self {
            limited: true,
            reset_hint: reset_hint(msg),
            ..Self::default()
        })
    }

    /// The banner's text: the limit state, or a near-full quota warning.
    /// `None` hides the row.
    pub fn banner(&self, now: std::time::SystemTime) -> Option<String> {
        if self.limited {
            let reset = self.reset_hint.clone().or_else(|| self.reset_time().map(|t| fmt_reset(t, now)));
            return Some(match reset {
                Some(r) => format!("Rate limited — resets {r}"),
                None => "Rate limited — try again later".into(),
            });
        }
        self.windows()
            .map(|w| w.used_percent)
            .reduce(f64::max)
            .filter(|&p| p >= APPROACHING)
            .map(|p| format!("Approaching rate limit — {:.0}% used", p))
    }

    /// Present windows, primary first — the popover's quota rows.
    pub fn windows(&self) -> impl Iterator<Item = &RateWindow> {
        [&self.primary, &self.secondary].into_iter().flatten()
    }

    /// Soonest window reset — when limited, that's when a retry can pass.
    pub fn reset_time(&self) -> Option<u64> {
        self.windows().filter_map(|w| w.resets_at).min()
    }
}

impl RateWindow {
    /// Popover row label: "5h limit", "1w limit", "Limit" when unknown.
    pub fn label(&self) -> String {
        match self.window_mins {
            Some(m) if m % 10080 == 0 => format!("{}w limit", m / 10080),
            Some(m) if m % 1440 == 0 => format!("{}d limit", m / 1440),
            Some(m) if m % 60 == 0 => format!("{}h limit", m / 60),
            Some(m) => format!("{m}m limit"),
            None => "Limit".into(),
        }
    }

    /// Popover row value: "92% · resets 3:04 PM", or just the percent.
    pub fn value(&self, now: std::time::SystemTime) -> String {
        match self.resets_at {
            Some(t) => format!("{:.0}% · resets {}", self.used_percent, fmt_reset(t, now)),
            None => format!("{:.0}%", self.used_percent),
        }
    }
}

/// One window object, camelCase or snake_case. `None` when absent/null.
fn window(v: &serde_json::Value) -> Option<RateWindow> {
    if !v.is_object() {
        return None;
    }
    Some(RateWindow {
        used_percent: v["usedPercent"].as_f64().or_else(|| v["used_percent"].as_f64()).unwrap_or(0.),
        window_mins: v["windowDurationMins"]
            .as_u64()
            .or_else(|| v["window_minutes"].as_u64())
            .or_else(|| claude_window_mins(v)),
        resets_at: v["resetsAt"].as_u64().or_else(|| v["resets_at"].as_u64()),
    })
}

/// Claude's `rateLimitType` names the window instead of giving minutes.
fn claude_window_mins(v: &serde_json::Value) -> Option<u64> {
    match v["rateLimitType"].as_str()? {
        "five_hour" => Some(300),
        "seven_day" => Some(10080),
        _ => None,
    }
}

/// The reset phrase inside an error message, keeping the backend's own
/// wording: "try again at 3:04 PM" → "3:04 PM", "try again in 35 seconds"
/// → "in 35 seconds". Stops at the sentence's end or a parenthetical.
fn reset_hint(msg: &str) -> Option<String> {
    let lower = msg.to_lowercase();
    for (pat, prefix) in [
        ("try again at ", ""),
        ("try again in ", "in "),
        ("resets at ", ""),
        ("resets in ", "in "),
        ("reset at ", ""),
        ("retry after ", "in "),
        ("resets ", ""),
    ] {
        if let Some(ix) = lower.find(pat) {
            let rest = &msg[ix + pat.len()..];
            let end = rest.find(['.', '\n', '(']).unwrap_or(rest.len());
            let hint = rest[..end].trim().trim_end_matches([',', ')']);
            if !hint.is_empty() {
                return Some(format!("{prefix}{hint}"));
            }
        }
    }
    None
}

/// Epoch seconds → "3:04 PM" same-day, "Sep 17, 3:04 PM" otherwise;
/// "soon" when the reset already passed (a stale snapshot).
fn fmt_reset(epoch: u64, now: std::time::SystemTime) -> String {
    let Some(reset) = chrono::DateTime::from_timestamp(epoch as i64, 0) else { return "soon".into() };
    if std::time::UNIX_EPOCH + std::time::Duration::from_secs(epoch) <= now {
        return "soon".into();
    }
    let local = reset.with_timezone(&chrono::Local);
    let today = chrono::DateTime::<chrono::Local>::from(now).date_naive();
    if local.date_naive() == today {
        local.format("%-I:%M %p").to_string()
    } else {
        local.format("%b %-d, %-I:%M %p").to_string()
    }
}
