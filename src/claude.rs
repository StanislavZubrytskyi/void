//! Quota from an existing Claude Code login. This endpoint is undocumented.
use crate::{
    i18n::s,
    usage::{Limit, Limits, Sample},
};
use serde_json::Value;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
#[derive(Debug, Clone, Copy)]
pub enum Failure {
    Login,
    Expired,
    Denied,
    Network,
    Unavailable,
    RateLimited,
    Format,
}
impl Failure {
    pub fn label(self) -> &'static str {
        match self {
            Self::Login => s(
                "Немає джерела квот",
                "Нет источника квот",
                "No quota source",
            ),
            Self::Expired => s(
                "Вхід Claude завершився",
                "Вход Claude истёк",
                "Claude login expired",
            ),
            Self::Denied => s(
                "Доступ до квот відхилено",
                "Доступ к квотам отклонён",
                "Quota access denied",
            ),
            Self::Network => s("Немає зв’язку", "Нет связи", "No connection"),
            Self::Unavailable => s("Квоти недоступні", "Квоты недоступны", "Quota unavailable"),
            Self::RateLimited => s(
                "Claude обмежив запити, спробую пізніше",
                "Claude ограничил запросы, попробую позже",
                "Claude rate-limited us, retrying later",
            ),
            Self::Format => s("Невідомий формат", "Неизвестный формат", "Unknown format"),
        }
    }
}
/// Refresh this long before expiry, the same margin Claude Code uses.
const REFRESH_MARGIN_MS: u64 = 5 * 60 * 1000;
/// A failed renewal is not retried sooner than this: each attempt is a real Claude Code turn.
const RENEW_PAUSE_MS: u64 = 60 * 60 * 1000;
static NEXT_RENEW_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn account_sample() -> Result<Sample, Failure> {
    let dir = config_dir().ok_or(Failure::Login)?;
    let credentials = read_credentials(&dir).ok_or(Failure::Login)?;
    let mut token = access_token(&credentials).ok_or(Failure::Login)?;
    if expires_soon(&credentials, now_ms()) {
        token = renew(&dir, &token)?;
    }
    let body = match fetch(&token) {
        // The token can be revoked before its expiry: renew once and retry.
        Err(Failure::Expired) => fetch(&renew(&dir, &token)?)?,
        result => result?,
    };
    sample_from_usage(&serde_json::from_slice(&body).map_err(|_| Failure::Format)?)
        .ok_or(Failure::Format)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

fn credentials_path(dir: &Path) -> PathBuf {
    dir.join(".credentials.json")
}

// Do not invoke Keychain here: unattended polling must not trigger consent dialogs.
fn read_credentials(dir: &Path) -> Option<Value> {
    serde_json::from_slice(&fs::read(credentials_path(dir)).ok()?).ok()
}

/// Tokens go into curl config lines, so reject anything that could break out of a quoted value.
pub(crate) fn safe(token: &str) -> bool {
    !token.is_empty()
        && token
            .bytes()
            .all(|b| b.is_ascii_graphic() && b != b'"' && b != b'\\')
}

fn access_token(credentials: &Value) -> Option<String> {
    let token = credentials
        .pointer("/claudeAiOauth/accessToken")?
        .as_str()?;
    safe(token).then(|| token.to_owned())
}

fn expires_soon(credentials: &Value, now: u64) -> bool {
    credentials
        .pointer("/claudeAiOauth/expiresAt")
        .and_then(Value::as_u64)
        .is_some_and(|at| now + REFRESH_MARGIN_MS >= at)
}

/// Claude Code renews its own login, and Void only reads the result: the token endpoint
/// answered Void's own requests with 429 while the CLI's went through.
fn renew(dir: &Path, used: &str) -> Result<String, Failure> {
    let current = |dir| read_credentials(dir).and_then(|login| access_token(&login));
    let fresh = current(dir).ok_or(Failure::Login)?;
    if fresh != used {
        return Ok(fresh); // renewed since we read it
    }
    let now = now_ms();
    if now < NEXT_RENEW_MS.load(std::sync::atomic::Ordering::Relaxed) {
        return Err(Failure::Expired);
    }
    NEXT_RENEW_MS.store(now + RENEW_PAUSE_MS, std::sync::atomic::Ordering::Relaxed);
    if !run_claude_code() {
        return Err(Failure::Expired);
    }
    current(dir)
        .filter(|token| token != used)
        .ok_or(Failure::Expired)
}

/// One short headless turn, which makes the CLI renew its login. Saves no session.
fn run_claude_code() -> bool {
    #[cfg(windows)]
    let mut command = {
        use std::os::windows::process::CommandExt;
        let mut command = Command::new("cmd");
        // npm installs `claude` as a .cmd, which only cmd can start.
        command.args(["/C", "claude"]).creation_flags(0x08000000);
        command
    };
    #[cfg(not(windows))]
    let mut command = Command::new("claude");
    command
        .args(["-p", "ok", "--model", "haiku", "--no-session-persistence"])
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let Ok(mut child) = command.spawn() else {
        return false;
    };
    for _ in 0..240 {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(500)),
            Err(_) => return false,
        }
    }
    // shortcut: kills cmd, not its child; a hung CLI lingers until it exits by itself.
    let _ = child.kill();
    false
}

fn fetch(token: &str) -> Result<Vec<u8>, Failure> {
    let (body, code) = curl(
        "https://api.anthropic.com/api/oauth/usage",
        &format!(
            "header = \"Authorization: Bearer {token}\"\nheader = \"anthropic-beta: oauth-2025-04-20\"\n"
        ),
    )?;
    match code.as_slice() {
        b"200" => Ok(body),
        b"401" => Err(Failure::Expired),
        b"403" => Err(Failure::Denied),
        b"429" => Err(Failure::RateLimited),
        _ => Err(Failure::Unavailable),
    }
}

/// POST/GET through the system curl. Secrets travel in `config` over stdin, never in the process list.
pub(crate) fn curl(url: &str, config: &str) -> Result<(Vec<u8>, Vec<u8>), Failure> {
    #[cfg(windows)]
    let curl = PathBuf::from(std::env::var_os("SystemRoot").ok_or(Failure::Unavailable)?)
        .join("System32/curl.exe");
    #[cfg(not(windows))]
    let curl = PathBuf::from("/usr/bin/curl");
    let mut command = Command::new(curl);
    command
        .args([
            "--silent",
            "--write-out",
            "\n%{http_code}",
            "--max-time",
            "15",
            "--config",
            "-",
            url,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command.spawn().map_err(|_| Failure::Unavailable)?;
    let written = child
        .stdin
        .take()
        .map(|mut stdin| stdin.write_all(config.as_bytes()));
    let output = child.wait_with_output().map_err(|_| Failure::Network)?;
    if !matches!(written, Some(Ok(()))) || !output.status.success() {
        return Err(Failure::Network);
    }
    let split = output
        .stdout
        .iter()
        .rposition(|b| *b == b'\n')
        .ok_or(Failure::Format)?;
    Ok((
        output.stdout[..split].to_vec(),
        output.stdout[split + 1..].to_vec(),
    ))
}
fn sample_from_usage(usage: &Value) -> Option<Sample> {
    let window = |key: &str| {
        let window = usage.get(key)?;
        let used_percentage = window.get("utilization")?.as_f64()?;
        if !(0.0..=100.0).contains(&used_percentage) {
            return None;
        }
        Some(Limit {
            used_percentage,
            resets_at: crate::codex::epoch(window.get("resets_at")?.as_str()?)?,
        })
    };
    let limits = Limits {
        five_hour: window("five_hour"),
        seven_day: window("seven_day"),
    };
    (limits.five_hour.is_some() || limits.seven_day.is_some()).then(|| Sample {
        time: 0,
        session_id: "claude-account".into(),
        prompt_id: None,
        rate_limits: Some(limits),
        cost: None,
        context_window: None,
        model: None,
    })
}
pub fn config_dir() -> Option<PathBuf> {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .or_else(|| std::env::var_os("HOME"))
                .map(|home| PathBuf::from(home).join(".claude"))
        })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_windows_independently_and_rejects_invalid_usage() {
        let value = serde_json::json!({"five_hour":{"utilization":23.0,"resets_at":"2026-11-07T16:59:59.943648+00:00"},
            "seven_day":{"utilization":101.0,"resets_at":"2026-11-10T03:59:59Z"}});
        let limits = sample_from_usage(&value).unwrap().rate_limits.unwrap();
        assert_eq!(limits.five_hour.unwrap().used_percentage, 23.0);
        assert!(limits.seven_day.is_none());
        assert!(sample_from_usage(&serde_json::json!({"five_hour":null})).is_none());
    }

    #[test]
    fn a_token_renewed_elsewhere_is_picked_up_without_running_the_cli() {
        let dir = std::env::temp_dir().join(format!("void-renew-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let login = serde_json::json!({"claudeAiOauth": {"accessToken": "new"}});
        fs::write(credentials_path(&dir), login.to_string()).unwrap();
        assert_eq!(renew(&dir, "old").ok().as_deref(), Some("new"));
        let _ = fs::remove_dir_all(dir);
    }
}

/// Read the desktop-owned quota cache without accessing desktop credentials.
pub fn desktop_history() -> Vec<crate::usage::Record> {
    let mut paths = Vec::new();
    #[cfg(windows)]
    {
        if let Some(home) = std::env::var_os("USERPROFILE").map(PathBuf::from) {
            if let Ok(entries) = std::fs::read_dir(home.join("AppData/Local/Claude-Data")) {
                paths.extend(
                    entries
                        .flatten()
                        .map(|e| e.path().join("plan-usage-history.json")),
                );
            }
            paths.push(home.join("AppData/Roaming/Claude/plan-usage-history.json"));
        }
    }
    #[cfg(not(windows))]
    {
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            paths.push(home.join("Library/Application Support/Claude/plan-usage-history.json"));
            paths.push(home.join(".config/Claude/plan-usage-history.json"));
        }
    }
    paths
        .into_iter()
        .filter_map(|p| {
            let file = std::fs::File::open(p).ok()?;
            if file.metadata().ok()?.len() > 8 * 1024 * 1024 {
                return None;
            }
            let value: Value = serde_json::from_reader(file).ok()?;
            Some(parse_desktop(&value))
        })
        .max_by_key(|rows| rows.last().map(|row| row.time).unwrap_or(0))
        .unwrap_or_default()
}
fn parse_desktop(value: &Value) -> Vec<crate::usage::Record> {
    if value.get("version").and_then(Value::as_u64) != Some(2) {
        return Vec::new();
    }
    let Some(samples) = value.get("samples").and_then(Value::as_array) else {
        return Vec::new();
    };
    let org = samples
        .iter()
        .max_by_key(|s| s.get("t").and_then(Value::as_u64).unwrap_or(0))
        .and_then(|s| s.get("org"))
        .and_then(Value::as_str);
    let Some(org) = org else {
        return Vec::new();
    };
    let mut rows: Vec<_> = samples
        .iter()
        .filter(|s| s.get("org").and_then(Value::as_str) == Some(org))
        .filter_map(|s| {
            let time = s.get("t")?.as_u64()? / 1000;
            if time > crate::usage::now() + 60 {
                return None;
            }
            let window = |key| {
                s.get("u")?
                    .get(key)?
                    .as_f64()
                    .filter(|n| (0.0..=100.0).contains(n))
                    .map(|used_percentage| Limit {
                        used_percentage,
                        resets_at: 0,
                    })
            };
            let limits = Limits {
                five_hour: window("fh"),
                seven_day: window("sd"),
            };
            if limits.five_hour.is_none() && limits.seven_day.is_none() {
                return None;
            }
            Some(crate::usage::Record {
                time,
                sample: Sample {
                    time,
                    session_id: "claude-desktop".into(),
                    prompt_id: None,
                    rate_limits: Some(limits),
                    cost: None,
                    context_window: None,
                    model: None,
                },
            })
        })
        .collect();
    rows.sort_by_key(|r| r.time);
    rows
}
#[cfg(test)]
mod desktop_tests {
    use super::*;
    #[test]
    fn cache_preserves_zero_and_unknown_reset_and_separates_orgs() {
        let value = serde_json::json!({"version":2,"samples":[
            {"t":100000,"org":"a","u":{"fh":90,"sd":80}},
            {"t":200000,"org":"b","u":{"fh":0,"sd":100}}]});
        let rows = parse_desktop(&value);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].time, 200);
        let limits = rows[0].sample.rate_limits.as_ref().unwrap();
        assert_eq!(limits.five_hour.unwrap().used_percentage, 0.0);
        assert_eq!(limits.seven_day.unwrap().resets_at, 0);
        assert!(crate::usage::forecast(&rows, |l| l.five_hour, 200).is_none());
    }
}
