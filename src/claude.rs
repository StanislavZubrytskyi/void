//! Quota from an existing Claude Code login. This endpoint is undocumented.
use crate::usage::{Limit, Limits, Sample};
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
    Format,
}
impl Failure {
    pub fn label(self) -> &'static str {
        match self {
            Self::Login => "Немає джерела квот",
            Self::Expired => "Вхід Claude завершився",
            Self::Denied => "Доступ до квот відхилено",
            Self::Network => "Немає зв’язку",
            Self::Unavailable => "Квоти недоступні",
            Self::Format => "Невідомий формат",
        }
    }
}
// Claude Code's own production OAuth endpoint and public client id, as its refresh uses them.
const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
/// Refresh this long before expiry, the same margin Claude Code uses.
const REFRESH_MARGIN_MS: u64 = 5 * 60 * 1000;
/// At most one refresh request per this pause: the token endpoint rate-limits (429) eager clients.
const REFRESH_PAUSE_MS: u64 = 5 * 60 * 1000;
static LAST_REFRESH_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn account_sample() -> Result<Sample, Failure> {
    let dir = config_dir().ok_or(Failure::Login)?;
    let credentials = read_credentials(&dir).ok_or(Failure::Login)?;
    let mut token = access_token(&credentials).ok_or(Failure::Login)?;
    if expires_soon(&credentials, now_ms()) {
        token = refresh(&dir, &token)?;
    }
    let body = match fetch(&token) {
        // The token can be revoked before its expiry: refresh once and retry.
        Err(Failure::Expired) => fetch(&refresh(&dir, &token)?)?,
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
fn safe(token: &str) -> bool {
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

/// Refresh the login exactly as Claude Code does: under its lock, writing the new tokens back,
/// so the CLI and Void keep sharing one login. `used` is the token we already have.
fn refresh(dir: &Path, used: &str) -> Result<String, Failure> {
    // Claude Code is refreshing right now: its new token lands on disk, pick it up next poll.
    let _lock = Lock::acquire(dir).ok_or(Failure::Unavailable)?;
    let mut credentials = read_credentials(dir).ok_or(Failure::Login)?;
    let current = access_token(&credentials).ok_or(Failure::Login)?;
    if current != used {
        return Ok(current); // someone refreshed while we waited
    }
    let now = now_ms();
    let last = LAST_REFRESH_MS.load(std::sync::atomic::Ordering::Relaxed);
    if last != 0 && now < last + REFRESH_PAUSE_MS {
        return Err(Failure::Unavailable);
    }
    LAST_REFRESH_MS.store(now, std::sync::atomic::Ordering::Relaxed);
    let refresh_token = credentials
        .pointer("/claudeAiOauth/refreshToken")
        .and_then(Value::as_str)
        .filter(|token| safe(token))
        .ok_or(Failure::Expired)?;
    let mut body = serde_json::json!({
        "grant_type": "refresh_token",
        "refresh_token": refresh_token,
        "client_id": CLIENT_ID,
    });
    if let Some(scopes) = credentials
        .pointer("/claudeAiOauth/scopes")
        .and_then(Value::as_array)
    {
        let scopes: Vec<&str> = scopes.iter().filter_map(Value::as_str).collect();
        body["scope"] = scopes.join(" ").into();
    }
    let data = body.to_string().replace('"', "\\\"");
    let (response, code) = curl(
        TOKEN_URL,
        &format!("header = \"Content-Type: application/json\"\ndata = \"{data}\"\n"),
    )?;
    match code.as_slice() {
        b"200" => {}
        // invalid_grant: the refresh token is dead, only a new /login helps.
        b"400" | b"401" => return Err(Failure::Expired),
        // 429 and server errors: the old token stays, the next attempt waits out REFRESH_PAUSE_MS.
        _ => return Err(Failure::Unavailable),
    }
    let response: Value = serde_json::from_slice(&response).map_err(|_| Failure::Format)?;
    apply_refresh(&mut credentials, &response, now_ms()).ok_or(Failure::Format)?;
    write_credentials(dir, &credentials).map_err(|_| Failure::Unavailable)?;
    access_token(&credentials).ok_or(Failure::Format)
}

/// Merge a token response into the stored login, keeping every other field.
fn apply_refresh(credentials: &mut Value, response: &Value, now: u64) -> Option<()> {
    let oauth = credentials.get_mut("claudeAiOauth")?.as_object_mut()?;
    let access = response.get("access_token")?.as_str()?;
    let expires_in = response.get("expires_in")?.as_u64()?;
    if !safe(access) {
        return None;
    }
    oauth.insert("accessToken".into(), access.into());
    oauth.insert("expiresAt".into(), (now + expires_in * 1000).into());
    if let Some(token) = response.get("refresh_token").and_then(Value::as_str) {
        oauth.insert("refreshToken".into(), token.into());
    }
    if let Some(seconds) = response
        .get("refresh_token_expires_in")
        .and_then(Value::as_u64)
    {
        oauth.insert(
            "refreshTokenExpiresAt".into(),
            (now + seconds * 1000).into(),
        );
    }
    if let Some(scope) = response.get("scope").and_then(Value::as_str) {
        oauth.insert(
            "scopes".into(),
            scope.split_whitespace().collect::<Vec<_>>().into(),
        );
    }
    Some(())
}

/// Replace the file atomically: a reader never sees half a login.
fn write_credentials(dir: &Path, credentials: &Value) -> std::io::Result<()> {
    let path = credentials_path(dir);
    let temporary = path.with_extension("json.void-tmp");
    fs::write(&temporary, serde_json::to_vec(credentials)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
    }
    fs::rename(&temporary, &path).inspect_err(|_| {
        let _ = fs::remove_file(&temporary);
    })
}

/// Claude Code's refresh lock: the directories `<dir>/.oauth_refresh.lock` and `<dir>.lock`
/// (proper-lockfile, stale after 60 s). Held while refreshing, removed on drop.
struct Lock(Vec<PathBuf>);

impl Lock {
    fn acquire(dir: &Path) -> Option<Self> {
        let mut legacy = dir.as_os_str().to_owned();
        legacy.push(".lock");
        let mut lock = Lock(Vec::new());
        for path in [dir.join(".oauth_refresh.lock"), PathBuf::from(legacy)] {
            if fs::create_dir(&path).is_err() {
                // A holder that died leaves the directory behind; Claude Code treats 60 s as dead too.
                let stale = fs::metadata(&path)
                    .and_then(|meta| meta.modified())
                    .ok()
                    .and_then(|at| at.elapsed().ok())
                    .is_some_and(|age| age > std::time::Duration::from_secs(60));
                if !(stale && fs::remove_dir(&path).is_ok() && fs::create_dir(&path).is_ok()) {
                    return None; // dropping `lock` releases what we already hold
                }
            }
            lock.0.push(path);
        }
        Some(lock)
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        for path in self.0.iter().rev() {
            let _ = fs::remove_dir(path);
        }
    }
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
        _ => Err(Failure::Unavailable),
    }
}

/// POST/GET through the system curl. Secrets travel in `config` over stdin, never in the process list.
fn curl(url: &str, config: &str) -> Result<(Vec<u8>, Vec<u8>), Failure> {
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
    fn refresh_response_updates_tokens_and_keeps_the_rest() {
        let mut credentials = serde_json::json!({
            "mcpOAuth": {"x": 1},
            "claudeAiOauth": {"accessToken": "old", "refreshToken": "r1", "expiresAt": 1000,
                "scopes": ["user:inference"], "subscriptionType": "pro"}
        });
        assert!(expires_soon(&credentials, 0));
        let response = serde_json::json!({"access_token": "new", "refresh_token": "r2",
            "expires_in": 28800, "scope": "user:inference user:profile"});
        apply_refresh(&mut credentials, &response, 10_000).unwrap();
        let oauth = &credentials["claudeAiOauth"];
        assert_eq!(oauth["accessToken"], "new");
        assert_eq!(oauth["refreshToken"], "r2");
        assert_eq!(oauth["expiresAt"], 10_000 + 28_800_000);
        assert_eq!(oauth["scopes"][1], "user:profile");
        assert_eq!(oauth["subscriptionType"], "pro");
        assert_eq!(credentials["mcpOAuth"]["x"], 1);
        assert!(!expires_soon(&credentials, 10_000));
        // A token that could break the curl config is refused.
        let bad = serde_json::json!({"access_token": "a\"b", "expires_in": 1});
        assert!(apply_refresh(&mut credentials, &bad, 0).is_none());
    }

    /// Manual: rotates the real login once. `cargo test -- --ignored refreshes_the_real_login`
    #[test]
    #[ignore]
    fn refreshes_the_real_login() {
        let dir = config_dir().unwrap();
        let before = access_token(&read_credentials(&dir).unwrap()).unwrap();
        let after = refresh(&dir, &before).unwrap();
        assert_ne!(before, after);
        assert_eq!(
            access_token(&read_credentials(&dir).unwrap()).unwrap(),
            after
        );
        assert!(fetch(&after).is_ok());
    }

    #[test]
    fn lock_is_exclusive_released_on_drop_and_written_atomically() {
        let root = std::env::temp_dir().join(format!("void-lock-{}", std::process::id()));
        let dir = root.join(".claude");
        fs::create_dir_all(&dir).unwrap();
        let lock = Lock::acquire(&dir).unwrap();
        assert!(dir.join(".oauth_refresh.lock").is_dir());
        assert!(root.join(".claude.lock").is_dir());
        assert!(Lock::acquire(&dir).is_none());
        drop(lock);
        assert!(!dir.join(".oauth_refresh.lock").exists());
        // Claude Code holding only the legacy lock still blocks us, and we release ours.
        fs::create_dir(root.join(".claude.lock")).unwrap();
        assert!(Lock::acquire(&dir).is_none());
        assert!(!dir.join(".oauth_refresh.lock").exists());
        fs::remove_dir(root.join(".claude.lock")).unwrap();

        let credentials = serde_json::json!({"claudeAiOauth": {"accessToken": "t"}});
        write_credentials(&dir, &credentials).unwrap();
        assert_eq!(read_credentials(&dir).unwrap(), credentials);
        let _ = fs::remove_dir_all(root);
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
