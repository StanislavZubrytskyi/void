//! Kimi Code: quota from the app's own login, tokens from its session logs. Read only: the
//! login is short-lived and the app renews it while it works, so quota updates while Kimi Code
//! is in use and Void never touches the credentials.

use crate::{
    claude,
    usage::{Limit, Limits, Sample},
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

/// The default API root, when the app's config does not name one.
const API: &str = "https://api.kimi.ai/coding/v1";
const TOKEN_DAYS: u64 = 8;

pub fn home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(|home| PathBuf::from(home).join(".kimi-code"))
}

/// The value of a top-level `key = "value"` line in the app's TOML config.
pub fn config_value(config: &str, key: &str) -> Option<String> {
    config.lines().find_map(|line| {
        let (name, value) = line.split_once('=')?;
        (name.trim() == key).then(|| value.trim().trim_matches('"').to_owned())
    })
}

fn access_token(home: &Path, now: u64) -> Option<String> {
    let newest = fs::read_dir(home.join("credentials"))
        .ok()?
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .max_by_key(|entry| entry.metadata().and_then(|m| m.modified()).ok())?;
    let login: Value = serde_json::from_slice(&fs::read(newest.path()).ok()?).ok()?;
    // An expired token would only earn a 401: wait for the app to renew it.
    (login.get("expires_at")?.as_u64()? > now + 30)
        .then(|| login.get("access_token")?.as_str().map(str::to_owned))
        .flatten()
        .filter(|token| claude::safe(token))
}

/// Current quota, when the app holds a live login. None otherwise.
pub fn account_sample() -> Option<Sample> {
    let home = home()?;
    let token = access_token(&home, crate::usage::now())?;
    let root = fs::read_to_string(home.join("config.toml"))
        .ok()
        .and_then(|config| config_value(&config, "base_url"))
        .filter(|url| url.starts_with("https://") && !url.contains(['"', '\\', ' ']))
        .unwrap_or_else(|| API.to_owned());
    let url = format!("{}/usages", root.trim_end_matches('/'));
    let (body, code) = claude::curl(
        &url,
        &format!("header = \"Authorization: Bearer {token}\"\n"),
    )
    .ok()?;
    if code != b"200" {
        return None;
    }
    sample_from_usage(&serde_json::from_slice(&body).ok()?)
}

fn number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
}

fn limit(detail: &Value) -> Option<Limit> {
    let total = number(detail.get("limit")?).filter(|total| *total > 0.0)?;
    let used = detail
        .get("used")
        .and_then(number)
        .or_else(|| Some(total - number(detail.get("remaining")?)?))?;
    Some(Limit {
        used_percentage: (used / total * 100.0).clamp(0.0, 100.0),
        resets_at: crate::codex::epoch(detail.get("resetTime")?.as_str()?)?,
    })
}

/// `usage` is the weekly quota; `limits` holds the rolling windows, five hours among them.
fn sample_from_usage(usage: &Value) -> Option<Sample> {
    let five_hour = usage
        .get("limits")?
        .as_array()?
        .iter()
        .find(|entry| {
            let minutes = entry.pointer("/window/duration").and_then(number);
            let unit = entry.pointer("/window/timeUnit").and_then(Value::as_str);
            match unit {
                Some(unit) if unit.contains("HOUR") => minutes == Some(5.0),
                _ => minutes == Some(300.0),
            }
        })
        .and_then(|entry| limit(entry.get("detail")?));
    let seven_day = usage.get("usage").and_then(limit);
    (five_hour.is_some() || seven_day.is_some()).then(|| Sample {
        time: crate::usage::now(),
        session_id: "kimi-account".into(),
        prompt_id: None,
        rate_limits: Some(Limits {
            five_hour,
            seven_day,
        }),
        cost: None,
        context_window: None,
        model: None,
    })
}

/// Every session log (`sessions/<workspace>/<session>/agents/<agent>/wire.jsonl`) written lately.
fn logs(root: &Path, depth: u8, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(root).into_iter().flatten().flatten() {
        let path = entry.path();
        if depth > 0 {
            logs(&path, depth - 1, out);
        } else if path.file_name().is_some_and(|name| name == "wire.jsonl")
            && entry
                .metadata()
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|at| SystemTime::now().duration_since(at).ok())
                .is_some_and(|age| age < Duration::from_secs(TOKEN_DAYS * 86_400))
        {
            out.push(path);
        }
    }
}

/// Tokens spent per minute over the last eight days.
// shortcut: rereads each recent log every poll; add an offset ledger like Codex's if logs grow large.
pub fn token_minutes(now: u64) -> Vec<(u64, u64)> {
    let mut files = Vec::new();
    if let Some(home) = home() {
        logs(&home.join("sessions"), 4, &mut files);
    }
    let cutoff = now.saturating_sub(TOKEN_DAYS * 86_400);
    let mut minutes = BTreeMap::new();
    for text in files
        .iter()
        .filter_map(|path| fs::read_to_string(path).ok())
    {
        for (at, tokens) in text
            .lines()
            .filter(|line| line.contains("\"usage.record\""))
            .filter_map(usage_record)
            .filter(|(at, _)| *at >= cutoff)
        {
            *minutes.entry(at / 60 * 60).or_insert(0) += tokens;
        }
    }
    minutes.into_iter().collect()
}

fn usage_record(line: &str) -> Option<(u64, u64)> {
    let entry: Value = serde_json::from_str(line).ok()?;
    Some((
        entry.get("time")?.as_u64()? / 1000,
        crate::activity::kimi_tokens(entry.get("usage")?),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_both_windows_and_tolerates_string_numbers() {
        let usage = serde_json::json!({
            "usage": {"limit": "100", "used": "25", "resetTime": "2026-11-10T00:00:00.123456Z"},
            "limits": [{
                "window": {"duration": 300, "timeUnit": "TIME_UNIT_MINUTE"},
                "detail": {"limit": "200", "remaining": "50", "resetTime": "2026-11-08T20:00:00Z"}
            }]
        });
        let limits = sample_from_usage(&usage).unwrap().rate_limits.unwrap();
        assert_eq!(limits.seven_day.unwrap().used_percentage, 25.0);
        assert_eq!(limits.five_hour.unwrap().used_percentage, 75.0);
        // Newer plans have no weekly quota.
        let usage = serde_json::json!({"limits": [{
            "window": {"duration": 5, "timeUnit": "TIME_UNIT_HOUR"},
            "detail": {"limit": 10, "used": 1, "resetTime": "2026-11-08T20:00:00Z"}
        }]});
        let limits = sample_from_usage(&usage).unwrap().rate_limits.unwrap();
        assert!(limits.seven_day.is_none() && limits.five_hour.is_some());
        assert!(sample_from_usage(&serde_json::json!({"limits": []})).is_none());
    }

    #[test]
    fn counts_fresh_tokens_only_and_reads_config_values() {
        let line = r#"{"type":"usage.record","time":120000,"usage":{"inputOther":10,"output":5,"inputCacheRead":900,"inputCacheCreation":2}}"#;
        assert_eq!(usage_record(line), Some((120, 17)));
        let config = "default_model = \"a/b\"\n[models.\"a/b\"]\nbase_url = \"https://x/v1\"\n";
        assert_eq!(
            config_value(config, "default_model").as_deref(),
            Some("a/b")
        );
        assert_eq!(
            config_value(config, "base_url").as_deref(),
            Some("https://x/v1")
        );
        assert_eq!(config_value(config, "nope"), None);
    }

    #[test]
    fn an_expired_login_is_left_alone() {
        let home = std::env::temp_dir().join(format!("void-kimi-{}", std::process::id()));
        fs::create_dir_all(home.join("credentials")).unwrap();
        let login = serde_json::json!({"access_token": "tok", "expires_at": 1000});
        fs::write(home.join("credentials/a.json"), login.to_string()).unwrap();
        assert_eq!(access_token(&home, 900).as_deref(), Some("tok"));
        assert_eq!(access_token(&home, 990), None);
        let _ = fs::remove_dir_all(home);
    }
}
