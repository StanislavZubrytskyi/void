#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod activity;
mod advice;
mod claude;
mod codex;
mod estimate;
mod panel;
mod pet;
mod runtime;
mod statusline;
#[cfg(windows)]
mod tray;
mod usage;

use advice::{Status, Tone};
use eframe::egui::{self, Pos2, Vec2, ViewportCommand};
use std::{collections::HashSet, time::Duration};
use usage::Record;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let command = std::env::args().nth(1);
    // A GUI-subsystem exe has no console: borrow the terminal's so CLI output and errors show up.
    // The status line keeps its pipes from Claude Code, so it never attaches.
    #[cfg(windows)]
    if command
        .as_deref()
        .is_some_and(|command| command != "statusline")
    {
        unsafe {
            windows_sys::Win32::System::Console::AttachConsole(
                windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS,
            );
        }
    }
    match command.as_deref() {
        Some("statusline") => usage::capture(),
        Some("connect-statusline") => statusline::connect(),
        Some("disconnect-statusline") => statusline::disconnect(),
        Some("--version") => {
            println!("Void {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("--check") => {
            let claude = usage::history().map(|history| history.len());
            println!(
                "Claude status line samples: {}",
                claude.map_or_else(|_| "unreadable".into(), |count| count.to_string())
            );
            println!(
                "Claude account limits: {}",
                claude::account_sample()
                    .map(|_| "available")
                    .unwrap_or_else(|error| error.label())
            );
            let estimate = estimate::Estimator::default().confirmed_blocks(usage::now());
            println!(
                "Claude confirmed blocks: {}",
                if estimate.is_empty() {
                    "none active".to_owned()
                } else {
                    format!("{} measurements", estimate.len())
                }
            );
            let desktop = claude::desktop_history();
            println!("Claude Desktop quota samples: {}", desktop.len());
            if let Some(row) = desktop.last() {
                println!(
                    "Claude Desktop snapshot age: {} seconds",
                    usage::now().saturating_sub(row.time)
                );
            }
            let [busy_claude, busy_codex] = activity::scan();
            println!(
                "Working now: Claude {}, Codex {}",
                busy_claude.len(),
                busy_codex.len()
            );
            println!("Codex session samples: {}", codex::history().len());
            println!(
                "Codex account limits: {}",
                if codex::account_snapshot().is_some() {
                    "available"
                } else {
                    "unavailable"
                }
            );
            Ok(())
        }
        Some("--watch-check") => {
            let feed = runtime::start(egui::Context::default());
            let deadline = std::time::Instant::now() + Duration::from_secs(35);
            let mut counts = [0; 3];
            while std::time::Instant::now() < deadline {
                if let Ok(update) = feed.recv_timeout(Duration::from_secs(1)) {
                    match update {
                        runtime::Update::Claude(_, _) => counts[0] += 1,
                        runtime::Update::ClaudeAccount(result) => {
                            counts[1] += 1;
                            println!(
                                "Claude account: {}",
                                result
                                    .map(|_| "available")
                                    .unwrap_or_else(|error| error.label())
                            );
                        }
                        runtime::Update::Codex(_, online) => {
                            counts[2] += 1;
                            if counts[2] == 1 {
                                println!("Codex connected: {online}");
                            }
                        }
                        runtime::Update::Activity([claude, codex]) => {
                            println!(
                                "Working now: Claude {}, Codex {}",
                                claude.len(),
                                codex.len()
                            );
                        }
                    }
                }
            }
            println!(
                "Runtime updates: Claude local={}, account={}, Codex={}",
                counts[0], counts[1], counts[2]
            );
            Ok(())
        }
        Some("--demo") if cfg!(debug_assertions) => desktop(true),
        None => desktop(false),
        Some(_) => Err("unknown command".into()),
    }
}

// Held until the window closes. If the lock cannot be set up, run without it.
fn single_instance() -> Result<Option<std::fs::File>, ()> {
    let Ok(dir) = usage::data_dir() else {
        return Ok(None);
    };
    let file = std::fs::create_dir_all(&dir).and_then(|_| {
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(dir.join("void.lock"))
    });
    match file.map(|file| (file.try_lock(), file)) {
        Ok((Ok(()), file)) => Ok(Some(file)),
        Ok((Err(std::fs::TryLockError::WouldBlock), _)) => Err(()),
        _ => Ok(None),
    }
}

fn desktop(demo: bool) -> Result<(), Box<dyn std::error::Error>> {
    let _instance = if demo {
        None
    } else {
        match single_instance() {
            Ok(lock) => lock,
            Err(()) => return Ok(()),
        }
    };
    let viewport = egui::ViewportBuilder::default()
        .with_title("Void")
        .with_inner_size([96.0, 88.0])
        .with_resizable(false)
        .with_decorations(false)
        .with_transparent(true)
        .with_active(false)
        .with_always_on_top();
    eframe::run_native(
        "Void",
        eframe::NativeOptions {
            viewport,
            ..Default::default()
        },
        Box::new(move |cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            panel::install_fonts(&cc.egui_ctx);
            Ok(Box::new(Companion {
                demo,
                pet: pet::Pet::load(&cc.egui_ctx),
                #[cfg(windows)]
                tray: tray::Tray::install(cc),
                records: Vec::new(),
                measured: Vec::new(),
                codex_records: Vec::new(),
                feed: (!demo).then(|| runtime::start(cc.egui_ctx.clone())),
                claude_accounts: Vec::new(),
                claude_failure: None,
                codex_online: false,
                claude_estimate: Vec::new(),
                updated: None,
                details: false,
                selected_provider: 0,
                open_size: Vec2::new(96.0, 88.0),
                positioned: false,
                offset: advice::utc_offset(),
                notified: HashSet::new(),
                blocked_before: [None; 2],
                busy: Busy {
                    sessions: if demo {
                        demo_busy(usage::now())
                    } else {
                        Default::default()
                    },
                    hover: None,
                    shown: None,
                    away: None,
                    reduced: std::env::var_os("VOID_REDUCED_MOTION").is_some(),
                },
            }))
        }),
    )?;
    Ok(())
}

struct Companion {
    #[cfg(windows)]
    tray: Option<tray::Tray>,
    demo: bool,
    pet: pet::Pet,
    records: Vec<Record>,
    measured: Vec<Record>,
    codex_records: Vec<Record>,
    feed: Option<std::sync::mpsc::Receiver<runtime::Update>>,
    claude_accounts: Vec<Record>,
    claude_failure: Option<claude::Failure>,
    codex_online: bool,
    claude_estimate: Vec<Record>,
    updated: Option<std::time::Instant>,
    details: bool,
    selected_provider: usize,
    open_size: Vec2,
    positioned: bool,
    offset: i64,
    notified: HashSet<(String, u64)>,
    blocked_before: [Option<u64>; 2],
    busy: Busy,
}

/// Working sessions above the closed cat: Claude, then Codex.
#[derive(Default)]
struct Busy {
    sessions: [Vec<activity::Session>; 2],
    /// The agent whose bubble segment (or its card) is under the pointer.
    hover: Option<usize>,
    /// When the bubble appeared, for its pop-in.
    shown: Option<f64>,
    /// When the pointer was last reported gone while a card was open.
    away: Option<f64>,
    reduced: bool,
}

impl Busy {
    /// Chips above the closed cat for every agent that is working, and a card for the hovered one.
    /// Returns the window size this needs and the agent whose chip was clicked.
    #[allow(clippy::too_many_arguments)]
    fn strip(
        &mut self,
        ui: &egui::Ui,
        ctx: &egui::Context,
        bounds: egui::Rect,
        cat: egui::Rect,
        statuses: [&Status; 2],
        now: u64,
        offset: i64,
    ) -> (Vec2, Option<usize>) {
        let agents: Vec<usize> = (0..2).filter(|i| !self.sessions[*i].is_empty()).collect();
        if agents.is_empty() {
            self.hover = None;
            self.shown = None;
            return (Vec2::new(96.0, 88.0), None);
        }
        let clock = ui.input(|input| input.time);
        let shown = *self.shown.get_or_insert(clock);
        let segments = panel::bubble_segments(cat, agents.len());
        let (mut hover, mut clicked) = (None, None);
        for (rect, &agent) in segments.iter().zip(&agents) {
            let segment = ui
                .interact(*rect, ui.id().with(("busy", agent)), egui::Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if segment.hovered() {
                hover = Some(agent);
            }
            if segment.clicked() {
                clicked = Some(agent);
            }
        }
        // The card stays while the pointer is anywhere in the window except the cat. Growing the
        // window under a still pointer briefly reports it as gone, so absence closes the card
        // only after a short grace; without it the window would flicker between sizes.
        let pointer = ctx.pointer_hover_pos();
        if hover.is_none()
            && let Some(agent) = self.hover.filter(|agent| agents.contains(agent))
        {
            match pointer {
                Some(at) if cat.contains(at) => {}
                Some(_) => hover = Some(agent),
                None => {
                    let away = *self.away.get_or_insert(clock);
                    if clock - away < 0.4 {
                        hover = Some(agent);
                        ctx.request_repaint_after(Duration::from_millis(100));
                    }
                }
            }
        }
        if pointer.is_some() || hover.is_none() {
            self.away = None;
        }
        self.hover = hover;
        let time = (!self.reduced).then_some(clock);
        let puffs: Vec<panel::Puff> = agents
            .iter()
            .map(|&agent| panel::Puff {
                agent,
                sessions: self.sessions[agent].len(),
                hover: ctx.animate_bool_with_time(
                    ui.id().with(("busy-hover", agent)),
                    hover == Some(agent),
                    0.18,
                ),
            })
            .collect();
        let appear = if self.reduced {
            1.0
        } else {
            ((clock - shown) / 0.45).min(1.0) as f32
        };
        panel::thought_bubble(ui.painter(), cat, &segments, &puffs, appear, time);
        if time.is_some() {
            ctx.request_repaint_after(Duration::from_millis(33));
        }
        // Two agents need a little room left of the cat for the wider bubble.
        let closed = Vec2::new(
            if agents.len() > 1 { 120.0 } else { 96.0 },
            88.0 + panel::STRIP,
        );
        let Some(agent) = hover else {
            return (closed, clicked);
        };
        let size = panel::busy_card_size(self.sessions[agent].len());
        let window = Vec2::new(
            (size.x + panel::MARGIN * 2.0).max(closed.x),
            88.0 + panel::STRIP + 6.0 + size.y + panel::MARGIN,
        );
        // Draw only once the window has grown, never clipped mid-resize.
        if bounds.width() >= window.x - 0.5 && bounds.height() >= window.y - 0.5 {
            let card = egui::Rect::from_min_size(
                Pos2::new(
                    bounds.right() - panel::MARGIN - size.x,
                    cat.top() - panel::STRIP - 6.0 - size.y,
                ),
                size,
            );
            panel::busy_card(
                ui.painter(),
                card,
                agent,
                &self.sessions[agent],
                &quota_line(statuses[agent], now, offset),
                now,
                time,
            );
        }
        (window, clicked)
    }
}

impl Companion {
    fn has_tray(&self) -> bool {
        #[cfg(windows)]
        return self.tray.is_some();
        #[cfg(not(windows))]
        false
    }
}

impl eframe::App for Companion {
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        if !self.positioned {
            self.positioned = true;
            #[cfg(windows)]
            if let Some(tray) = &self.tray {
                tray.dock();
            }
            if !self.has_tray()
                && let Some(size) = ctx.input(|i| i.viewport().monitor_size)
            {
                ctx.send_viewport_cmd(ViewportCommand::OuterPosition(Pos2::new(
                    (size.x - 108.0).max(0.0),
                    (size.y - 144.0).max(0.0),
                )));
            }
        }
        let time = usage::now();
        let mut changed = self.demo;
        if self
            .updated
            .is_none_or(|last| last.elapsed() >= Duration::from_secs(1))
        {
            self.updated = Some(std::time::Instant::now());
            self.offset = advice::utc_offset();
            if self.demo {
                self.codex_records = demo_codex_history(time);
                self.records = vec![demo_claude_snapshot(time)];
            }
        }
        if let Some(feed) = &self.feed {
            while let Ok(update) = feed.try_recv() {
                changed = true;
                match update {
                    runtime::Update::Claude(rows, estimate) => {
                        self.records = rows;
                        self.claude_estimate = estimate;
                    }
                    runtime::Update::ClaudeAccount(result) => match result {
                        Ok(row) => {
                            self.claude_accounts.push(row);
                            self.claude_failure = None;
                        }
                        Err(error) => self.claude_failure = Some(error),
                    },
                    runtime::Update::Codex(rows, online) => {
                        self.codex_records = rows;
                        self.codex_online = online;
                    }
                    runtime::Update::Activity(sessions) => self.busy.sessions = sessions,
                }
            }
        }
        if changed {
            self.claude_accounts
                .retain(|row| time.saturating_sub(row.time) <= 1800);
            self.measured = self.records.clone();
            self.measured.extend(self.claude_estimate.iter().cloned());
            self.measured.extend(self.claude_accounts.iter().cloned());
            self.measured.sort_by_key(|row| row.time);
            self.measured.dedup_by(|a, b| {
                a.time == b.time
                    && a.sample.session_id == b.sample.session_id
                    && a.sample.prompt_id == b.sample.prompt_id
            });
        }
        let claude = self.measured.as_slice();
        let claude_status = advice::assess(claude, time);
        let codex_status = advice::assess(&self.codex_records, time);
        if !self.demo {
            let messages = alerts(
                &mut self.notified,
                &mut self.blocked_before,
                [
                    ("Claude", &claude_status, claude),
                    ("Codex", &codex_status, &self.codex_records),
                ],
                time,
                self.offset,
            );
            #[cfg(windows)]
            if let Some(tray) = &self.tray {
                for (title, body) in &messages {
                    tray.notify(title, body);
                }
            }
            #[cfg(not(windows))]
            let _ = messages; // shortcut: notifications are Windows-only until the macOS and Linux builds ship.
        }
        let asleep = asleep([&claude_status, &codex_status]);

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                let bounds = ui.max_rect();
                let cat_rect = egui::Rect::from_min_size(
                    bounds.right_bottom() - Vec2::new(96.0, 88.0),
                    Vec2::new(96.0, 88.0),
                );
                let window = if self.details {
                    let bubble = panel::layout([
                        panel::agent(
                            "Claude",
                            &claude_status,
                            claude,
                            false,
                            self.claude_failure.map(|failure| failure.label()),
                            time,
                            self.offset,
                        ),
                        panel::agent(
                            "Codex",
                            &codex_status,
                            &self.codex_records,
                            false,
                            (!self.codex_online).then_some("Немає зв’язку · повторюю підключення"),
                            time,
                            self.offset,
                        ),
                    ]);
                    let window = panel::window_size(&bubble);
                    if bounds.width() >= window.x - 0.5 && bounds.height() >= window.y - 0.5 {
                        panel::draw(
                            ui,
                            &bubble,
                            egui::pos2(bounds.right() - panel::MARGIN, cat_rect.top() + 4.0),
                            cat_rect.center().x,
                            &mut self.selected_provider,
                        );
                    }
                    window
                } else {
                    let (window, clicked) = self.busy.strip(
                        ui,
                        ctx,
                        bounds,
                        cat_rect,
                        [&claude_status, &codex_status],
                        time,
                        self.offset,
                    );
                    if let Some(agent) = clicked {
                        // The full panel opens on the agent whose chip was clicked.
                        self.details = true;
                        self.selected_provider = agent;
                        ctx.request_repaint();
                    }
                    window
                };

                let cat = ui
                    .interact(cat_rect, ui.id().with("cat"), egui::Sense::click_and_drag())
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                if cat.drag_started() {
                    ctx.send_viewport_cmd(ViewportCommand::StartDrag);
                }
                if cat.clicked() {
                    self.details = !self.details;
                    ctx.request_repaint();
                }
                if self.open_size != window {
                    self.open_size = window;
                    // Grow up and left from the bottom-right corner, so the cat itself never moves.
                    if let Some(rect) = ctx.input(|i| i.viewport().outer_rect) {
                        ctx.send_viewport_cmd(ViewportCommand::OuterPosition(rect.max - window));
                    }
                    ctx.send_viewport_cmd(ViewportCommand::InnerSize(window));
                }
                // The cat sleeps only while every agent with data is blocked.
                self.pet
                    .draw(ui, cat_rect, !asleep || self.details || cat.hovered());
            });
        ctx.request_repaint_after(Duration::from_secs(1));
    }
}

/// The closed cat sleeps only while every agent with data is blocked.
fn asleep(agents: [&Status; 2]) -> bool {
    let known: Vec<_> = agents.iter().filter(|s| s.used.is_some()).collect();
    !known.is_empty() && known.iter().all(|s| s.tone == Tone::Blocked)
}

/// Three notifications, each at most once per window: 30 minutes left, access is back,
/// and 80% of the week gone with more than two days to go.
fn alerts(
    notified: &mut HashSet<(String, u64)>,
    blocked_before: &mut [Option<u64>; 2],
    agents: [(&str, &Status, &[Record]); 2],
    time: u64,
    offset: i64,
) -> Vec<(String, String)> {
    let mut messages = Vec::new();
    for (i, (name, status, records)) in agents.into_iter().enumerate() {
        let now_blocked = status.blocked.map(|(_, reset)| reset);
        if let Some(reset) = blocked_before[i]
            && now_blocked.is_none()
            && time >= reset
            && status.used.is_some()
            && records.iter().any(|row| {
                row.time >= reset
                    && row.time <= time
                    && advice::WINDOWS.iter().any(|(_, select)| {
                        row.sample
                            .rate_limits
                            .as_ref()
                            .and_then(select)
                            .is_some_and(|limit| {
                                limit.resets_at > time && limit.used_percentage < 100.0
                            })
                    })
            })
        {
            messages.push((
                format!("{name} знову доступний"),
                "Можна продовжувати роботу.".to_owned(),
            ));
        }
        blocked_before[i] = if now_blocked.is_none()
            && !messages
                .iter()
                .any(|(title, _)| title == &format!("{name} знову доступний"))
        {
            blocked_before[i]
        } else {
            now_blocked
        };
        if let Some(risk) = &status.risk
            && risk.minutes <= 30.0
            && notified.insert((format!("{name} risk"), risk.reset_at))
        {
            messages.push((
                format!(
                    "{name}: ≈ {} до ліміту {}",
                    duration(risk.minutes),
                    risk.window
                ),
                format!(
                    "Закінчиться {}, оновлення {}.",
                    advice::clock(time + (risk.minutes * 60.0) as u64, time, offset),
                    advice::clock(risk.reset_at, time, offset)
                ),
            ));
        }
        if let Some((limit, seen)) = usage::last_window(records, advice::WINDOWS[1].1)
            && time.saturating_sub(seen) <= 1800
            && limit.used_percentage >= 80.0
            && limit.used_percentage < 100.0
            && limit.resets_at > time + 2 * 86_400
            && notified.insert((format!("{name} week"), limit.resets_at))
        {
            messages.push((
                format!(
                    "{name}: тиждень використано на {:.0}%",
                    limit.used_percentage
                ),
                format!(
                    "Оновлення {}. Розподіли ≈ {:.0}% на день.",
                    advice::clock(limit.resets_at, time, offset),
                    advice::per_day(limit, time)
                ),
            ));
        }
    }
    messages
}

/// The quota in one line under the busy card.
fn quota_line(status: &Status, now: u64, offset: i64) -> String {
    if let Some((_, reset)) = status.blocked {
        format!("Квота на паузі до {}", advice::hhmm(reset, offset))
    } else if let Some(risk) = &status.risk {
        format!("Ліміт {} ≈ через {}", risk.window, duration(risk.minutes))
    } else if let Some((_, _, _, reset)) = status.lasting {
        format!(
            "Квоти вистачить {}",
            advice::clock(reset, now, offset).replacen("о ", "до ", 1)
        )
    } else if let Some(used) = status.used {
        format!("Квота: {:.0}% лишилось", (100.0 - used).max(0.0))
    } else {
        "Квота: ще немає даних".into()
    }
}

fn duration(minutes: f64) -> String {
    let minutes = minutes.round() as u64;
    if minutes >= 1440 {
        // Round to whole hours first, so 23.5 h never shows up as "24 год".
        let hours = minutes.div_ceil(60);
        format!("{} дн {} год", hours / 24, hours % 24)
    } else if minutes >= 60 {
        format!("{} год {:02} хв", minutes / 60, minutes % 60)
    } else {
        format!("{minutes} хв")
    }
}

fn demo_codex_history(time: u64) -> Vec<Record> {
    (0_u64..=16).map(|i| {
        let row = serde_json::json!({
            "time": time - (16 - i) * 225,
            "session_id": "demo-codex",
            "prompt_id": i.to_string(),
            "rate_limits": {
                "five_hour": { "used_percentage": 44.0 + i as f64 * 2.0, "resets_at": time + 4800 },
                "seven_day": { "used_percentage": 32.0 + i as f64 * 0.3, "resets_at": time + 86400 }
            },

        });
        serde_json::from_value(row).expect("valid synthetic demo record")
    }).collect()
}

fn demo_busy(time: u64) -> [Vec<activity::Session>; 2] {
    let session = |project: &str, minutes: u64, tool: Option<&str>| activity::Session {
        id: project.into(),
        project: project.into(),
        since: time - minutes * 60,
        tool: tool.map(Into::into),
    };
    [
        vec![
            session("demo-site", 12, Some("Bash")),
            session("demo-bot", 3, None),
        ],
        vec![session("demo-api", 7, Some("shell"))],
    ]
}

fn demo_claude_snapshot(time: u64) -> Record {
    serde_json::from_value(serde_json::json!({
        "time": time,
        "session_id": "demo-claude",
        "rate_limits": {
            "seven_day": { "used_percentage": 100.0, "resets_at": time + 57_600 }
        }
    }))
    .expect("valid synthetic Claude snapshot")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_estimates_a_near_term_five_hour_limit() {
        let time = 1_800_000_000;
        let rows = demo_codex_history(time);
        let estimate = usage::forecast(&rows, |r| r.five_hour, time).unwrap();
        assert_eq!(estimate.typical_prompts, Some(12.0));
        assert!((estimate.minutes.unwrap() - 45.0).abs() < 0.01);
        assert!(usage::forecast(&rows, |r| r.five_hour, time + 1801).is_none());
        assert!(usage::forecast(&rows, |r| r.five_hour, time + 4800).is_none());
        assert!(
            usage::forecast(&rows[15..], |r| r.five_hour, time)
                .unwrap()
                .minutes
                .is_none()
        );
    }

    fn status(tone: Tone, used: Option<f64>) -> Status {
        Status {
            tone,
            used,
            blocked: None,
            risk: None,
            lasting: None,
            opus: false,
        }
    }

    #[test]
    fn closed_cat_sleeps_only_when_every_known_agent_is_blocked() {
        let blocked = Status {
            blocked: Some(("5 год", 3600)),
            ..status(Tone::Blocked, Some(100.0))
        };
        let unknown = status(Tone::Unknown, None);
        assert!(asleep([&blocked, &unknown]));
        let free = status(Tone::Calm, Some(10.0));
        assert!(!asleep([&blocked, &free]));
        assert!(!asleep([&unknown, &unknown]));
    }

    #[test]
    fn alerts_fire_once_and_on_return() {
        let time = 1_000_000;
        let mut notified = HashSet::new();
        let mut before = [None; 2];
        let risky = Status {
            risk: Some(advice::Risk {
                window: "5 год",
                minutes: 25.0,
                reset_at: time + 7200,
                prompts: None,
            }),
            ..status(Tone::Warn, Some(90.0))
        };
        let calm = status(Tone::Calm, Some(10.0));
        let agents = [("Claude", &risky, &[][..]), ("Codex", &calm, &[][..])];
        assert_eq!(alerts(&mut notified, &mut before, agents, time, 0).len(), 1);
        assert!(alerts(&mut notified, &mut before, agents, time + 60, 0).is_empty());

        let blocked = Status {
            blocked: Some(("5 год", time + 600)),
            ..status(Tone::Blocked, Some(100.0))
        };
        let agents = [("Claude", &blocked, &[][..]), ("Codex", &calm, &[][..])];
        assert!(alerts(&mut notified, &mut before, agents, time, 0).is_empty());
        let recovered = demo_codex_history(time + 600);
        let agents = [
            ("Claude", &calm, recovered.as_slice()),
            ("Codex", &calm, &[][..]),
        ];
        let back = alerts(&mut notified, &mut before, agents, time + 600, 0);
        assert_eq!(back[0].0, "Claude знову доступний");
    }

    #[test]
    fn reset_without_fresh_quota_does_not_announce_recovery() {
        let mut notified = HashSet::new();
        let mut before = [Some(1000), None];
        let unknown = status(Tone::Unknown, None);
        assert!(
            alerts(
                &mut notified,
                &mut before,
                [("Claude", &unknown, &[]), ("Codex", &unknown, &[])],
                1001,
                0
            )
            .is_empty()
        );
        assert_eq!(before[0], Some(1000));
        let available = status(Tone::Calm, Some(1.0));
        let recovered = vec![Record {
            time: 1002,
            sample: usage::Sample {
                time: 1002,
                session_id: "test".into(),
                prompt_id: None,
                rate_limits: Some(usage::Limits {
                    five_hour: Some(usage::Limit {
                        used_percentage: 1.0,
                        resets_at: 2000,
                    }),
                    seven_day: None,
                }),
                cost: None,
                context_window: None,
                model: None,
            },
        }];
        assert_eq!(
            alerts(
                &mut notified,
                &mut before,
                [
                    ("Claude", &available, recovered.as_slice()),
                    ("Codex", &unknown, &[])
                ],
                1002,
                0
            )
            .len(),
            1
        );
    }

    #[test]
    fn stale_available_quota_is_not_presented_as_current() {
        let time = 1_800_000_000;
        let rows = demo_codex_history(time);
        assert!(advice::assess(&rows, time).used.is_some());
        assert!(advice::assess(&rows, time + 1801).used.is_none());
    }

    #[test]
    fn claude_demo_does_not_inherit_codex_measurements() {
        let time = 1_800_000_000;
        let claude = [demo_claude_snapshot(time)];
        let codex = demo_codex_history(time);
        assert!(usage::forecast(&claude, |r| r.five_hour, time).is_none());
        assert_eq!(
            usage::blocked_until(&claude, |r| r.seven_day, time),
            Some(time + 57_600)
        );
        assert!(usage::blocked_until(&codex, |r| r.seven_day, time).is_none());
    }
}
