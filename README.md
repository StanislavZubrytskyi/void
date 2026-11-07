<div align="center">

# Void

**A quiet companion for your AI coding sessions.**

A small desktop cat that keeps Claude and Codex quotas close at hand.

![Void](assets/cat-awake.png)

Rust · Windows · Local first · MIT

</div>

## What Void does

Click the cat to open a compact monochrome glass panel. Switch between **Claude** and **Codex** to see quota remaining, reset times when available, and a personal forecast once enough measurements exist.

- Separate five-hour and weekly limits for each provider.
- Time and typical prompts remaining, based on observed consumption.
- Countdown for confirmed blocks with a known reset time.
- Background updates, without restarting the app.
- Transparent animated cat, draggable across the desktop.
- Windows system tray, optional startup and notifications; no taskbar button.
- Explicit missing and stale data. Unknown values stay unknown.

## Install on Windows

Build or obtain `Void-0.1.0-setup.exe` from a trusted release of this project. The per-user installer needs no administrator privileges. It installs Void, creates a Start menu shortcut, and connects the Claude Code status line. Startup with Windows is optional.

Click the cat to open or close the panel. Drag it to move it. Right-click the tray icon for visibility, startup, notifications and exit controls.

**Status:** early version. Windows is the primary target. The Rust GUI also targets macOS and Linux, but installers, native tray integration and full platform verification are currently Windows-only.

## Where the numbers come from

| Source | Available data | Limitations |
|---|---|---|
| Codex app-server | Current account quota and reset times | Requires an existing authorized Codex installation |
| Codex session files | Historical quota measurements for forecasts | Other clients may consume the same quota |
| Claude Code status line | Quota measurements and reset times when supplied by Claude | Depends on plan, session activity and workspace trust |
| Claude Desktop quota history | Desktop-owned percentage snapshots | Undocumented v2 cache; refresh is controlled by Claude; no reset timestamps |
| Claude transcript block events | Explicit quota rejection and reset time | Only available after a recorded block |
| Claude Code account endpoint | Current quota using an existing local CLI login | Undocumented endpoint; may be unavailable or change |

Void has no separate sign-in. Existing authorization belongs to the native client. A Desktop login does not imply a CLI credential file exists. Void does not read Desktop cookies or trigger Keychain permission dialogs.

Local files are checked every two seconds. Account snapshots are requested every 30 seconds; Claude failures retry after 60 seconds. Codex uses a persistent helper connection with reconnect. Available measurements older than 30 minutes are marked stale. Reaching a reset time alone does not confirm that access has returned.

Token totals are **not** converted into subscription quota percentages in the production panel. Forecasts require sufficient real measurements and known reset times; they are estimates, not guarantees.

## Build

Requires Rust **1.92+**. Windows requires MSVC Build Tools and Windows SDK. Linux requires a graphical session and the native dependencies used by eframe/OpenGL; window positioning depends on the compositor.

```sh
cargo build --release --locked
```

Run `target/release/void.exe` on Windows or `target/release/void` elsewhere.

### Windows installer

Install Inno Setup 6, then run from the project root:

```powershell
cargo build --release --locked
& "C:\Program Files (x86)\Inno Setup 6\ISCC.exe" installer\void.iss
```

Output: `target/installer/Void-0.1.0-setup.exe`. Version is specified in both `Cargo.toml` and `installer/void.iss`.

### Checks

```sh
cargo fmt --check
cargo clippy --locked -- -D warnings
cargo test --locked
```

```sh
void --check
void --watch-check
```

The first command reports source availability. The second observes background updates for 35 seconds. Neither prints credentials or conversation content. Synthetic data is available only in debug builds: `cargo run --locked -- --demo`.

## Configuration and privacy

| Variable | Purpose |
|---|---|
| `VOID_DATA_DIR` | Override the default `~/.void` history directory |
| `VOID_CODEX_BIN` | Absolute path to the Codex binary |
| `VOID_REDUCED_MOTION=1` | Disable continuous cat animation |
| `CODEX_HOME` | Existing Codex configuration/session location |
| `CLAUDE_CONFIG_DIR` | Existing Claude Code configuration location |

Void stores bounded numeric Claude history locally. It parses relevant local session metadata but does not persist conversation text. Account requests go to the provider through Codex app-server or, for the optional Claude CLI source, the Anthropic quota endpoint. Claude CLI credentials are passed to system curl through stdin, not command-line arguments; Void does not store or refresh them.

The installer preserves an existing Claude status line and chains its output. Uninstallation restores it. Manual commands: `void connect-statusline` and `void disconnect-statusline`. Invalid settings JSON is left untouched. History remains after uninstall; close Void before deleting `~/.void` to remove it.

## Project layout

```text
src/         Rust desktop app, provider adapters, forecasting and native tray
assets/      Cat artwork embedded in the executable
installer/   Per-user Windows installer
```

Build output and local data do not belong in the public repository. No paid AI service or local inference model is required by Void itself.

## Contributing

Keep provider data separate, preserve uncertainty, and avoid logging secrets or conversation content. Run formatting, Clippy and tests before submitting a change. For quota fixes, include a sanitized fixture rather than a personal session file.

## License

[MIT](LICENSE). Cat artwork in `assets/` is included with this project under the same license.
