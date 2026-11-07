//! Connects Void to the Claude Code status line without losing the user's own one.
//! The installer calls `connect-statusline` and `disconnect-statusline`.

use serde_json::{Value, json};
use std::{
    error::Error,
    fs, io,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    time::Duration,
};

/// A hung user script must not hang Claude Code's status line: past this, Void prints its own.
const PREVIOUS_TIMEOUT: Duration = Duration::from_secs(3);

fn settings_path() -> Option<PathBuf> {
    crate::claude::config_dir().map(|dir| dir.join("settings.json"))
}

fn previous_path() -> io::Result<PathBuf> {
    Ok(crate::usage::data_dir()?.join("previous-statusline.json"))
}

fn is_ours(line: &Value) -> bool {
    line.get("command")
        .and_then(Value::as_str)
        .is_some_and(|command| {
            // Also the Clawdle line this app replaces, so uninstalling never restores it.
            let command = command.to_ascii_lowercase();
            command.contains("void") || command.contains("clawdle")
        })
}

fn read_settings(path: &Path) -> Result<Value, Box<dyn Error>> {
    match fs::read(path) {
        // Invalid JSON is an error: never overwrite settings we could not parse.
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(json!({})),
        Err(error) => Err(error.into()),
    }
}

fn write_settings(path: &Path, settings: &Value) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(path.parent().unwrap())?;
    let temporary = path.with_extension("json.void-tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(settings)?)?;
    fs::rename(temporary, path)?;
    Ok(())
}

pub fn connect() -> Result<(), Box<dyn Error>> {
    let exe = std::env::current_exe()?;
    connect_at(
        &settings_path().ok_or("Claude config directory unavailable")?,
        &previous_path()?,
        &exe,
    )
}

pub fn disconnect() -> Result<(), Box<dyn Error>> {
    disconnect_at(
        &settings_path().ok_or("Claude config directory unavailable")?,
        &previous_path()?,
    )
}

fn connect_at(
    settings_file: &Path,
    previous_file: &Path,
    exe: &Path,
) -> Result<(), Box<dyn Error>> {
    let mut settings = read_settings(settings_file)?;
    let object = settings
        .as_object_mut()
        .ok_or("settings.json is not a JSON object")?;
    if let Some(previous) = object.get("statusLine").filter(|line| !is_ours(line)) {
        fs::create_dir_all(previous_file.parent().unwrap())?;
        fs::write(previous_file, serde_json::to_vec_pretty(previous)?)?;
    }
    let exe = exe.display().to_string().replace('\\', "/");
    object.insert(
        "statusLine".into(),
        json!({ "type": "command", "command": format!("\"{exe}\" statusline") }),
    );
    write_settings(settings_file, &settings)
}

fn disconnect_at(settings_file: &Path, previous_file: &Path) -> Result<(), Box<dyn Error>> {
    let mut settings = read_settings(settings_file)?;
    let Some(object) = settings.as_object_mut() else {
        return Ok(());
    };
    if object.get("statusLine").is_some_and(is_ours) {
        match fs::read(previous_file)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        {
            Some(previous) => object.insert("statusLine".into(), previous),
            None => object.remove("statusLine"),
        };
        write_settings(settings_file, &settings)?;
    }
    let _ = fs::remove_file(previous_file);
    Ok(())
}

/// Run the user's previous status line with the same input and return what it printed.
pub fn previous_output(input: &[u8]) -> Option<Vec<u8>> {
    let previous: Value = serde_json::from_slice(&fs::read(previous_path().ok()?).ok()?).ok()?;
    run(previous.get("command")?.as_str()?, input, PREVIOUS_TIMEOUT)
}

fn run(command: &str, input: &[u8], timeout: Duration) -> Option<Vec<u8>> {
    #[cfg(windows)]
    let mut process = {
        use std::os::windows::process::CommandExt;
        let mut process = Command::new("cmd");
        process
            .arg("/C")
            .raw_arg(command)
            .creation_flags(0x08000000); // CREATE_NO_WINDOW
        process
    };
    #[cfg(not(windows))]
    let mut process = {
        let mut process = Command::new("sh");
        process.args(["-c", command]);
        process
    };
    let mut child = process
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // Write and read on their own threads: a script that ignores stdin or never exits must not block us.
    let (mut stdin, mut stdout) = (child.stdin.take()?, child.stdout.take()?);
    let input = input.to_vec();
    std::thread::spawn(move || {
        let _ = stdin.write_all(&input);
    });
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        let mut output = Vec::new();
        let _ = stdout.read_to_end(&mut output);
        let _ = send.send(output);
    });
    let output = receive.recv_timeout(timeout).ok();
    let _ = child.kill();
    let _ = child.wait();
    output.filter(|output| !output.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previous_status_line_gets_the_input_and_a_hung_one_times_out() {
        #[cfg(windows)]
        let (echo, hang) = ("findstr x", "ping -n 30 127.0.0.1 >nul");
        #[cfg(not(windows))]
        let (echo, hang) = ("cat", "sleep 30");
        let output = run(echo, b"x\n", Duration::from_secs(10)).unwrap();
        assert_eq!(String::from_utf8_lossy(&output).trim(), "x");
        let start = std::time::Instant::now();
        assert!(run(hang, b"x\n", Duration::from_millis(500)).is_none());
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn connect_keeps_the_previous_status_line_and_disconnect_restores_it() {
        let dir = std::env::temp_dir().join(format!("void-statusline-{}", std::process::id()));
        let settings = dir.join("settings.json");
        let previous = dir.join("previous.json");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            &settings,
            r#"{"theme":"dark","statusLine":{"type":"command","command":"mine.sh"},"z":1}"#,
        )
        .unwrap();
        let exe = Path::new(r"C:\Apps\Void\void.exe");

        connect_at(&settings, &previous, exe).unwrap();
        // Reconnecting must not save Void as the user's previous line.
        connect_at(&settings, &previous, exe).unwrap();
        let connected: Value = serde_json::from_slice(&fs::read(&settings).unwrap()).unwrap();
        assert_eq!(
            connected["statusLine"]["command"],
            r#""C:/Apps/Void/void.exe" statusline"#
        );
        assert_eq!(connected["theme"], "dark");
        let keys: Vec<_> = connected.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["theme", "statusLine", "z"]);

        disconnect_at(&settings, &previous).unwrap();
        let restored: Value = serde_json::from_slice(&fs::read(&settings).unwrap()).unwrap();
        assert_eq!(restored["statusLine"]["command"], "mine.sh");
        assert!(!previous.exists());

        fs::write(&settings, "not json").unwrap();
        assert!(connect_at(&settings, &previous, exe).is_err());
        assert_eq!(fs::read_to_string(&settings).unwrap(), "not json");
        let _ = fs::remove_dir_all(dir);
    }
}
