//! Independent provider summaries on a monochrome glass surface.

use crate::{
    advice::{self, Status},
    i18n::s,
    usage::{self, Record},
};
use eframe::egui::{
    self, Color32, FontFamily, FontId, Pos2, Rect, Shape, Stroke, StrokeKind, Vec2, epaint::Shadow,
};
use std::sync::Arc;

// Porcelain palette: semantic tokens keep the surface independent of the pet artwork.
const GLASS_TOP: Color32 = Color32::from_rgba_premultiplied(43, 43, 43, 248);
const GLASS_BOTTOM: Color32 = Color32::from_rgba_premultiplied(17, 17, 17, 250);
const LABEL: Color32 = Color32::from_gray(245);
const SECONDARY: Color32 = Color32::from_gray(180);
const GRAY: Color32 = Color32::from_gray(145);

/// Room around the bubble for its soft shadow.
pub const MARGIN: f32 = 14.0;
const PAD: f32 = 18.0;
const WIDTH: f32 = 320.0;
const RADIUS: f32 = 22.0;

/// The system UI font (Segoe UI on Windows, SF on macOS), with egui's own fonts as fallback.
pub fn install_fonts(ctx: &egui::Context) {
    #[cfg(windows)]
    let (regular, semibold, symbols) = {
        let dir = std::path::PathBuf::from(
            std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()),
        )
        .join("Fonts");
        let read = |name: &str| std::fs::read(dir.join(name)).ok();
        (
            read("segoeui.ttf"),
            read("seguisb.ttf"),
            read("seguisym.ttf"),
        )
    };
    #[cfg(target_os = "macos")]
    let (regular, semibold, symbols) = (
        std::fs::read("/System/Library/Fonts/SFNS.ttf").ok(),
        None::<Vec<u8>>,
        None::<Vec<u8>>,
    );
    #[cfg(not(any(windows, target_os = "macos")))]
    let (regular, semibold, symbols) = (None::<Vec<u8>>, None::<Vec<u8>>, None::<Vec<u8>>);

    let mut fonts = egui::FontDefinitions::default();
    let mut add = |name: &str, bytes: Option<Vec<u8>>| {
        bytes.map(|bytes| {
            fonts
                .font_data
                .insert(name.to_owned(), Arc::new(egui::FontData::from_owned(bytes)));
            name.to_owned()
        })
    };
    let regular = add("ui", regular);
    let semibold = add("ui-semibold", semibold);
    let symbols = add("ui-symbols", symbols);
    let fallback = fonts.families[&FontFamily::Proportional].clone();
    let proportional: Vec<String> = regular
        .iter()
        .cloned()
        .chain(fallback.iter().cloned())
        .chain(symbols.iter().cloned())
        .collect();
    let strong: Vec<String> = semibold
        .into_iter()
        .chain(proportional.iter().cloned())
        .collect();
    fonts
        .families
        .insert(FontFamily::Proportional, proportional);
    fonts.families.insert(strong_family(), strong);
    ctx.set_fonts(fonts);
}

fn strong_family() -> FontFamily {
    FontFamily::Name("semibold".into())
}

fn strong(size: f32) -> FontId {
    FontId::new(size, strong_family())
}

fn regular(size: f32) -> FontId {
    FontId::proportional(size)
}

pub struct Quota {
    pub name: &'static str,
    pub remaining: Option<f32>,
    pub reset: String,
}
/// Both quota windows of one provider, as the panel and the agent card show them.
pub fn quotas(records: &[Record], now: u64, offset: i64) -> Vec<Quota> {
    advice::windows()
        .iter()
        .map(|(name, select)| {
            let latest = usage::last_window(records, *select);
            let current = latest.filter(|(limit, seen)| {
                (limit.resets_at == 0 || limit.resets_at > now)
                    && ((limit.used_percentage >= 100.0 && limit.resets_at > 0)
                        || now.saturating_sub(*seen) <= 1800)
            });
            match current {
                Some((limit, _)) => {
                    let left = (100.0 - limit.used_percentage).clamp(0.0, 100.0);
                    Quota {
                        name,
                        remaining: Some(left as f32 / 100.0),
                        reset: if limit.resets_at == 0 {
                            s("Час невідомий", "Время неизвестно", "Time unknown").into()
                        } else {
                            format!("↻ {}", advice::clock(limit.resets_at, now, offset))
                        },
                    }
                }
                None => Quota {
                    name,
                    remaining: None,
                    reset: if latest.is_some() {
                        s(
                            "Очікую оновлення",
                            "Жду обновления",
                            "Waiting for an update",
                        )
                    } else {
                        s(
                            "Ще немає вимірювань",
                            "Ещё нет измерений",
                            "No measurements yet",
                        )
                    }
                    .into(),
                },
            }
        })
        .collect()
}

/// Whether an agent can be used right now.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Verdict {
    Ready,
    Low,
    Paused,
    Unknown,
}

/// One of an agent's limits: what is left, when it resets, and what was spent in it.
pub struct Window {
    pub label: &'static str,
    /// 0..1 of the limit that is left; None when there is no fresh measurement.
    pub left: Option<f32>,
    /// "↻ о 23:49", or why there is nothing to show.
    pub reset: String,
    /// "витрачено 1,6 млн".
    pub spent: Option<String>,
}

/// One agent in the menu: can it be used, and where each limit stands.
pub struct Row {
    pub agent: usize,
    pub verdict: Verdict,
    /// "Можна працювати", "Мало", "Пауза до 19:28", or the reason there is no data.
    pub status: String,
    /// When a paused agent comes back, without a preposition ("19:28").
    pub until: Option<String>,
    pub windows: [Window; 2],
}

/// Below this share left, an agent is "running low".
const LOW: f32 = 0.15;

/// "1,2 млн", "640 тис." or "950" — a count that fits a short line.
pub fn short_tokens(tokens: u64) -> String {
    let comma = |text: String| match crate::i18n::lang() {
        crate::i18n::Lang::En => text,
        _ => text.replace('.', ","),
    };
    if tokens >= 1_000_000 {
        let m = comma(format!("{:.1}", tokens as f64 / 1e6));
        t!("{m} млн", "{m} млн", "{m}M")
    } else if tokens >= 1000 {
        let k = tokens / 1000;
        t!("{k} тис.", "{k} тыс.", "{k}k")
    } else {
        tokens.to_string()
    }
}

/// What the menu says about one agent.
#[allow(clippy::too_many_arguments)]
pub fn row(
    agent: usize,
    status: &Status,
    records: &[Record],
    unavailable: Option<&str>,
    spent: [Option<u64>; 2],
    now: u64,
    offset: i64,
) -> Row {
    let quotas = quotas(records, now, offset);
    let windows = std::array::from_fn(|i| {
        let quota = quotas.get(i);
        Window {
            label: quota.map_or("", |quota| quota.name),
            left: quota.and_then(|quota| quota.remaining),
            reset: quota.map_or_else(String::new, |quota| quota.reset.clone()),
            spent: spent[i].map(|tokens| {
                let amount = short_tokens(tokens);
                t!("витрачено {amount}", "потрачено {amount}", "spent {amount}")
            }),
        }
    });
    let known = windows.iter().any(|window| window.left.is_some());
    let low = windows
        .iter()
        .filter_map(|window| window.left)
        .any(|left| left <= LOW);
    let (verdict, status, until) = if let Some((_, reset)) = status.blocked {
        let at = advice::until(reset, now, offset);
        (
            Verdict::Paused,
            t!("Пауза до {at}", "Пауза до {at}", "Paused until {at}"),
            Some(at),
        )
    } else if !known {
        (
            Verdict::Unknown,
            unavailable
                .unwrap_or(s("Немає даних", "Нет данных", "No data"))
                .to_owned(),
            None,
        )
    } else if low {
        (
            Verdict::Low,
            s("Мало", "Мало", "Running low").to_owned(),
            None,
        )
    } else {
        (
            Verdict::Ready,
            s("Можна працювати", "Можно работать", "Ready to work").to_owned(),
            None,
        )
    };
    Row {
        agent,
        verdict,
        status,
        until,
        windows,
    }
}

/// The one-line answer at the top, built from the same rows so it never disagrees with them.
fn summary(rows: &[Row; 2]) -> (String, String) {
    let name = |row: &Row| ["Claude", "Codex"][row.agent];
    let room = |row: &Row| {
        row.windows
            .iter()
            .filter_map(|window| window.left)
            .fold(f32::INFINITY, f32::min)
    };
    let (a, b) = (&rows[0], &rows[1]);
    // What to say about the other agent when this one is the subject.
    let about = |other: &Row| match other.verdict {
        Verdict::Ready => {
            let n = name(other);
            t!("Працюй у {n}", "Работай в {n}", "Work in {n}")
        }
        Verdict::Low => {
            let n = name(other);
            t!(
                "{n}: лишилось мало",
                "{n}: осталось мало",
                "{n}: running low"
            )
        }
        _ => {
            let n = name(other);
            t!("{n}: немає даних", "{n}: нет данных", "{n}: no data")
        }
    };
    match (a.verdict, b.verdict) {
        (Verdict::Paused, Verdict::Paused) => (
            s("Обидва на паузі", "Оба на паузе", "Both are paused").to_owned(),
            t!(
                "Claude до {} · Codex до {}",
                "Claude до {} · Codex до {}",
                "Claude until {} · Codex until {}",
                a.until.clone().unwrap_or_default(),
                b.until.clone().unwrap_or_default()
            ),
        ),
        (Verdict::Paused, _) | (_, Verdict::Paused) => {
            let (paused, other) = if a.verdict == Verdict::Paused {
                (a, b)
            } else {
                (b, a)
            };
            let (n, at) = (name(paused), paused.until.clone().unwrap_or_default());
            (
                t!(
                    "{n} на паузі до {at}",
                    "{n} на паузе до {at}",
                    "{n} paused until {at}"
                ),
                about(other),
            )
        }
        (Verdict::Low, Verdict::Low) => (
            s(
                "Обидва майже вичерпано",
                "Оба почти исчерпаны",
                "Both are almost out",
            )
            .to_owned(),
            String::new(),
        ),
        (Verdict::Low, _) | (_, Verdict::Low) => {
            let (low, other) = if a.verdict == Verdict::Low {
                (a, b)
            } else {
                (b, a)
            };
            let n = name(low);
            (
                t!(
                    "У {n} лишилось мало",
                    "У {n} осталось мало",
                    "{n} is running low"
                ),
                about(other),
            )
        }
        (Verdict::Ready, Verdict::Ready) => {
            let detail = if (room(a) - room(b)).abs() >= 0.15 {
                let n = name(if room(a) > room(b) { a } else { b });
                t!(
                    "Більше запасу в {n}",
                    "Больше запаса в {n}",
                    "More room in {n}"
                )
            } else {
                String::new()
            };
            (
                s(
                    "Обидва готові до роботи",
                    "Оба готовы к работе",
                    "Both are ready to work",
                )
                .to_owned(),
                detail,
            )
        }
        (Verdict::Ready, Verdict::Unknown) | (Verdict::Unknown, Verdict::Ready) => {
            let (ready, other) = if a.verdict == Verdict::Ready {
                (a, b)
            } else {
                (b, a)
            };
            let n = name(ready);
            (
                t!(
                    "{n} готовий до роботи",
                    "{n} готов к работе",
                    "{n} is ready to work"
                ),
                about(other),
            )
        }
        (Verdict::Unknown, Verdict::Unknown) => (
            s(
                "Поки нічого не бачу",
                "Пока ничего не вижу",
                "Nothing to see yet",
            )
            .to_owned(),
            s(
                "Попрацюй у Claude Code чи Codex",
                "Поработай в Claude Code или Codex",
                "Work in Claude Code or Codex",
            )
            .to_owned(),
        ),
    }
}

const HEADER: f32 = 62.0;
const ROW_HEIGHT: f32 = 124.0;
/// The Claude | Codex switch under the header.
const TABS: f32 = 44.0;

/// Soft status colors, kept out of the monochrome surface so a status reads at a glance.
const READY: Color32 = Color32::from_rgb(126, 200, 150);
const WARN: Color32 = Color32::from_rgb(240, 184, 92);
const STOP: Color32 = Color32::from_rgb(236, 112, 102);

fn verdict_color(verdict: Verdict) -> Color32 {
    match verdict {
        Verdict::Ready => READY,
        Verdict::Low => WARN,
        Verdict::Paused => STOP,
        Verdict::Unknown => GRAY,
    }
}

/// The menu: one answer at the top, each agent below it, and settings behind the gear.
pub struct Menu {
    pub title: String,
    pub detail: String,
    pub rows: [Row; 2],
    pub size: Vec2,
}

pub fn menu(rows: [Row; 2]) -> Menu {
    let (title, detail) = summary(&rows);
    Menu {
        title,
        detail,
        rows,
        size: Vec2::new(WIDTH, PAD + HEADER + TABS + ROW_HEIGHT + PAD - 8.0),
    }
}

pub fn window_size(menu: &Menu) -> Vec2 {
    Vec2::new(
        menu.size.x + MARGIN * 2.0,
        88.0 + menu.size.y + MARGIN + 6.0,
    )
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let [ar, ag, ab, aa] = a.to_array();
    let [br, bg, bb, ba] = b.to_array();
    let lerp = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(lerp(ar, br), lerp(ag, bg), lerp(ab, bb), lerp(aa, ba))
}

/// A rounded rectangle filled with a vertical gradient: a triangle fan with per-vertex colors.
fn gradient(painter: &egui::Painter, rect: Rect, radius: f32, top: Color32, bottom: Color32) {
    let color = |y: f32| {
        mix(
            top,
            bottom,
            ((y - rect.top()) / rect.height()).clamp(0.0, 1.0),
        )
    };
    // Let egui create the rounded contour AND its anti-aliased fringe, then tint vertices.
    let shape = egui::epaint::ClippedShape {
        clip_rect: painter.clip_rect(),
        shape: Shape::rect_filled(rect, radius, Color32::WHITE),
    };
    for primitive in painter
        .ctx()
        .tessellate(vec![shape], painter.ctx().pixels_per_point())
    {
        if let egui::epaint::Primitive::Mesh(mut mesh) = primitive.primitive {
            for vertex in &mut mesh.vertices {
                vertex.color = color(vertex.pos.y).gamma_multiply(vertex.color.a() as f32 / 255.0);
            }
            painter.add(Shape::mesh(mesh));
        }
    }
}

/// A choice made in the settings view, for the app to apply and save.
pub enum Action {
    Language(crate::i18n::Lang),
    Sound(bool),
    Preview,
}

/// Draws the menu with its bottom edge just above the cat. Returns the menu's rectangle and
/// any settings choice.
pub fn draw(
    ui: &egui::Ui,
    menu: &Menu,
    bottom_right: Pos2,
    settings_open: &mut bool,
    tab: &mut usize,
    settings: &crate::settings::Settings,
) -> (Rect, Option<Action>) {
    let painter = ui.painter();
    let rect = Rect::from_min_max(bottom_right - menu.size, bottom_right);
    painter.add(
        Shadow {
            offset: [0, 10],
            blur: 24,
            spread: 0,
            color: Color32::from_black_alpha(110),
        }
        .as_shape(rect, RADIUS),
    );
    gradient(painter, rect, RADIUS, GLASS_TOP, GLASS_BOTTOM);
    painter.rect_stroke(
        rect,
        RADIUS,
        Stroke::new(1.0, Color32::from_white_alpha(28)),
        StrokeKind::Inside,
    );
    let (left, right) = (rect.left() + PAD + 4.0, rect.right() - PAD - 4.0);
    let top = rect.top() + PAD;

    // Header: the verdict, or the settings title, and the gear.
    let gear = Rect::from_min_size(Pos2::new(right - 30.0, top - 4.0), Vec2::splat(30.0));
    let response = ui
        .interact(gear, ui.id().with("settings"), egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            true,
            *settings_open,
            s("Налаштування", "Настройки", "Settings"),
        )
    });
    if response.clicked() {
        *settings_open = !*settings_open;
    }
    if *settings_open || response.hovered() {
        painter.rect_filled(
            gear,
            9.0,
            Color32::from_white_alpha(if *settings_open { 34 } else { 14 }),
        );
    }
    painter.text(
        gear.center(),
        egui::Align2::CENTER_CENTER,
        if *settings_open { "✕" } else { "⚙" },
        regular(15.0),
        if *settings_open || response.hovered() {
            LABEL
        } else {
            SECONDARY
        },
    );
    let (title, detail) = if *settings_open {
        (
            s("Налаштування", "Настройки", "Settings").to_owned(),
            String::new(),
        )
    } else {
        (menu.title.clone(), menu.detail.clone())
    };
    let title = painter.layout(title, strong(15.5), LABEL, right - left - 40.0);
    painter.galley(Pos2::new(left, top), title, LABEL);
    let detail = painter.layout(detail, regular(11.5), SECONDARY, right - left - 40.0);
    painter.galley(Pos2::new(left, top + 24.0), detail, SECONDARY);

    let area = Rect::from_min_max(
        Pos2::new(left, top + HEADER - 8.0),
        Pos2::new(right, rect.bottom() - PAD),
    );
    let action = if *settings_open {
        settings_body(ui, area, settings)
    } else {
        // Tabs: one per agent, each with a dot in its status color.
        *tab = (*tab).min(1);
        let tabs = Rect::from_min_size(
            Pos2::new(left, area.top() + 4.0),
            Vec2::new(right - left, 32.0),
        );
        painter.rect_filled(tabs, 10.0, Color32::from_black_alpha(65));
        for (i, row) in menu.rows.iter().enumerate() {
            let cell = Rect::from_min_size(
                tabs.min + Vec2::new(i as f32 * tabs.width() / 2.0, 0.0),
                Vec2::new(tabs.width() / 2.0, tabs.height()),
            )
            .shrink(3.0);
            let response = ui
                .interact(cell, ui.id().with(("agent-tab", i)), egui::Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            let name = ["Claude", "Codex"][row.agent];
            response.widget_info(|| {
                egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, *tab == i, name)
            });
            if response.clicked() {
                *tab = i;
            }
            if *tab == i {
                gradient(
                    painter,
                    cell,
                    8.0,
                    Color32::from_white_alpha(38),
                    Color32::from_white_alpha(18),
                );
            } else if response.hovered() {
                painter.rect_filled(cell, 8.0, Color32::from_white_alpha(8));
            }
            let label = painter.layout_no_wrap(name.to_owned(), strong(12.5), LABEL);
            let width = label.size().x + 12.0;
            let start = cell.center().x - width / 2.0;
            painter.circle_filled(
                Pos2::new(start + 3.0, cell.center().y),
                3.5,
                verdict_color(row.verdict),
            );
            painter.galley(
                Pos2::new(start + 12.0, cell.center().y - label.size().y / 2.0),
                label,
                if *tab == i { LABEL } else { SECONDARY },
            );
        }
        // The chosen agent: its status, then its two limits side by side.
        let row = &menu.rows[*tab];
        let y = area.top() + TABS;
        let color = tint(row.agent);
        painter.text(
            Pos2::new(left, y + 6.0),
            egui::Align2::LEFT_TOP,
            &row.status,
            strong(13.0),
            verdict_color(row.verdict),
        );
        let gap = 16.0;
        let column = (right - left - gap) / 2.0;
        for (j, window) in row.windows.iter().enumerate() {
            let x = left + j as f32 * (column + gap);
            painter.text(
                Pos2::new(x, y + 34.0),
                egui::Align2::LEFT_TOP,
                window.label,
                regular(10.5),
                SECONDARY,
            );
            let (big, big_color) = match window.left {
                Some(left) => (format!("{:.0}%", left * 100.0), LABEL),
                None => ("—".to_owned(), GRAY),
            };
            painter.text(
                Pos2::new(x, y + 47.0),
                egui::Align2::LEFT_TOP,
                big,
                strong(24.0),
                big_color,
            );
            let track = Rect::from_min_size(Pos2::new(x, y + 80.0), Vec2::new(column, 4.0));
            painter.rect_filled(track, 2.0, Color32::from_white_alpha(25));
            if let Some(share) = window.left.filter(|share| *share > 0.0) {
                let fill = Rect::from_min_size(track.min, Vec2::new(track.width() * share, 4.0));
                painter.rect_filled(fill, 2.0, if share <= LOW { WARN } else { color });
            }
            painter.text(
                Pos2::new(x, y + 89.0),
                egui::Align2::LEFT_TOP,
                &window.reset,
                regular(10.0),
                SECONDARY,
            );
            if let Some(spent) = &window.spent {
                painter.text(
                    Pos2::new(x, y + 103.0),
                    egui::Align2::LEFT_TOP,
                    spent,
                    regular(10.0),
                    color.gamma_multiply(0.9),
                );
            }
        }
        None
    };
    (rect, action)
}

/// Language and sound, in the space the agent lines use.
fn settings_body(
    ui: &egui::Ui,
    area: Rect,
    settings: &crate::settings::Settings,
) -> Option<Action> {
    let painter = ui.painter();
    let mut action = None;
    let (left, right, top) = (area.left(), area.right(), area.top());
    painter.text(
        Pos2::new(left, top + 4.0),
        egui::Align2::LEFT_TOP,
        s("Мова", "Язык", "Language"),
        regular(11.0),
        SECONDARY,
    );
    let row = Rect::from_min_max(Pos2::new(left, top + 22.0), Pos2::new(right, top + 52.0));
    painter.rect_filled(row, 9.0, Color32::from_black_alpha(65));
    let width = row.width() / 3.0;
    for (i, lang) in crate::i18n::Lang::ALL.into_iter().enumerate() {
        let cell = Rect::from_min_size(
            row.min + Vec2::new(i as f32 * width, 0.0),
            Vec2::new(width, row.height()),
        )
        .shrink(3.0);
        let chosen = settings.language == lang;
        let response = ui
            .interact(cell, ui.id().with(("language", i)), egui::Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, chosen, lang.label())
        });
        if response.clicked() && !chosen {
            action = Some(Action::Language(lang));
        }
        if chosen {
            gradient(
                painter,
                cell,
                7.0,
                Color32::from_white_alpha(38),
                Color32::from_white_alpha(18),
            );
        } else if response.hovered() {
            painter.rect_filled(cell, 7.0, Color32::from_white_alpha(8));
        }
        painter.text(
            cell.center(),
            egui::Align2::CENTER_CENTER,
            lang.label(),
            strong(12.0),
            if chosen { LABEL } else { SECONDARY },
        );
    }
    painter.text(
        Pos2::new(left, top + 70.0),
        egui::Align2::LEFT_TOP,
        s(
            "Звук, коли агент закінчив",
            "Звук, когда агент закончил",
            "Sound when an agent finishes",
        ),
        regular(12.0),
        LABEL,
    );
    let switch = Rect::from_min_size(Pos2::new(right - 40.0, top + 67.0), Vec2::new(40.0, 22.0));
    let response = ui
        .interact(switch, ui.id().with("sound"), egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Checkbox,
            true,
            settings.sound,
            s("Звук", "Звук", "Sound"),
        )
    });
    if response.clicked() {
        action = Some(Action::Sound(!settings.sound));
    }
    let on = ui
        .ctx()
        .animate_bool_with_time(response.id, settings.sound, 0.15);
    painter.rect_filled(
        switch,
        11.0,
        mix(Color32::from_white_alpha(30), tint(0), on),
    );
    painter.circle_filled(
        Pos2::new(switch.left() + 11.0 + on * 18.0, switch.center().y),
        8.0,
        LABEL,
    );
    let preview = Rect::from_min_size(Pos2::new(left, top + 96.0), Vec2::new(160.0, 20.0));
    let response = ui
        .interact(preview, ui.id().with("preview"), egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() {
        action = Some(Action::Preview);
    }
    painter.text(
        preview.left_center(),
        egui::Align2::LEFT_CENTER,
        s("▶ Прослухати", "▶ Прослушать", "▶ Play it"),
        regular(11.0),
        if response.hovered() { LABEL } else { SECONDARY },
    );
    painter.text(
        Pos2::new(right, area.bottom() - 2.0),
        egui::Align2::RIGHT_BOTTOM,
        format!("Void {}", env!("CARGO_PKG_VERSION")),
        regular(10.0),
        GRAY,
    );
    action
}

/// Starting a task from the sleeping cat: which agent, which project, and what to do.
#[derive(Default)]
pub struct Compose {
    pub open: bool,
    pub agent: usize,
    /// The chosen folder per agent; None means its newest project.
    pub project: [Option<std::path::PathBuf>; 2],
    pub text: String,
    /// The last result shown under the field: the message and whether it worked.
    pub note: Option<(String, bool)>,
    /// Put the cursor in the field on the next frame.
    pub focus: bool,
}

pub fn compose_size() -> Vec2 {
    Vec2::new(WIDTH, 240.0)
}

/// Window size for the composer above the cat.
pub fn compose_window(size: Vec2) -> Vec2 {
    Vec2::new(size.x + MARGIN * 2.0, 88.0 + size.y + MARGIN + 6.0)
}

/// Draws the composer with its bottom edge just above the cat. Returns its rectangle, whether
/// the text field has the keyboard, and the folder to start the task in when the user sends it.
pub fn compose(
    ui: &mut egui::Ui,
    bottom_right: Pos2,
    state: &mut Compose,
    projects: &[Vec<crate::projects::Project>; 2],
) -> (Rect, bool, Option<std::path::PathBuf>) {
    let size = compose_size();
    let rect = Rect::from_min_max(bottom_right - size, bottom_right);
    let color = tint(state.agent);
    {
        let painter = ui.painter();
        painter.add(
            Shadow {
                offset: [0, 10],
                blur: 24,
                spread: 0,
                color: Color32::from_black_alpha(110),
            }
            .as_shape(rect, RADIUS),
        );
        gradient(painter, rect, RADIUS, GLASS_TOP, GLASS_BOTTOM);
        painter.rect_stroke(
            rect,
            RADIUS,
            Stroke::new(1.0, Color32::from_white_alpha(28)),
            StrokeKind::Inside,
        );
        painter.line_segment(
            [
                Pos2::new(rect.left() + 24.0, rect.top() + 0.5),
                Pos2::new(rect.right() - 24.0, rect.top() + 0.5),
            ],
            Stroke::new(1.5, color.gamma_multiply(0.7)),
        );
    }
    let inner = rect.shrink2(Vec2::new(PAD + 4.0, PAD));
    let mut send = None;
    let mut typing = false;
    ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
        let visuals = ui.visuals_mut();
        visuals.extreme_bg_color = Color32::from_black_alpha(90);
        visuals.widgets.inactive.corner_radius = 8.into();
        visuals.widgets.hovered.corner_radius = 8.into();
        visuals.widgets.active.corner_radius = 8.into();
        visuals.selection.bg_fill = color.gamma_multiply(0.45);
        ui.spacing_mut().item_spacing = Vec2::new(8.0, 10.0);

        ui.label(
            egui::RichText::new(s("Нове завдання", "Новая задача", "New task"))
                .font(strong(15.5))
                .color(LABEL),
        );
        // Which agent.
        ui.horizontal(|ui| {
            for agent in 0..2 {
                let name = ["Claude", "Codex"][agent];
                let chosen = state.agent == agent;
                let text = egui::RichText::new(name)
                    .font(strong(12.5))
                    .color(if chosen { LABEL } else { SECONDARY });
                if ui
                    .add_sized([100.0, 28.0], egui::Button::selectable(chosen, text))
                    .clicked()
                {
                    state.agent = agent;
                    state.note = None;
                }
            }
        });
        // Which project: the agent's newest by default.
        let list = &projects[state.agent];
        let chosen = state.project[state.agent]
            .clone()
            .filter(|path| list.iter().any(|project| &project.path == path))
            .or_else(|| list.first().map(|project| project.path.clone()));
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(s("Проєкт", "Проект", "Project"))
                    .font(regular(11.5))
                    .color(SECONDARY),
            );
            let name = chosen
                .as_ref()
                .and_then(|path| list.iter().find(|project| &project.path == path))
                .map_or_else(
                    || s("немає проєктів", "нет проектов", "no projects").to_owned(),
                    |project| project.name.clone(),
                );
            let mut picked = chosen.clone();
            egui::ComboBox::from_id_salt(("project", state.agent))
                .width(ui.available_width())
                .selected_text(egui::RichText::new(name).font(strong(12.0)).color(LABEL))
                .show_ui(ui, |ui| {
                    for project in list {
                        ui.selectable_value(&mut picked, Some(project.path.clone()), &project.name)
                            .on_hover_text(project.path.display().to_string());
                    }
                });
            if picked != chosen {
                state.project[state.agent] = picked;
            }
        });
        // What to do. Enter sends; Shift+Enter makes a new line.
        let field = egui::TextEdit::multiline(&mut state.text)
            .hint_text(s("Що зробити?", "Что сделать?", "What should it do?"))
            .font(regular(13.0))
            .desired_rows(4)
            .desired_width(f32::INFINITY)
            .return_key(Some(egui::KeyboardShortcut::new(
                egui::Modifiers::SHIFT,
                egui::Key::Enter,
            )));
        let field = ui.add(field);
        if state.focus {
            field.request_focus();
            state.focus = false;
        }
        typing = field.has_focus();
        let enter = typing
            && ui.input(|input| input.key_pressed(egui::Key::Enter) && !input.modifiers.shift);
        ui.horizontal(|ui| {
            if let Some((message, ok)) = &state.note {
                ui.label(
                    egui::RichText::new(message)
                        .font(regular(11.0))
                        .color(if *ok { READY } else { STOP }),
                );
            } else {
                ui.label(
                    egui::RichText::new(s(
                        "Enter — запустити",
                        "Enter — запустить",
                        "Enter to start",
                    ))
                    .font(regular(10.5))
                    .color(GRAY),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let start = egui::Button::new(
                    egui::RichText::new(s("Запустити", "Запустить", "Start"))
                        .font(strong(12.5))
                        .color(Color32::from_gray(20)),
                )
                .fill(color)
                .min_size(Vec2::new(96.0, 28.0));
                if ui.add(start).clicked() || enter {
                    send = Some(chosen.clone());
                }
            });
        });
    });
    // No project to start in: say so instead of sending.
    let send = match send {
        Some(Some(path)) => Some(path),
        Some(None) => {
            state.note = Some((
                s(
                    "Немає проєкту для запуску",
                    "Нет проекта для запуска",
                    "No project to start in",
                )
                .to_owned(),
                false,
            ));
            None
        }
        None => None,
    };
    (rect, typing, send)
}

/// Three little "z"s drifting up from the cat's head while it sleeps. Each rises, grows and
/// fades, one after another. `time` is None under reduced motion: they stay put, faded.
pub fn sleep_zs(painter: &egui::Painter, cat: Rect, time: Option<f64>) {
    const CYCLE: f32 = 2.6;
    let head = Pos2::new(cat.center().x + 14.0, cat.top() + 14.0);
    for i in 0..3 {
        let phase = i as f32 / 3.0;
        let (rise, alpha) = match time {
            Some(t) => {
                let p = ((t as f32 / CYCLE) + phase).rem_euclid(1.0);
                // Fade in over the first fifth of the rise, out over the last third.
                let fade = (p / 0.2).min(1.0) * ((1.0 - p) / 0.35).min(1.0);
                (p, fade)
            }
            None => (0.5, 0.6),
        };
        let size = 12.0 + 6.0 * rise;
        let at = Pos2::new(
            head.x + 10.0 * i as f32 + 6.0 * (rise * std::f32::consts::TAU).sin(),
            head.y - 6.0 - 26.0 * rise - 10.0 * i as f32,
        );
        painter.text(
            at,
            egui::Align2::CENTER_CENTER,
            "z",
            strong(size),
            Color32::from_white_alpha((alpha * 255.0) as u8),
        );
    }
}

/// Height above the cat taken by the thought bubble while an agent works.
pub const STRIP: f32 = 44.0;
const SEGMENT: f32 = 48.0;
const PUFF: f32 = 24.0;
const BUSY_WIDTH: f32 = 236.0;
const BUSY_ROWS: usize = 3;

/// Soft identity tints, not brand marks: warm for Claude, cool for Codex.
pub fn tint(agent: usize) -> Color32 {
    if agent == 0 {
        Color32::from_rgb(244, 146, 108)
    } else {
        Color32::from_rgb(118, 196, 255)
    }
}

fn glyph(agent: usize) -> &'static str {
    // Generic symbols: a spark for Claude, a prompt chevron for Codex.
    if agent == 0 { "✳" } else { "❯" }
}

/// One segment per busy agent inside a single thought bubble above the cat's head.
pub fn bubble_segments(cat: Rect, count: usize) -> Vec<Rect> {
    let width = count as f32 * SEGMENT;
    // Over the head, but never past the window's right edge (the halo needs ~10 px).
    let left = (cat.center().x + 6.0 - width / 2.0).min(cat.right() - 12.0 - width);
    (0..count)
        .map(|i| {
            Rect::from_min_size(
                Pos2::new(left + i as f32 * SEGMENT, cat.top() - STRIP + 12.0),
                Vec2::new(SEGMENT, PUFF),
            )
        })
        .collect()
}

pub struct Puff {
    pub agent: usize,
    pub sessions: usize,
    /// 0..1, eased hover highlight of this segment.
    pub hover: f32,
}

/// The cat "thinks" about whoever is working: a glass bubble with a trail of little puffs
/// from its head, each agent a glowing glyph with dots that bounce while it works.
/// `appear` (0..1) pops the bubble in; `time` is None under reduced motion.
pub fn thought_bubble(
    painter: &egui::Painter,
    cat: Rect,
    segments: &[Rect],
    puffs: &[Puff],
    appear: f32,
    time: Option<f64>,
) {
    let (Some(first), Some(last)) = (segments.first(), segments.last()) else {
        return;
    };
    // Ease-out-back: a small overshoot makes the pop feel alive.
    let pop = {
        let t = appear.clamp(0.0, 1.0) - 1.0;
        1.0 + 2.2 * t * t * t + 1.2 * t * t
    };
    let alpha = appear.clamp(0.0, 1.0);
    let lift = puffs.iter().map(|p| p.hover).fold(0.0, f32::max);
    let scale = pop * (1.0 + 0.06 * lift);
    let center = first.union(*last).center();
    let at = |p: Pos2| center + (p - center) * scale;
    let bubble = Rect::from_two_pos(at(first.min), at(last.max));
    let radius = bubble.height() / 2.0;
    let t = time.unwrap_or(0.0) as f32;
    let fade = |c: Color32| c.gamma_multiply(alpha);

    // Trail from the head: two puffs that drift a little, like breath.
    let head = Pos2::new(cat.left() + cat.width() * 0.42, cat.top() + 12.0);
    let drift = |phase: f32| (t * 1.6 + phase).sin() * 1.2;
    for (k, (offset, size)) in [(0.35_f32, 2.6_f32), (0.7, 4.0)].into_iter().enumerate() {
        let p = head.lerp(bubble.left_bottom() + Vec2::new(radius, 0.0), offset)
            + Vec2::new(drift(k as f32), 0.0);
        painter.circle_filled(p, size * pop.max(0.0), fade(GLASS_TOP));
        painter.circle_stroke(
            p,
            size * pop.max(0.0),
            Stroke::new(1.0, fade(Color32::from_white_alpha(30))),
        );
    }

    // A breathing halo in the working agents' colors.
    let breath = 0.5 + 0.5 * (t * 2.0).sin();
    for (i, puff) in puffs.iter().enumerate() {
        let seg = Rect::from_two_pos(at(segments[i].min), at(segments[i].max));
        for ring in 1..=2 {
            let grow = ring as f32 * (2.0 + breath + puff.hover * 1.5);
            painter.rect_filled(
                seg.expand(grow),
                radius + grow,
                fade(tint(puff.agent).gamma_multiply(0.05 + 0.04 * puff.hover)),
            );
        }
    }
    painter.add(
        Shadow {
            offset: [0, 4],
            blur: 12,
            spread: 0,
            color: Color32::from_black_alpha((110.0 * alpha) as u8),
        }
        .as_shape(bubble, radius),
    );
    gradient(painter, bubble, radius, fade(GLASS_TOP), fade(GLASS_BOTTOM));
    painter.rect_stroke(
        bubble,
        radius,
        Stroke::new(1.0, fade(Color32::from_white_alpha(34))),
        StrokeKind::Inside,
    );

    for (i, puff) in puffs.iter().enumerate() {
        let seg = Rect::from_two_pos(at(segments[i].min), at(segments[i].max)).shrink(2.0);
        let color = tint(puff.agent);
        if puff.hover > 0.01 {
            painter.rect_filled(
                seg,
                seg.height() / 2.0,
                fade(color.gamma_multiply(0.22 * puff.hover)),
            );
        }
        if i > 0 {
            let x = seg.left() - 2.0;
            painter.line_segment(
                [
                    Pos2::new(x, seg.top() + 6.0),
                    Pos2::new(x, seg.bottom() - 6.0),
                ],
                Stroke::new(1.0, fade(Color32::from_white_alpha(22))),
            );
        }
        let glyph_at = Pos2::new(seg.left() + 13.0 * scale, seg.center().y);
        painter.text(
            glyph_at,
            egui::Align2::CENTER_CENTER,
            glyph(puff.agent),
            // Pop-in starts at scale 0, and fonts need a positive size.
            strong((12.5 * scale).max(1.0)),
            fade(color),
        );
        // Three dots bounce in a wave; faster and higher under the pointer.
        let speed = 6.0 + 4.0 * puff.hover;
        let jump = 2.5 + 1.5 * puff.hover;
        for k in 0..3 {
            let bounce = time.map_or(0.0, |_| (t * speed - k as f32 * 0.9).sin().max(0.0) * jump);
            let p = Pos2::new(
                seg.left() + (26.0 + k as f32 * 6.0) * scale,
                seg.center().y - bounce,
            );
            painter.circle_filled(
                p,
                1.9 * scale,
                fade(color.gamma_multiply(0.55 + 0.45 * (bounce / jump))),
            );
        }
        if puff.sessions > 1 {
            let badge = Pos2::new(seg.right() - 2.0, seg.top() + 1.0);
            painter.circle_filled(badge, 6.5, fade(color));
            painter.text(
                badge,
                egui::Align2::CENTER_CENTER,
                puff.sessions.min(9).to_string(),
                strong(9.0),
                fade(Color32::from_gray(18)),
            );
        }
    }
}

fn sessions_word(count: usize) -> &'static str {
    let (one, few, many) = match crate::i18n::lang() {
        crate::i18n::Lang::Uk => ("сесія", "сесії", "сесій"),
        crate::i18n::Lang::Ru => ("сессия", "сессии", "сессий"),
        crate::i18n::Lang::En => ("session", "sessions", "sessions"),
    };
    match (count % 10, count % 100) {
        _ if crate::i18n::lang() == crate::i18n::Lang::En && count != 1 => few,
        (1, n) if n != 11 => one,
        (2..=4, n) if !(12..=14).contains(&n) => few,
        _ => many,
    }
}

fn elapsed(since: u64, now: u64) -> String {
    let minutes = now.saturating_sub(since) / 60;
    if minutes == 0 {
        s("щойно", "только что", "just now").into()
    } else {
        crate::duration(minutes as f64)
    }
}

/// `mcp__server__tool` reads as `tool`.
fn tool_name(tool: &str) -> &str {
    tool.rsplit("__").next().unwrap_or(tool)
}

const SESSION_ROW: f32 = 40.0;

/// Height of the hover card: a header, one line per session, and the quota line.
pub fn busy_card_size(sessions: &[crate::activity::Session]) -> Vec2 {
    let rows = sessions.len().min(BUSY_ROWS) as f32;
    let more = if sessions.len() > BUSY_ROWS {
        18.0
    } else {
        0.0
    };
    Vec2::new(BUSY_WIDTH, 40.0 + rows * SESSION_ROW + more + 34.0)
}

/// One line per session of work: the project and how long it has run, then the step the agent
/// is on. The quota sits underneath. The menu holds the details; the card stays quiet.
pub fn busy_card(
    painter: &egui::Painter,
    rect: Rect,
    agent: usize,
    sessions: &[crate::activity::Session],
    quotas: &[Quota],
    now: u64,
    time: Option<f64>,
) {
    let name = ["Claude", "Codex"][agent];
    let color = tint(agent);
    painter.add(
        Shadow {
            offset: [0, 8],
            blur: 20,
            spread: 0,
            color: Color32::from_black_alpha(110),
        }
        .as_shape(rect, 16.0),
    );
    gradient(painter, rect, 16.0, GLASS_TOP, GLASS_BOTTOM);
    painter.rect_stroke(
        rect,
        16.0,
        Stroke::new(1.0, Color32::from_white_alpha(28)),
        StrokeKind::Inside,
    );
    // A thin line in the agent's color along the top edge ties the card to its bubble.
    painter.line_segment(
        [
            Pos2::new(rect.left() + 18.0, rect.top() + 0.5),
            Pos2::new(rect.right() - 18.0, rect.top() + 0.5),
        ],
        Stroke::new(1.5, color.gamma_multiply(0.7)),
    );
    let (left, right) = (rect.left() + 16.0, rect.right() - 16.0);
    let mut y = rect.top() + 14.0;
    // A live dot, pulsing like the bubble.
    let pulse = time.map_or(1.0, |t| 0.6 + 0.4 * (t as f32 * 3.0).sin().abs());
    let dot = Pos2::new(left + 4.0, y + 9.0);
    painter.circle_filled(dot, 6.0 * pulse, color.gamma_multiply(0.18));
    painter.circle_filled(dot, 3.5, color);
    painter.text(
        Pos2::new(left + 14.0, y),
        egui::Align2::LEFT_TOP,
        t!("{name} працює", "{name} работает", "{name} is working"),
        strong(13.5),
        LABEL,
    );
    if sessions.len() > 1 {
        painter.text(
            Pos2::new(right, y + 2.0),
            egui::Align2::RIGHT_TOP,
            format!("{} {}", sessions.len(), sessions_word(sessions.len())),
            regular(10.5),
            SECONDARY,
        );
    }
    y += 30.0;
    for session in sessions.iter().take(BUSY_ROWS) {
        let project = if session.project.is_empty() {
            s("Без назви", "Без названия", "Untitled")
        } else {
            &session.project
        };
        let since = painter.layout_no_wrap(elapsed(session.since, now), regular(11.0), SECONDARY);
        let room = right - left - since.size().x - 10.0;
        let title = painter.layout(project.to_owned(), strong(12.0), LABEL, room);
        // One line only: long folder names are clipped by the width, not wrapped.
        let title_row = title.rows.first().map_or(0.0, |row| row.height());
        painter
            .with_clip_rect(Rect::from_min_size(
                Pos2::new(left, y),
                Vec2::new(room, title_row),
            ))
            .galley(Pos2::new(left, y), title, LABEL);
        painter.galley(Pos2::new(right - since.size().x, y + 1.0), since, SECONDARY);
        let step = session.tool.as_deref().map_or_else(
            || s("думає", "думает", "thinking").to_owned(),
            |tool| {
                let tool = tool_name(tool);
                t!("виконує {tool}", "выполняет {tool}", "running {tool}")
            },
        );
        painter.text(
            Pos2::new(left, y + 18.0),
            egui::Align2::LEFT_TOP,
            step,
            regular(10.5),
            SECONDARY,
        );
        y += SESSION_ROW;
    }
    if sessions.len() > BUSY_ROWS {
        painter.text(
            Pos2::new(left, y - 2.0),
            egui::Align2::LEFT_TOP,
            t!(
                "і ще {}",
                "и ещё {}",
                "and {} more",
                sessions.len() - BUSY_ROWS
            ),
            regular(10.5),
            GRAY,
        );
        y += 18.0;
    }
    painter.line_segment(
        [Pos2::new(left, y + 2.0), Pos2::new(right, y + 2.0)],
        Stroke::new(1.0, Color32::from_white_alpha(20)),
    );
    // Both limits as numbers only: the menu explains what they mean and when they reset.
    let quota = quotas
        .iter()
        .map(|quota| match quota.remaining {
            Some(share) => format!("{} {:.0}%", quota.name, share * 100.0),
            None => format!("{} —", quota.name),
        })
        .collect::<Vec<_>>()
        .join("   ·   ");
    painter.text(
        Pos2::new(left, y + 10.0),
        egui::Align2::LEFT_TOP,
        quota,
        regular(10.5),
        SECONDARY,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn busy_card_words_read_naturally() {
        assert_eq!(sessions_word(1), "сесія");
        assert_eq!(sessions_word(3), "сесії");
        assert_eq!(sessions_word(5), "сесій");
        assert_eq!(sessions_word(12), "сесій");
        assert_eq!(sessions_word(21), "сесія");
        assert_eq!(elapsed(1000, 1030), "щойно");
        assert_eq!(elapsed(0, 65 * 60), "1 год 05 хв");
        assert_eq!(tool_name("mcp__github__create_issue"), "create_issue");
        let session = || crate::activity::Session {
            id: Default::default(),
            project: "p".into(),
            since: 0,
            tool: None,
            busy: true,
            end: 0,
            prompt: None,
            tokens: 0,
        };
        let five: Vec<_> = (0..5).map(|_| session()).collect();
        let three: Vec<_> = (0..3).map(|_| session()).collect();
        assert_eq!(busy_card_size(&five).y, busy_card_size(&three).y + 18.0);
        assert_eq!(busy_card_size(&three).y, 40.0 + 3.0 * SESSION_ROW + 34.0);
        let now = 1_800_000_000;
        let windows = quotas(&crate::demo_codex_history(now), now, 0);
        assert_eq!(
            windows.iter().map(|q| q.name).collect::<Vec<_>>(),
            ["5 год", "7 днів"]
        );
    }
    fn limits(five: f64, week: f64, now: u64) -> Vec<Record> {
        vec![
            serde_json::from_value(serde_json::json!({
                "time": now, "session_id": "s",
                "rate_limits": {
                    "five_hour": { "used_percentage": five, "resets_at": now + 3600 },
                    "seven_day": { "used_percentage": week, "resets_at": now + 86_400 }
                }
            }))
            .unwrap(),
        ]
    }

    #[test]
    fn menu_rows_say_whether_an_agent_can_be_used_and_where_each_limit_stands() {
        let now = 1_800_000_000;
        let ready = limits(40.0, 26.0, now);
        let ready = row(
            1,
            &advice::assess(&ready, now),
            &ready,
            None,
            [Some(1_250_000), Some(9_800_000)],
            now,
            0,
        );
        assert_eq!(ready.verdict, Verdict::Ready);
        assert_eq!(ready.status, "Можна працювати");
        assert_eq!(
            ready.windows[0].left.map(|l| (l * 100.0).round()),
            Some(60.0)
        );
        assert_eq!(
            ready.windows[1].left.map(|l| (l * 100.0).round()),
            Some(74.0)
        );
        assert_eq!(ready.windows[0].spent.as_deref(), Some("витрачено 1,2 млн"));
        assert_eq!(ready.windows[1].spent.as_deref(), Some("витрачено 9,8 млн"));

        let low = limits(92.0, 30.0, now);
        let low = row(
            0,
            &advice::assess(&low, now),
            &low,
            None,
            [None, None],
            now,
            0,
        );
        assert_eq!((low.verdict, low.status.as_str()), (Verdict::Low, "Мало"));

        let claude = vec![crate::demo_claude_snapshot(now)];
        let paused = row(
            0,
            &advice::assess(&claude, now),
            &claude,
            None,
            [None, None],
            now,
            0,
        );
        assert_eq!(paused.verdict, Verdict::Paused);
        assert!(paused.status.starts_with("Пауза до "), "{}", paused.status);
        assert!(paused.until.is_some());

        let none: Vec<Record> = Vec::new();
        let unknown = row(
            1,
            &advice::assess(&none, now),
            &none,
            Some("Немає зв’язку"),
            [None, None],
            now,
            0,
        );
        assert_eq!(
            (unknown.verdict, unknown.status.as_str()),
            (Verdict::Unknown, "Немає зв’язку")
        );
        assert_eq!(short_tokens(640_400), "640 тис.");
        assert_eq!(short_tokens(950), "950");

        // The header agrees with the rows.
        let (title, detail) = summary(&[paused, ready]);
        assert!(title.starts_with("Claude на паузі до "), "{title}");
        assert_eq!(detail, "Працюй у Codex");
        let both = [
            row(
                0,
                &advice::assess(&limits(10.0, 10.0, now), now),
                &limits(10.0, 10.0, now),
                None,
                [None, None],
                now,
                0,
            ),
            row(
                1,
                &advice::assess(&limits(80.0, 10.0, now), now),
                &limits(80.0, 10.0, now),
                None,
                [None, None],
                now,
                0,
            ),
        ];
        let (title, detail) = summary(&both);
        assert_eq!(title, "Обидва готові до роботи");
        assert_eq!(detail, "Більше запасу в Claude");
    }
}
