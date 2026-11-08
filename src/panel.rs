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
fn summary(rows: &[Row]) -> (String, String) {
    let name = |row: &Row| crate::launch::APPS[row.agent];
    let room = |row: &Row| {
        row.windows
            .iter()
            .filter_map(|window| window.left)
            .fold(f32::INFINITY, f32::min)
    };
    if rows.is_empty() {
        return (
            s(
                "Агентів не знайдено",
                "Агенты не найдены",
                "No agents found",
            )
            .to_owned(),
            s(
                "Встанови Claude, Codex або Kimi Code",
                "Установи Claude, Codex или Kimi Code",
                "Install Claude, Codex or Kimi Code",
            )
            .to_owned(),
        );
    }
    // What to say about the other agents when this one is the subject: the most usable of them.
    let about = |subject: &Row| {
        let rank = |row: &Row| match row.verdict {
            Verdict::Ready => 3,
            Verdict::Low => 2,
            Verdict::Unknown => 1,
            Verdict::Paused => 0,
        };
        let Some(other) = rows
            .iter()
            .filter(|row| row.agent != subject.agent)
            .max_by_key(|row| rank(row))
        else {
            return String::new();
        };
        let n = name(other);
        match other.verdict {
            Verdict::Ready => t!("Працюй у {n}", "Работай в {n}", "Work in {n}"),
            Verdict::Low => t!(
                "{n}: лишилось мало",
                "{n}: осталось мало",
                "{n}: running low"
            ),
            Verdict::Paused => {
                let at = other.until.clone().unwrap_or_default();
                t!(
                    "{n}: пауза до {at}",
                    "{n}: пауза до {at}",
                    "{n}: paused until {at}"
                )
            }
            Verdict::Unknown => t!("{n}: немає даних", "{n}: нет данных", "{n}: no data"),
        }
    };
    let all = |verdict: Verdict| rows.iter().all(|row| row.verdict == verdict);
    let every = |two: &'static str, more: &'static str| if rows.len() == 2 { two } else { more };
    if all(Verdict::Unknown) {
        let names = rows.iter().map(name).collect::<Vec<_>>().join(", ");
        return (
            s(
                "Поки нічого не бачу",
                "Пока ничего не вижу",
                "Nothing to see yet",
            )
            .to_owned(),
            t!(
                "Попрацюй у {names}",
                "Поработай в {names}",
                "Work in {names}"
            ),
        );
    }
    if rows.len() > 1 && all(Verdict::Paused) {
        let until = rows
            .iter()
            .map(|row| {
                let (n, at) = (name(row), row.until.clone().unwrap_or_default());
                t!("{n} до {at}", "{n} до {at}", "{n} until {at}")
            })
            .collect::<Vec<_>>()
            .join(" · ");
        let title = every(
            s("Обидва на паузі", "Оба на паузе", "Both are paused"),
            s("Усі на паузі", "Все на паузе", "All are paused"),
        );
        return (title.to_owned(), until);
    }
    if rows.len() > 1 && all(Verdict::Low) {
        let title = every(
            s(
                "Обидва майже вичерпано",
                "Оба почти исчерпаны",
                "Both are almost out",
            ),
            s(
                "Усі майже вичерпано",
                "Все почти исчерпаны",
                "All are almost out",
            ),
        );
        return (title.to_owned(), String::new());
    }
    if rows.len() > 1 && all(Verdict::Ready) {
        let (most, least) = rows
            .iter()
            .fold((f32::MIN, f32::MAX), |(most, least), row| {
                (most.max(room(row)), least.min(room(row)))
            });
        let detail = if most - least >= 0.15 {
            let best = rows
                .iter()
                .max_by(|a, b| room(a).total_cmp(&room(b)))
                .map(name)
                .unwrap_or_default();
            t!(
                "Більше запасу в {best}",
                "Больше запаса в {best}",
                "More room in {best}"
            )
        } else {
            String::new()
        };
        let title = every(
            s(
                "Обидва готові до роботи",
                "Оба готовы к работе",
                "Both are ready to work",
            ),
            s(
                "Усі готові до роботи",
                "Все готовы к работе",
                "All are ready to work",
            ),
        );
        return (title.to_owned(), detail);
    }
    if let Some(paused) = rows.iter().find(|row| row.verdict == Verdict::Paused) {
        let (n, at) = (name(paused), paused.until.clone().unwrap_or_default());
        return (
            t!(
                "{n} на паузі до {at}",
                "{n} на паузе до {at}",
                "{n} paused until {at}"
            ),
            about(paused),
        );
    }
    if let Some(low) = rows.iter().find(|row| row.verdict == Verdict::Low) {
        let n = name(low);
        return (
            t!(
                "У {n} лишилось мало",
                "У {n} осталось мало",
                "{n} is running low"
            ),
            about(low),
        );
    }
    // At least one is ready and none is low or paused: name the one with the most room.
    let ready = rows
        .iter()
        .filter(|row| row.verdict == Verdict::Ready)
        .max_by(|a, b| room(a).total_cmp(&room(b)))
        .unwrap_or(&rows[0]);
    let n = name(ready);
    (
        t!(
            "{n} готовий до роботи",
            "{n} готов к работе",
            "{n} is ready to work"
        ),
        about(ready),
    )
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
    /// The agents installed on this machine.
    pub rows: Vec<Row>,
    pub size: Vec2,
}

pub fn menu(rows: Vec<Row>) -> Menu {
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

/// The glass surface every Void window stands on. `accent` draws the agent's thin top line.
fn glass(painter: &egui::Painter, rect: Rect, accent: Option<Color32>) {
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
    if let Some(color) = accent {
        painter.line_segment(
            [
                Pos2::new(rect.left() + RADIUS, rect.top() + 0.5),
                Pos2::new(rect.right() - RADIUS, rect.top() + 0.5),
            ],
            Stroke::new(1.5, color.gamma_multiply(0.7)),
        );
    }
}

/// The header every window opens with: a title and one quieter line under it.
fn header(painter: &egui::Painter, left: f32, top: f32, width: f32, title: String, detail: String) {
    let title = painter.layout(title, strong(15.5), LABEL, width);
    painter.galley(Pos2::new(left, top), title, LABEL);
    let detail = painter.layout(detail, regular(11.5), SECONDARY, width);
    painter.galley(Pos2::new(left, top + 24.0), detail, SECONDARY);
}

/// The one switch used everywhere (agent tabs, language, agent choice): a dark track with the
/// chosen cell lifted, each option with an optional colored dot. Returns the option clicked.
fn segmented(
    ui: &egui::Ui,
    track: Rect,
    id: &str,
    options: &[(&str, Option<Color32>)],
    chosen: usize,
) -> Option<usize> {
    let painter = ui.painter();
    painter.rect_filled(track, 10.0, Color32::from_black_alpha(65));
    let width = track.width() / options.len() as f32;
    let mut clicked = None;
    for (i, (label, dot)) in options.iter().enumerate() {
        let cell = Rect::from_min_size(
            track.min + Vec2::new(i as f32 * width, 0.0),
            Vec2::new(width, track.height()),
        )
        .shrink(3.0);
        let response = ui
            .interact(cell, ui.id().with((id, i)), egui::Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, chosen == i, *label)
        });
        if response.clicked() && chosen != i {
            clicked = Some(i);
        }
        if chosen == i {
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
        let text = painter.layout_no_wrap((*label).to_owned(), strong(12.5), LABEL);
        let lead = if dot.is_some() { 12.0 } else { 0.0 };
        let start = cell.center().x - (text.size().x + lead) / 2.0;
        if let Some(color) = dot {
            painter.circle_filled(Pos2::new(start + 3.0, cell.center().y), 3.5, *color);
        }
        painter.galley(
            Pos2::new(start + lead, cell.center().y - text.size().y / 2.0),
            text,
            if chosen == i { LABEL } else { SECONDARY },
        );
    }
    clicked
}

/// The square icon button in a window's header: the gear, the close cross.
fn icon_button(ui: &egui::Ui, rect: Rect, id: &str, icon: &str, active: bool, label: &str) -> bool {
    let painter = ui.painter();
    let response = ui
        .interact(rect, ui.id().with(id), egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    response
        .widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, active, label));
    let lit = active || response.hovered();
    if lit {
        painter.rect_filled(
            rect,
            9.0,
            Color32::from_white_alpha(if active { 34 } else { 14 }),
        );
    }
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icon,
        regular(15.0),
        if lit { LABEL } else { SECONDARY },
    );
    response.clicked()
}

/// The one filled button: in the agent's color, dark text, lighter under the pointer.
fn primary_button(ui: &egui::Ui, rect: Rect, id: &str, label: &str, color: Color32) -> bool {
    let painter = ui.painter();
    let response = ui
        .interact(rect, ui.id().with(id), egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let fill = if response.hovered() {
        mix(color, Color32::WHITE, 0.18)
    } else {
        color
    };
    painter.rect_filled(rect, 10.0, fill);
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        strong(12.5),
        Color32::from_gray(20),
    );
    response.clicked()
}

/// A field: the same dark track as the switch, so inputs read as one family.
fn field(painter: &egui::Painter, rect: Rect, focused: bool) {
    painter.rect_filled(rect, 10.0, Color32::from_black_alpha(65));
    if focused {
        painter.rect_stroke(
            rect,
            10.0,
            Stroke::new(1.0, Color32::from_white_alpha(46)),
            StrokeKind::Inside,
        );
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
    glass(painter, rect, None);
    let (left, right) = (rect.left() + PAD + 4.0, rect.right() - PAD - 4.0);
    let top = rect.top() + PAD;

    // Header: the verdict, or the settings title, and the gear.
    let gear = Rect::from_min_size(Pos2::new(right - 30.0, top - 4.0), Vec2::splat(30.0));
    if icon_button(
        ui,
        gear,
        "settings",
        if *settings_open { "✕" } else { "⚙" },
        *settings_open,
        s("Налаштування", "Настройки", "Settings"),
    ) {
        *settings_open = !*settings_open;
    }
    let (title, detail) = if *settings_open {
        (
            s("Налаштування", "Настройки", "Settings").to_owned(),
            String::new(),
        )
    } else {
        (menu.title.clone(), menu.detail.clone())
    };
    header(painter, left, top, right - left - 40.0, title, detail);

    let area = Rect::from_min_max(
        Pos2::new(left, top + HEADER - 8.0),
        Pos2::new(right, rect.bottom() - PAD),
    );
    let action = if *settings_open {
        settings_body(ui, area, settings)
    } else if menu.rows.is_empty() {
        None
    } else {
        // Tabs: one per installed agent, each with a dot in its status color.
        let mut chosen = menu
            .rows
            .iter()
            .position(|row| row.agent == *tab)
            .unwrap_or(0);
        let tabs = Rect::from_min_size(
            Pos2::new(left, area.top() + 4.0),
            Vec2::new(right - left, 32.0),
        );
        let options: Vec<(&str, Option<Color32>)> = menu
            .rows
            .iter()
            .map(|row| {
                (
                    crate::launch::APPS[row.agent],
                    Some(verdict_color(row.verdict)),
                )
            })
            .collect();
        if let Some(i) = segmented(ui, tabs, "agent-tab", &options, chosen) {
            chosen = i;
            *tab = menu.rows[i].agent;
        }
        // The chosen agent: its status, then its two limits side by side.
        let row = &menu.rows[chosen];
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
    let row = Rect::from_min_max(Pos2::new(left, top + 22.0), Pos2::new(right, top + 54.0));
    let languages: Vec<(&str, Option<Color32>)> = crate::i18n::Lang::ALL
        .iter()
        .map(|lang| (lang.label(), None))
        .collect();
    let current = crate::i18n::Lang::ALL
        .iter()
        .position(|lang| *lang == settings.language)
        .unwrap_or(0);
    if let Some(i) = segmented(ui, row, "language", &languages, current) {
        action = Some(Action::Language(crate::i18n::Lang::ALL[i]));
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
    pub project: [Option<std::path::PathBuf>; crate::launch::AGENTS],
    pub text: String,
    /// The last result shown under the field: the message and whether it worked.
    pub note: Option<(String, bool)>,
    /// Put the cursor in the field on the next frame.
    pub focus: bool,
    /// Which list is open in place of the field.
    pub picking: Picking,
    /// The existing session chosen per agent; None starts a new one.
    pub session: [Option<String>; crate::launch::AGENTS],
}

#[derive(Clone, Copy, Default, PartialEq)]
pub enum Picking {
    #[default]
    Nothing,
    Project,
    Session,
}

/// What the composer's button does.
pub enum Send {
    /// A new session in this folder, with the text filled in.
    New(std::path::PathBuf),
    /// An existing session, by id and title.
    Existing(String, String),
}

/// A row that opens a list: label on the left, the current value on the right.
fn picker_row(ui: &egui::Ui, row: Rect, id: &str, label: &str, value: &str, open: bool) -> bool {
    let response = ui
        .interact(row, ui.id().with(id), egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, value));
    let painter = ui.painter();
    field(painter, row, open || response.hovered());
    painter.text(
        Pos2::new(row.left() + 12.0, row.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        regular(11.5),
        SECONDARY,
    );
    let shown: String = if value.chars().count() > 26 {
        format!("{}…", value.chars().take(25).collect::<String>().trim_end())
    } else {
        value.to_owned()
    };
    painter.text(
        Pos2::new(row.right() - 30.0, row.center().y),
        egui::Align2::RIGHT_CENTER,
        shown,
        strong(12.5),
        LABEL,
    );
    painter.text(
        Pos2::new(row.right() - 14.0, row.center().y),
        egui::Align2::CENTER_CENTER,
        if open { "▴" } else { "▾" },
        regular(11.0),
        SECONDARY,
    );
    response.clicked()
}

/// One line of a list: the title, and quieter text on the right.
struct Choice {
    title: String,
    side: Option<String>,
    tooltip: Option<String>,
    selected: bool,
}

/// The scrolling list that opens in place of the field. Returns the line clicked.
fn pick_list(ui: &mut egui::Ui, area: Rect, id: &str, choices: &[Choice]) -> Option<usize> {
    ui.painter()
        .rect_filled(area, 10.0, Color32::from_black_alpha(65));
    let mut picked = None;
    ui.scope_builder(egui::UiBuilder::new().max_rect(area.shrink(3.0)), |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.spacing_mut().scroll.bar_width = 4.0;
        egui::ScrollArea::vertical().id_salt(id).show(ui, |ui| {
            for (i, choice) in choices.iter().enumerate() {
                let (line, mut response) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width(), 22.0),
                    egui::Sense::click(),
                );
                response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
                if let Some(tooltip) = &choice.tooltip {
                    response = response.on_hover_text(tooltip);
                }
                let painter = ui.painter();
                if choice.selected {
                    painter.rect_filled(line, 7.0, Color32::from_white_alpha(22));
                } else if response.hovered() {
                    painter.rect_filled(line, 7.0, Color32::from_white_alpha(8));
                }
                let side = choice
                    .side
                    .clone()
                    .map(|text| painter.layout_no_wrap(text, regular(10.5), GRAY));
                let right = line.right() - side.as_ref().map_or(9.0, |side| side.size().x + 20.0);
                // Long titles are cut by the column, never run into the side text.
                painter
                    .with_clip_rect(Rect::from_min_max(
                        line.min,
                        Pos2::new(right, line.bottom()),
                    ))
                    .text(
                        Pos2::new(line.left() + 9.0, line.center().y),
                        egui::Align2::LEFT_CENTER,
                        &choice.title,
                        strong(12.0),
                        if choice.selected { LABEL } else { SECONDARY },
                    );
                if let Some(side) = side {
                    painter.galley(
                        Pos2::new(right + 8.0, line.center().y - side.size().y / 2.0),
                        side,
                        GRAY,
                    );
                }
                if response.clicked() {
                    picked = Some(i);
                }
            }
        });
    });
    picked
}

fn ago(at: u64, now: u64) -> String {
    let since = elapsed(at, now);
    if now.saturating_sub(at) < 60 {
        since
    } else {
        t!("{since} тому", "{since} назад", "{since} ago")
    }
}

const FIELD_HEIGHT: f32 = 92.0;

pub fn compose_size() -> Vec2 {
    Vec2::new(WIDTH, PAD + 286.0 + FIELD_HEIGHT - 88.0 + 30.0 + PAD)
}

/// Window size for the composer above the cat.
pub fn compose_window(size: Vec2) -> Vec2 {
    Vec2::new(size.x + MARGIN * 2.0, 88.0 + size.y + MARGIN + 6.0)
}

/// What the composer asks of the app this frame.
pub struct ComposeOut {
    pub area: Rect,
    /// The text field has the keyboard.
    pub typing: bool,
    pub send: Option<Send>,
    pub close: bool,
}

/// Draws the composer with its bottom edge just above the cat, in the same glass, header,
/// switch, field and button as every other window.
pub fn compose(
    ui: &mut egui::Ui,
    bottom_right: Pos2,
    state: &mut Compose,
    projects: &[Vec<crate::projects::Project>; crate::launch::AGENTS],
    installed: &[usize],
) -> ComposeOut {
    if let Some(first) = installed.first()
        && !installed.contains(&state.agent)
    {
        state.agent = *first;
    }
    let size = compose_size();
    let rect = Rect::from_min_max(bottom_right - size, bottom_right);
    let color = tint(state.agent);
    let (left, right) = (rect.left() + PAD + 4.0, rect.right() - PAD - 4.0);
    let top = rect.top() + PAD;
    let mut out = ComposeOut {
        area: rect,
        typing: false,
        send: None,
        close: false,
    };
    let app = crate::launch::APPS[state.agent];
    // The model it will run with, when the app tells us: the session's, else the project's.
    let model = {
        let list = &projects[state.agent];
        let project = state.project[state.agent]
            .as_ref()
            .and_then(|path| list.iter().find(|project| &project.path == path))
            .or_else(|| list.iter().find(|project| project.preferred))
            .or(list.first());
        project.and_then(|project| {
            state.session[state.agent]
                .as_ref()
                .and_then(|id| project.sessions.iter().find(|thread| &thread.id == id))
                .and_then(|thread| thread.model.clone())
                .or_else(|| project.model.clone())
        })
    };
    {
        let painter = ui.painter();
        glass(painter, rect, Some(color));
        header(
            painter,
            left,
            top,
            right - left - 40.0,
            s("Нове завдання", "Новая задача", "New task").to_owned(),
            match model {
                Some(model) => t!(
                    "Відкриється в застосунку {app} · {model}",
                    "Откроется в приложении {app} · {model}",
                    "Opens in the {app} app · {model}"
                ),
                None => t!(
                    "Відкриється в застосунку {app}",
                    "Откроется в приложении {app}",
                    "Opens in the {app} app"
                ),
            },
        );
    }
    let close = Rect::from_min_size(Pos2::new(right - 30.0, top - 4.0), Vec2::splat(30.0));
    if icon_button(
        ui,
        close,
        "compose-close",
        "✕",
        false,
        s("Закрити", "Закрыть", "Close"),
    ) {
        out.close = true;
    }
    // Which agent.
    let switch = Rect::from_min_size(Pos2::new(left, top + 58.0), Vec2::new(right - left, 32.0));
    let agents: Vec<(&str, Option<Color32>)> = installed
        .iter()
        .map(|&agent| (crate::launch::APPS[agent], Some(tint(agent))))
        .collect();
    let chosen = installed
        .iter()
        .position(|&agent| agent == state.agent)
        .unwrap_or(0);
    if let Some(i) = segmented(ui, switch, "compose-agent", &agents, chosen) {
        state.agent = installed[i];
        state.note = None;
        state.picking = Picking::Nothing;
    }
    // Which project: the one the app has current by default; the row opens the list.
    let list = &projects[state.agent];
    let chosen = state.project[state.agent]
        .clone()
        .filter(|path| list.iter().any(|project| &project.path == path))
        .or_else(|| {
            list.iter()
                .find(|project| project.preferred)
                .or(list.first())
                .map(|project| project.path.clone())
        });
    let current = chosen
        .as_ref()
        .and_then(|path| list.iter().find(|project| &project.path == path));
    let name = current.map_or_else(
        || s("немає проєктів", "нет проектов", "no projects").to_owned(),
        |project| project.name.clone(),
    );
    let width = right - left;
    let toggle = |picking: &mut Picking, which: Picking| {
        *picking = if *picking == which {
            Picking::Nothing
        } else {
            which
        };
    };
    let row = Rect::from_min_size(Pos2::new(left, top + 102.0), Vec2::new(width, 32.0));
    let open = state.picking == Picking::Project;
    if picker_row(
        ui,
        row,
        "compose-project",
        s("Проєкт", "Проект", "Project"),
        &name,
        open,
    ) {
        toggle(&mut state.picking, Picking::Project);
    }
    // Which session in it: a new one by default.
    let sessions: &[crate::projects::Thread] = current.map_or(&[], |project| &project.sessions);
    let thread = state.session[state.agent]
        .as_ref()
        .and_then(|id| sessions.iter().find(|thread| &thread.id == id));
    let untitled = s("Без назви", "Без названия", "Untitled");
    let title = |thread: &crate::projects::Thread| {
        if thread.title.is_empty() {
            untitled.to_owned()
        } else {
            thread.title.clone()
        }
    };
    let new_session = s("Нова сесія", "Новая сессия", "New session");
    let row = Rect::from_min_size(Pos2::new(left, top + 142.0), Vec2::new(width, 32.0));
    let open = state.picking == Picking::Session;
    let value = thread.map_or_else(|| new_session.to_owned(), title);
    if picker_row(
        ui,
        row,
        "compose-session",
        s("Сесія", "Сессия", "Session"),
        &value,
        open,
    ) {
        toggle(&mut state.picking, Picking::Session);
    }
    let area = Rect::from_min_size(Pos2::new(left, top + 186.0), Vec2::new(width, FIELD_HEIGHT));
    let agent = state.agent;
    if state.picking == Picking::Session {
        let now = crate::usage::now();
        let mut choices = vec![Choice {
            title: new_session.to_owned(),
            side: None,
            tooltip: None,
            selected: thread.is_none(),
        }];
        choices.extend(sessions.iter().map(|item| Choice {
            title: title(item),
            side: Some(match &item.model {
                Some(model) => format!("{} · {model}", ago(item.last, now)),
                None => ago(item.last, now),
            }),
            tooltip: Some(title(item)),
            selected: thread.is_some_and(|thread| thread.id == item.id),
        }));
        if let Some(i) = pick_list(ui, area, "compose-sessions", &choices) {
            state.session[agent] = i.checked_sub(1).map(|i| sessions[i].id.clone());
            state.picking = Picking::Nothing;
            state.note = None;
        }
    } else if state.picking == Picking::Project {
        // The projects, in the app's own order, in place of the field; scroll for more.
        let choices: Vec<Choice> = list
            .iter()
            .map(|project| Choice {
                title: project.name.clone(),
                side: project.model.clone(),
                tooltip: Some(project.path.display().to_string()),
                selected: chosen.as_ref() == Some(&project.path),
            })
            .collect();
        if let Some(i) = pick_list(ui, area, "compose-projects", &choices) {
            state.project[agent] = Some(list[i].path.clone());
            state.session[agent] = None; // sessions belong to the project just left
            state.picking = Picking::Nothing;
        }
        if list.is_empty() {
            ui.painter().text(
                area.center(),
                egui::Align2::CENTER_CENTER,
                s("Немає проєктів", "Нет проектов", "No projects"),
                regular(11.5),
                GRAY,
            );
        }
    } else {
        // What to do. Enter starts; Shift+Enter makes a new line.
        field(ui.painter(), area, false);
        let editor = egui::TextEdit::multiline(&mut state.text)
            .frame(false)
            .hint_text(
                egui::RichText::new(s("Що зробити?", "Что сделать?", "What should it do?"))
                    .color(GRAY),
            )
            .font(regular(13.0))
            .text_color(LABEL)
            .desired_width(area.width() - 24.0)
            .return_key(Some(egui::KeyboardShortcut::new(
                egui::Modifiers::SHIFT,
                egui::Key::Enter,
            )));
        let inner = area.shrink2(Vec2::new(12.0, 10.0));
        let response = ui.put(inner, editor);
        if state.focus {
            response.request_focus();
            state.focus = false;
        }
        out.typing = response.has_focus();
        if out.typing {
            // Only the outline: the fill is already under the text.
            ui.painter().rect_stroke(
                area,
                10.0,
                Stroke::new(1.0, Color32::from_white_alpha(46)),
                StrokeKind::Inside,
            );
        }
    }
    let enter = out.typing
        && ui.input(|input| input.key_pressed(egui::Key::Enter) && !input.modifiers.shift);
    // The result of the last try, or how to send.
    let footer = top + 186.0 + FIELD_HEIGHT + 14.0;
    let (message, tone) = match &state.note {
        Some((message, ok)) => (message.clone(), if *ok { READY } else { STOP }),
        // Where the link takes no text, the text goes to the clipboard.
        None if (thread.is_some() || !crate::launch::takes_text(state.agent))
            && !state.text.trim().is_empty() =>
        {
            (
                s(
                    "Текст скопіюється, встав Ctrl+V",
                    "Текст скопируется, вставь Ctrl+V",
                    "Text gets copied, paste with Ctrl+V",
                )
                .to_owned(),
                GRAY,
            )
        }
        None if thread.is_some() => (
            s("Enter — відкрити", "Enter — открыть", "Enter to open").to_owned(),
            GRAY,
        ),
        None => (
            s("Enter — запустити", "Enter — запустить", "Enter to start").to_owned(),
            GRAY,
        ),
    };
    let start = Rect::from_min_size(
        Pos2::new(right - 108.0, footer - 2.0),
        Vec2::new(108.0, 30.0),
    );
    {
        let painter = ui.painter();
        let note = painter.layout(message, regular(11.0), tone, start.left() - left - 10.0);
        painter.galley(Pos2::new(left, footer + 6.0), note, tone);
    }
    if primary_button(
        ui,
        start,
        "compose-start",
        if thread.is_some() {
            s("Відкрити", "Открыть", "Open")
        } else {
            s("Запустити", "Запустить", "Start")
        },
        color,
    ) || enter
    {
        match (thread, chosen) {
            (Some(thread), _) => out.send = Some(Send::Existing(thread.id.clone(), title(thread))),
            (None, Some(path)) => out.send = Some(Send::New(path)),
            (None, None) => {
                state.note = Some((
                    s(
                        "Немає проєкту для запуску",
                        "Нет проекта для запуска",
                        "No project to start in",
                    )
                    .to_owned(),
                    false,
                ));
            }
        }
    }
    out
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

/// Soft identity tints, not brand marks: warm for Claude, cool for Codex, violet for Kimi.
pub fn tint(agent: usize) -> Color32 {
    match agent {
        0 => Color32::from_rgb(244, 146, 108),
        1 => Color32::from_rgb(118, 196, 255),
        _ => Color32::from_rgb(186, 160, 255),
    }
}

fn glyph(agent: usize) -> &'static str {
    // Generic symbols: a spark for Claude, a prompt chevron for Codex, a moon for Kimi.
    ["✳", "❯", "☾"][agent]
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
    /// Finished rather than working: a check mark instead of the bouncing dots.
    pub done: bool,
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
        if puff.done {
            let x = seg.left() + 26.0 * scale;
            let y = seg.center().y;
            let points = [
                Pos2::new(x, y),
                Pos2::new(x + 4.0 * scale, y + 4.0 * scale),
                Pos2::new(x + 12.0 * scale, y - 4.5 * scale),
            ];
            painter.add(Shape::line(
                points.to_vec(),
                Stroke::new(2.0 * scale, fade(READY)),
            ));
        } else {
            // Three dots bounce in a wave; faster and higher under the pointer.
            let speed = 6.0 + 4.0 * puff.hover;
            let jump = 2.5 + 1.5 * puff.hover;
            for k in 0..3 {
                let bounce =
                    time.map_or(0.0, |_| (t * speed - k as f32 * 0.9).sin().max(0.0) * jump);
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
    let name = crate::launch::APPS[agent];
    let color = tint(agent);
    glass(painter, rect, Some(color));
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

pub fn done_card_size(done: &[crate::activity::Done]) -> Vec2 {
    Vec2::new(
        BUSY_WIDTH,
        40.0 + done.len().min(BUSY_ROWS) as f32 * SESSION_ROW + 34.0,
    )
}

/// What just finished: the project, how long ago, and the task it was given.
pub fn done_card(
    painter: &egui::Painter,
    rect: Rect,
    agent: usize,
    done: &[crate::activity::Done],
    now: u64,
) {
    let name = crate::launch::APPS[agent];
    let color = tint(agent);
    glass(painter, rect, Some(color));
    let (left, right) = (rect.left() + 16.0, rect.right() - 16.0);
    let mut y = rect.top() + 14.0;
    painter.circle_filled(Pos2::new(left + 4.0, y + 9.0), 3.5, READY);
    painter.text(
        Pos2::new(left + 14.0, y),
        egui::Align2::LEFT_TOP,
        t!("{name} завершив", "{name} закончил", "{name} finished"),
        strong(13.5),
        LABEL,
    );
    y += 30.0;
    let line = |y: f32, text: String, font: egui::FontId, color: Color32, room: f32| {
        // One line only: the width cuts long names and tasks, never wraps them.
        let galley = painter.layout_no_wrap(text, font, color);
        let height = galley.size().y;
        painter
            .with_clip_rect(Rect::from_min_size(
                Pos2::new(left, y),
                Vec2::new(room, height),
            ))
            .galley(Pos2::new(left, y), galley, color);
    };
    for item in done.iter().take(BUSY_ROWS) {
        let since = painter.layout_no_wrap(ago(item.end, now), regular(11.0), SECONDARY);
        let room = right - left - since.size().x - 10.0;
        let project = if item.project.is_empty() {
            s("Без назви", "Без названия", "Untitled").to_owned()
        } else {
            item.project.clone()
        };
        line(y, project, strong(12.0), LABEL, room);
        painter.galley(Pos2::new(right - since.size().x, y + 1.0), since, SECONDARY);
        let task = item.prompt.as_deref().map_or_else(
            || s("Завдання виконано", "Задача выполнена", "Task complete").to_owned(),
            |prompt| prompt.split_whitespace().collect::<Vec<_>>().join(" "),
        );
        line(y + 18.0, task, regular(10.5), SECONDARY, right - left);
        y += SESSION_ROW;
    }
    painter.line_segment(
        [Pos2::new(left, y + 2.0), Pos2::new(right, y + 2.0)],
        Stroke::new(1.0, Color32::from_white_alpha(20)),
    );
    painter.text(
        Pos2::new(left, y + 9.0),
        egui::Align2::LEFT_TOP,
        t!(
            "Натисни, щоб відкрити {name}",
            "Нажми, чтобы открыть {name}",
            "Click to open {name}"
        ),
        regular(10.5),
        SECONDARY,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_summary_handles_one_three_and_no_agents() {
        let now = 1_800_000_000;
        let make = |agent: usize, used: Option<f64>| {
            let records = used.map_or_else(Vec::new, |used| limits(used, used, now));
            row(
                agent,
                &advice::assess(&records, now),
                &records,
                None,
                [None, None],
                now,
                0,
            )
        };
        assert_eq!(summary(&[]).0, "Агентів не знайдено");
        let (title, detail) = summary(&[make(2, Some(10.0))]);
        assert_eq!(
            (title.as_str(), detail.as_str()),
            ("Kimi готовий до роботи", "")
        );
        let all = [
            make(0, Some(10.0)),
            make(1, Some(70.0)),
            make(2, Some(20.0)),
        ];
        let (title, detail) = summary(&all);
        assert_eq!(title, "Усі готові до роботи");
        assert_eq!(detail, "Більше запасу в Claude");
        let mixed = [make(0, None), make(1, Some(95.0)), make(2, Some(20.0))];
        let (title, detail) = summary(&mixed);
        assert_eq!(title, "У Codex лишилось мало");
        assert_eq!(detail, "Працюй у Kimi");
    }

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
