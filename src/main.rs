#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[macro_use]
mod i18n;
mod activity;
mod advice;
mod claude;
mod codex;
mod estimate;
mod launch;
mod panel;
mod pet;
mod projects;
mod runtime;
mod settings;
mod sound;
mod statusline;
#[cfg(windows)]
mod tray;
mod turns;
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
            println!("Finished turns remembered: {}", turns::load().len());
            for (agent, name) in ["Claude", "Codex"].into_iter().enumerate() {
                let found = projects::list(agent);
                let names: Vec<String> = found
                    .iter()
                    .map(|project| {
                        let mark = if project.preferred { "*" } else { "" };
                        format!("{mark}{}", project.name)
                    })
                    .collect();
                println!("{name} projects ({}): {}", found.len(), names.join(", "));
            }
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
                        runtime::Update::Finished(agent) => {
                            println!("Turn finished: {}", ["Claude", "Codex"][agent]);
                        }
                        runtime::Update::Projects([claude, codex]) => {
                            println!(
                                "Recent projects: Claude {}, Codex {}",
                                claude.len(),
                                codex.len()
                            );
                        }
                        runtime::Update::Tokens(agent, minutes) => {
                            let now = usage::now();
                            let sum = |span: u64| -> u64 {
                                minutes
                                    .iter()
                                    .filter(|(at, _)| *at + span >= now)
                                    .map(|(_, tokens)| tokens)
                                    .sum()
                            };
                            println!(
                                "{} tokens: {} in 5 h, {} in 7 days",
                                ["Claude", "Codex"][agent],
                                sum(5 * 3600),
                                sum(7 * 86_400)
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
            let settings = settings::load();
            i18n::set(settings.language);
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            panel::install_fonts(&cc.egui_ctx);
            Ok(Box::new(Companion {
                demo,
                pet: pet::Pet::load(&cc.egui_ctx),
                settings,
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
                details_seen: None,
                menu_focused: false,
                menu_area: None,
                settings_open: false,
                menu_tab: 0,
                compose: Default::default(),
                compose_hover: None,
                compose_rearm: false,
                compose_area: None,
                compose_typing: false,
                compose_away: None,
                compose_close_at: None,
                projects: Default::default(),
                tokens: Default::default(),
                open_size: Vec2::new(96.0, 88.0),
                home: None,
                shift: Vec2::ZERO,
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
    /// Last moment the open panel had the pointer on it.
    details_seen: Option<f64>,
    /// The open menu has had keyboard focus, so losing it means a click elsewhere.
    menu_focused: bool,
    /// The menu and the cat, where the pointer keeps the menu open.
    menu_area: Option<[egui::Rect; 2]>,
    settings_open: bool,
    /// The agent tab shown in the menu.
    menu_tab: usize,
    /// The task composer on the sleeping cat.
    compose: panel::Compose,
    /// Since when the pointer rests on the sleeping cat, to open the composer.
    compose_hover: Option<f64>,
    /// Closed while the pointer was on the cat: wait for it to leave before opening again.
    compose_rearm: bool,
    /// The composer and the cat, where the pointer keeps the composer open.
    compose_area: Option<[egui::Rect; 2]>,
    compose_typing: bool,
    compose_away: Option<f64>,
    /// After a task starts, the confirmation stays this long, then the composer closes.
    compose_close_at: Option<f64>,
    /// Recent project folders: Claude, then Codex.
    projects: [Vec<projects::Project>; 2],
    /// Tokens per minute over the last eight days: Claude, then Codex.
    tokens: [Vec<(u64, u64)>; 2],
    open_size: Vec2,
    /// Where the cat's bottom-right corner lives when nothing is open.
    home: Option<Pos2>,
    /// How far the window was pushed to keep an open card on screen.
    shift: Vec2,
    positioned: bool,
    offset: i64,
    notified: HashSet<(String, u64)>,
    blocked_before: [Option<u64>; 2],
    busy: Busy,
    settings: settings::Settings,
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
        records: [&[Record]; 2],
        now: u64,
        offset: i64,
    ) -> (Vec2, Option<usize>) {
        let agents: Vec<usize> = (0..2).filter(|i| !self.sessions[*i].is_empty()).collect();
        if agents.is_empty() {
            self.hover = None;
            self.shown = None;
            // Headroom above the cat for the sleeping z letters; the window is transparent.
            return (Vec2::new(96.0, 88.0 + panel::STRIP), None);
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
        let size = panel::busy_card_size(&self.sessions[agent]);
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
                &panel::quotas(records[agent], now, offset),
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
        if ctx.input(|i| i.time) < SETTLE_SECONDS {
            ctx.request_repaint_after(Duration::from_millis(900));
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
                    runtime::Update::Tokens(agent, minutes) => self.tokens[agent] = minutes,
                    runtime::Update::Projects(found) => self.projects = found,
                    runtime::Update::Finished(_) => {
                        if self.settings.sound {
                            sound::play();
                        }
                    }
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
        // Awake while an agent works, or while someone is looking at it; asleep when nothing runs.
        let working = self.busy.sessions.iter().any(|list| !list.is_empty());

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                let bounds = ui.max_rect();
                let cat_rect = egui::Rect::from_min_size(
                    bounds.right_bottom() - Vec2::new(96.0, 88.0),
                    Vec2::new(96.0, 88.0),
                );
                // Resting the pointer on the sleeping cat opens the task composer.
                let clock = ui.input(|input| input.time);
                let on_cat = ctx
                    .pointer_hover_pos()
                    .is_some_and(|pointer| cat_rect.contains(pointer));
                if !on_cat {
                    self.compose_rearm = false;
                }
                if !self.details && !working && !self.compose.open && on_cat && !self.compose_rearm
                {
                    let since = *self.compose_hover.get_or_insert(clock);
                    if clock - since >= COMPOSE_DWELL {
                        self.compose.open = true;
                        self.compose.focus = true;
                        self.compose.note = None;
                        self.compose_hover = None;
                    } else {
                        ctx.request_repaint_after(Duration::from_millis(100));
                    }
                } else {
                    self.compose_hover = None;
                }
                let window = if self.details {
                    let menu = panel::menu([
                        panel::row(
                            0,
                            &claude_status,
                            claude,
                            self.claude_failure.map(|failure| failure.label()),
                            spent(&self.tokens[0], claude, time),
                            time,
                            self.offset,
                        ),
                        panel::row(
                            1,
                            &codex_status,
                            &self.codex_records,
                            (!self.codex_online).then_some(i18n::s(
                                "Немає зв’язку · повторюю підключення",
                                "Нет связи · переподключаюсь",
                                "No connection · reconnecting",
                            )),
                            spent(&self.tokens[1], &self.codex_records, time),
                            time,
                            self.offset,
                        ),
                    ]);
                    let window = panel::window_size(&menu);
                    if bounds.width() >= window.x - 0.5 && bounds.height() >= window.y - 0.5 {
                        let (area, action) = panel::draw(
                            ui,
                            &menu,
                            egui::pos2(bounds.right() - panel::MARGIN, cat_rect.top() + 4.0),
                            &mut self.settings_open,
                            &mut self.menu_tab,
                            &self.settings,
                        );
                        // The pointer keeps the menu open only on the menu itself or the cat,
                        // not on the transparent rest of the window.
                        self.menu_area = Some([area, cat_rect]);
                        match action {
                            Some(panel::Action::Language(lang)) => {
                                self.settings.language = lang;
                                i18n::set(lang);
                                let _ = settings::save(&self.settings);
                            }
                            Some(panel::Action::Sound(on)) => {
                                self.settings.sound = on;
                                let _ = settings::save(&self.settings);
                                if on {
                                    sound::play();
                                }
                            }
                            Some(panel::Action::Preview) => sound::play(),
                            None => {}
                        }
                    }
                    window
                } else if self.compose.open {
                    let window = panel::compose_window(panel::compose_size());
                    if bounds.width() >= window.x - 0.5 && bounds.height() >= window.y - 0.5 {
                        let out = panel::compose(
                            ui,
                            egui::pos2(bounds.right() - panel::MARGIN, cat_rect.top() + 4.0),
                            &mut self.compose,
                            &self.projects,
                        );
                        self.compose_area = Some([out.area, cat_rect]);
                        self.compose_typing = out.typing;
                        if out.close {
                            self.compose_close_at = Some(clock);
                        }
                        if let Some(send) = out.send {
                            let agent = self.compose.agent;
                            let app = launch::APPS[agent];
                            let text = self.compose.text.trim().to_owned();
                            let result = match &send {
                                panel::Send::New(folder) => launch::start(agent, folder, &text)
                                    .map(|()| {
                                        let project =
                                            activity::folder_name(&folder.to_string_lossy());
                                        t!(
                                            "Відкрито в {app} · {project}",
                                            "Открыто в {app} · {project}",
                                            "Opened in {app} · {project}"
                                        )
                                    }),
                                panel::Send::Existing(id, title) => {
                                    launch::open_session(agent, id).map(|()| {
                                        if text.is_empty() {
                                            t!(
                                                "Відкрито: {title}",
                                                "Открыто: {title}",
                                                "Opened: {title}"
                                            )
                                        } else {
                                            // The apps' session links take no text.
                                            ctx.copy_text(text.clone());
                                            s_paste()
                                        }
                                    })
                                }
                            };
                            match result {
                                Ok(message) => {
                                    self.compose.note = Some((message, true));
                                    self.compose.text.clear();
                                    self.compose_close_at = Some(clock + 1.8);
                                }
                                Err(failure) => {
                                    self.compose.note = Some((failure.label(agent), false));
                                }
                            }
                        }
                    }
                    window
                } else {
                    let (window, clicked) = self.busy.strip(
                        ui,
                        ctx,
                        bounds,
                        cat_rect,
                        [claude, &self.codex_records],
                        time,
                        self.offset,
                    );
                    if let Some(agent) = clicked {
                        // A click on the bubble opens the menu on that agent.
                        self.menu_tab = agent;
                        self.details = true;
                        self.settings_open = false;
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
                let outer = ctx.input(|i| i.viewport().outer_rect);
                // Follow drags while nothing is pushed aside; ignore sub-point rounding at 125%
                // scale, or the cat would creep a pixel with every open and close.
                if self.shift == Vec2::ZERO
                    && let Some(rect) = outer
                    && self
                        .home
                        .is_none_or(|home| (home - rect.max).length() > 2.0)
                {
                    self.home = Some(rect.max);
                }
                if self.open_size != window && ctx.input(|i| i.time) >= SETTLE_SECONDS {
                    self.open_size = window;
                    // Grow up and left from the cat's corner, so the cat itself stays put.
                    if let Some(anchor) = self.home.or(outer.map(|rect| rect.max)) {
                        let wanted = anchor - window;
                        let mut at = wanted;
                        // Near a screen edge the card would open off-screen: slide the window
                        // in instead, and let the cat come home when it closes.
                        // shortcut: assumes the cat's monitor starts at 0,0; other monitors skip this.
                        if let Some(monitor) = ctx.input(|i| i.viewport().monitor_size)
                            && (0.0..=monitor.x).contains(&anchor.x)
                            && (0.0..=monitor.y).contains(&anchor.y)
                        {
                            at.x = at.x.clamp(0.0, (monitor.x - window.x).max(0.0));
                            at.y = at.y.clamp(0.0, (monitor.y - window.y).max(0.0));
                        }
                        self.shift = at - wanted;
                        ctx.send_viewport_cmd(ViewportCommand::OuterPosition(at));
                    }
                    ctx.send_viewport_cmd(ViewportCommand::InnerSize(window));
                }
                // The cat sleeps only while every agent with data is blocked.
                let awake = cat_awake(
                    asleep,
                    working,
                    self.details || self.compose.open,
                    cat.hovered(),
                );
                self.pet.draw(ui, cat_rect, awake);
                if !awake {
                    let time = (!self.busy.reduced).then(|| ui.input(|input| input.time));
                    panel::sleep_zs(ui.painter(), cat_rect, time);
                }
            });
        // The composer closes on Escape, once a task has started, when the menu opens, when an
        // agent starts working with nothing typed, or when the pointer leaves it empty-handed.
        if self.compose.open {
            let clock = ctx.input(|input| input.time);
            let on = ctx.pointer_hover_pos().is_some_and(|pointer| {
                self.compose_area
                    .is_some_and(|areas| areas.iter().any(|area| area.contains(pointer)))
            });
            let empty = self.compose.text.trim().is_empty();
            if on || self.compose_typing || !empty {
                self.compose_away = None;
            } else {
                self.compose_away.get_or_insert(clock);
            }
            let close = ctx.input(|input| input.key_pressed(egui::Key::Escape))
                || self.compose_away.is_some_and(|since| clock - since >= 1.2)
                || self.compose_close_at.is_some_and(|at| clock >= at)
                || self.details
                || (working && empty);
            if close {
                self.compose.open = false;
                self.compose.note = None;
                self.compose.picking = panel::Picking::Nothing;
                self.compose_area = None;
                self.compose_typing = false;
                self.compose_away = None;
                self.compose_close_at = None;
                self.compose_rearm = true;
                ctx.request_repaint();
            } else {
                ctx.request_repaint_after(Duration::from_millis(200));
            }
        }
        // An open menu nobody looks at closes itself: only the pointer resting on the menu or
        // the cat keeps it open. A click in another window closes it at once.
        if self.details {
            let clock = ctx.input(|input| input.time);
            let seen = *self.details_seen.get_or_insert(clock);
            let focused = ctx.input(|input| input.focused);
            let on_menu = ctx.pointer_hover_pos().is_some_and(|pointer| {
                self.menu_area
                    .is_some_and(|areas| areas.iter().any(|area| area.contains(pointer)))
            });
            if on_menu {
                self.details_seen = Some(clock);
            }
            if focused {
                self.menu_focused = true;
            }
            if (self.menu_focused && !focused) || clock - seen >= PANEL_IDLE_SECONDS {
                self.details = false;
                ctx.request_repaint();
            }
            ctx.request_repaint_after(Duration::from_millis(500));
        } else {
            self.details_seen = None;
            self.menu_focused = false;
            self.menu_area = None;
            self.settings_open = false;
        }
        ctx.request_repaint_after(Duration::from_secs(1));
    }
}

/// Tokens spent in the current five-hour and weekly quota windows; rolling windows when the
/// reset time is unknown.
fn spent(minutes: &[(u64, u64)], records: &[Record], now: u64) -> [Option<u64>; 2] {
    if minutes.is_empty() {
        return [None; 2];
    }
    let windows = advice::windows();
    [(windows[0].1, 5 * 3600), (windows[1].1, 7 * 86_400)].map(|(select, span)| {
        let from = usage::last_window(records, select)
            .filter(|(limit, _)| limit.resets_at > now)
            .map_or(now.saturating_sub(span), |(limit, _)| {
                limit.resets_at.saturating_sub(span)
            });
        Some(
            minutes
                .iter()
                .filter(|(at, _)| *at + 60 > from)
                .map(|(_, tokens)| tokens)
                .sum(),
        )
    })
}

/// Startup grace: the system places the window (docking it bottom-right) a moment after the
/// first frame, and egui reports the old position until then. Resizing before it settles would
/// anchor the cat to that stale corner and send it to the top left of the screen.
const SETTLE_SECONDS: f64 = 0.8;

fn s_paste() -> String {
    i18n::s(
        "Відкрито · текст у буфері, встав Ctrl+V",
        "Открыто · текст в буфере, вставь Ctrl+V",
        "Opened · text copied, paste with Ctrl+V",
    )
    .to_owned()
}

/// How long the pointer rests on the sleeping cat before the task composer opens.
const COMPOSE_DWELL: f64 = 0.6;

/// The open panel closes after this long without the pointer on it.
const PANEL_IDLE_SECONDS: f64 = 30.0;

/// The cat wakes for work or attention, and sleeps when every agent is blocked or nothing runs.
fn cat_awake(blocked: bool, working: bool, details: bool, hovered: bool) -> bool {
    details || hovered || (working && !blocked)
}

/// Every agent with data is blocked: nothing can run, so the cat rests.
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
        let back = t!(
            "{name} знову доступний",
            "{name} снова доступен",
            "{name} is available again"
        );
        if let Some(reset) = blocked_before[i]
            && now_blocked.is_none()
            && time >= reset
            && status.used.is_some()
            && records.iter().any(|row| {
                row.time >= reset
                    && row.time <= time
                    && advice::windows().iter().any(|(_, select)| {
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
                back.clone(),
                i18n::s(
                    "Можна продовжувати роботу.",
                    "Можно продолжать работу.",
                    "You can continue working.",
                )
                .to_owned(),
            ));
        }
        blocked_before[i] =
            if now_blocked.is_none() && !messages.iter().any(|(title, _)| title == &back) {
                blocked_before[i]
            } else {
                now_blocked
            };
        if let Some(risk) = &status.risk
            && risk.minutes <= 30.0
            && notified.insert((format!("{name} risk"), risk.reset_at))
        {
            messages.push((
                t!(
                    "{name}: ≈ {} до ліміту {}",
                    "{name}: ≈ {} до лимита {}",
                    "{name}: ≈ {} to the {} limit",
                    duration(risk.minutes),
                    risk.window
                ),
                t!(
                    "Закінчиться {}, оновлення {}.",
                    "Закончится {}, обновление {}.",
                    "Runs out {}, resets {}.",
                    advice::clock(time + (risk.minutes * 60.0) as u64, time, offset),
                    advice::clock(risk.reset_at, time, offset)
                ),
            ));
        }
        if let Some((limit, seen)) = usage::last_window(records, advice::windows()[1].1)
            && time.saturating_sub(seen) <= 1800
            && limit.used_percentage >= 80.0
            && limit.used_percentage < 100.0
            && limit.resets_at > time + 2 * 86_400
            && notified.insert((format!("{name} week"), limit.resets_at))
        {
            messages.push((
                t!(
                    "{name}: тиждень використано на {:.0}%",
                    "{name}: неделя израсходована на {:.0}%",
                    "{name}: {:.0}% of the week used",
                    limit.used_percentage
                ),
                t!(
                    "Оновлення {}. Розподіли ≈ {:.0}% на день.",
                    "Обновление {}. Распредели ≈ {:.0}% в день.",
                    "Resets {}. Spread ≈ {:.0}% a day.",
                    advice::clock(limit.resets_at, time, offset),
                    advice::per_day(limit, time)
                ),
            ));
        }
    }
    messages
}

fn duration(minutes: f64) -> String {
    let minutes = minutes.round() as u64;
    if minutes >= 1440 {
        // Round to whole hours first, so 23.5 h never shows up as "24 год".
        let hours = minutes.div_ceil(60);
        t!(
            "{} дн {} год",
            "{} д {} ч",
            "{}d {}h",
            hours / 24,
            hours % 24
        )
    } else if minutes >= 60 {
        t!(
            "{} год {:02} хв",
            "{} ч {:02} мин",
            "{}h {:02}m",
            minutes / 60,
            minutes % 60
        )
    } else {
        t!("{minutes} хв", "{minutes} мин", "{minutes} min")
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
        busy: true,
        end: time,
        prompt: Some(format!("demo task in {project}")),
        tokens: minutes * 1500,
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
        // Idle with quota left: asleep. Work, a hover or the open menu wake it.
        assert!(!cat_awake(false, false, false, false));
        assert!(cat_awake(false, true, false, false));
        assert!(cat_awake(false, false, true, false));
        assert!(cat_awake(false, false, false, true));
        assert!(!cat_awake(true, true, false, false));
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
