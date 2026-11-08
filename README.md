<div align="center">

# Void

**A quiet companion for your AI coding sessions.**

A small desktop cat that keeps Claude and Codex quotas close at hand.

![Void](assets/cat-awake.png)

Rust · Windows · Local first · MIT

</div>

## What Void does

Click the cat to open the menu. It answers one question: can I use each agent right now, and what is left? At the top, one line built from the same data as the agent below it. Tabs switch between Claude and Codex; each tab carries a dot in that agent's status color. The chosen agent shows its status (Ready to work, Running low, Paused until 19:28, or why there is no data) and its two limits side by side, the five-hour window and the weekly one: how much is left, a bar, when it resets, and how many tokens were spent in that window. Subscriptions report quota as percentages; token counts come from the agents' local transcripts (fresh input, cache writes and output). Claude chat on the web or phone shares the quota but is not in those transcripts. The menu closes after 30 seconds without the pointer on it or the cat, and at once when you click another window. The gear opens settings.

Rest the pointer on the sleeping cat to start a task. Pick the agent and the project, from the app's own project list in its sidebar order and names (Claude Desktop's environments, the Codex app's projects), with the app's current project chosen by default and, for Claude, each project's model shown; write what to do, and press Enter (Shift+Enter for a new line). Void opens the task in the desktop app you work in, through the app's own link: a new Claude Code session in Claude Desktop (`claude://code/new`) or a new thread in the Codex app (`codex://threads/new`), in that folder with your text filled in. The app takes it from there, with its usual review and approvals. Below the project, pick a session: a new one by default, or one of the project's existing sessions, newest first. An existing session opens in its app (`claude://code/continue`, `codex://threads/<id>`); those links take no text, so what you wrote goes to the clipboard to paste with Ctrl+V. Esc or the cross closes the composer and keeps the draft.

Every window is built from the same parts: one glass surface (with a thin line in the agent's color on agent windows), one header, one switch for tabs, language and agent choice, one field shape, and one filled button.

- Separate five-hour and weekly limits for each provider.
- Time and typical prompts remaining, based on observed consumption.
- Countdown for confirmed blocks with a known reset time.
- Background updates, without restarting the app.
- Transparent animated cat, draggable across the desktop.
- Windows system tray, optional startup and notifications; no taskbar button.
- Explicit missing and stale data. Unknown values stay unknown.
- A thought bubble above the cat while Claude Code or Codex is working. Hover it for a quiet card: each running session with its project, elapsed time and current step (a tool, or "thinking"), and both quota windows as numbers. Click the bubble to open the menu, where the details are.
- A mystical bell when an agent finishes a turn, the same for Claude and Codex: inharmonic partials, a slow shimmer and a soft echo. Synthesized locally, no sound files.
- Settings behind the gear in the panel: interface language (Українська, Русский, English) and the finish sound, saved in `~/.void/settings.json`.
- Advice on where the next task should go, from how many typical turns each agent's five-hour quota still holds.

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

The installer speaks English, Українська or Русский, chosen from the Windows display language; any other language gets English.

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
| `VOID_NO_PROMPTS=1` | Remember finished turns without their prompt text |
| `CODEX_HOME` | Existing Codex configuration/session location |
| `CLAUDE_CONFIG_DIR` | Existing Claude Code configuration location |

Void stores bounded numeric Claude history locally. It also remembers finished agent turns in `~/.void/turns.jsonl`: agent, project folder name, start and end time, and the first 1000 characters of your prompt. Prompts are used only on this machine, to find similar past work for forecasts; they are never sent anywhere. Set `VOID_NO_PROMPTS=1` to keep turns without text, or delete the file to forget them. Responses, tool output and other conversation content are not stored. Void does not watch running processes; it reads only the agents' own local transcripts.

Account requests go to the provider through Codex app-server or, for the optional Claude CLI source, the Anthropic quota endpoint. Claude CLI credentials are passed to system curl through stdin, not command-line arguments. Five minutes before the CLI access token expires, Void refreshes it the way Claude Code does: under Claude Code's own refresh lock, writing the new tokens back atomically to `.credentials.json`, so the CLI and Void keep sharing one login. Refresh attempts are limited to one per five minutes.

The installer preserves an existing Claude status line and chains its output. Uninstallation restores it. Manual commands: `void connect-statusline` and `void disconnect-statusline`. Invalid settings JSON is left untouched. History remains after uninstall; close Void before deleting `~/.void` to remove it.

## Project layout

```text
src/         Rust desktop app, provider adapters, forecasting and native tray
assets/      Cat artwork embedded in the executable
installer/   Per-user Windows installer
```

Build output and local data do not belong in the public repository. No paid AI service or local inference model is required by Void itself.

## Contributing

Keep provider data separate, preserve uncertainty, never log secrets, and never send prompt text off the machine. Run formatting, Clippy and tests before submitting a change. For quota fixes, include a sanitized fixture rather than a personal session file.

## License

[MIT](LICENSE). Cat artwork in `assets/` is included with this project under the same license.
