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

/// One agent's line in the menu: what is left, until when, and what was spent.
pub struct Row {
    pub agent: usize,
    /// "84%" of the tighter limit, or "пауза".
    pub big: String,
    pub paused: bool,
    /// "лишилось · 5 год ↻ о 23:50" or "повернеться о 19:28".
    pub under: String,
    /// "Витрачено 1,2 млн за 5 год · 9,8 млн за тиждень".
    pub spent: Option<String>,
}

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
    let windows = quotas(records, now, offset);
    let tighter = windows
        .iter()
        .filter(|quota| quota.remaining.is_some())
        .min_by(|a, b| {
            a.remaining
                .partial_cmp(&b.remaining)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    let (big, paused, under) = if let Some((_, reset)) = status.blocked {
        let back = advice::clock(reset, now, offset);
        (
            s("пауза", "пауза", "paused").to_owned(),
            true,
            t!("повернеться {back}", "вернётся {back}", "back {back}"),
        )
    } else if let Some(quota) = tighter {
        let left = quota.remaining.unwrap_or(0.0) * 100.0;
        let (window, reset) = (quota.name, &quota.reset);
        (
            format!("{left:.0}%"),
            false,
            t!(
                "лишилось · {window} {reset}",
                "осталось · {window} {reset}",
                "left · {window} {reset}"
            ),
        )
    } else {
        (
            "—".to_owned(),
            false,
            unavailable
                .unwrap_or(s("немає даних", "нет данных", "no data"))
                .to_owned(),
        )
    };
    let spent = match spent {
        [Some(five), Some(week)] if week > 0 => {
            let (five, week) = (short_tokens(five), short_tokens(week));
            Some(t!(
                "Витрачено {five} за 5 год · {week} за тиждень",
                "Потрачено {five} за 5 ч · {week} за неделю",
                "Spent {five} in 5 h · {week} this week"
            ))
        }
        _ => None,
    };
    Row {
        agent,
        big,
        paused,
        under,
        spent,
    }
}

const HEADER: f32 = 62.0;
const ROW_HEIGHT: f32 = 72.0;

/// The menu: one verdict, one line per agent, and settings behind the gear.
pub struct Menu {
    pub title: String,
    pub detail: String,
    pub rows: [Row; 2],
    pub size: Vec2,
}

pub fn menu(title: String, detail: String, rows: [Row; 2]) -> Menu {
    Menu {
        title,
        detail,
        rows,
        size: Vec2::new(WIDTH, PAD + HEADER + 2.0 * ROW_HEIGHT + PAD - 8.0),
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
        for (i, row) in menu.rows.iter().enumerate() {
            let y = area.top() + i as f32 * ROW_HEIGHT;
            painter.line_segment(
                [Pos2::new(left, y), Pos2::new(right, y)],
                Stroke::new(1.0, Color32::from_white_alpha(18)),
            );
            let color = tint(row.agent);
            painter.circle_filled(Pos2::new(left + 4.0, y + 21.0), 4.0, color);
            painter.text(
                Pos2::new(left + 16.0, y + 12.0),
                egui::Align2::LEFT_TOP,
                ["Claude", "Codex"][row.agent],
                strong(13.5),
                LABEL,
            );
            painter.text(
                Pos2::new(right, y + 8.0),
                egui::Align2::RIGHT_TOP,
                &row.big,
                strong(if row.paused { 17.0 } else { 24.0 }),
                if row.paused { SECONDARY } else { LABEL },
            );
            painter.text(
                Pos2::new(left + 16.0, y + 31.0),
                egui::Align2::LEFT_TOP,
                &row.under,
                regular(11.0),
                SECONDARY,
            );
            if let Some(spent) = &row.spent {
                painter.text(
                    Pos2::new(left + 16.0, y + 48.0),
                    egui::Align2::LEFT_TOP,
                    spent,
                    regular(11.0),
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
    #[test]
    fn menu_rows_lead_with_what_is_left_and_what_was_spent() {
        let now = 1_800_000_000;
        let codex = crate::demo_codex_history(now);
        let row = row(
            1,
            &advice::assess(&codex, now),
            &codex,
            None,
            [Some(1_250_000), Some(9_800_000)],
            now,
            0,
        );
        assert_eq!(row.big, "24%");
        assert!(
            row.under.starts_with("лишилось · 5 год ↻ "),
            "{}",
            row.under
        );
        assert_eq!(
            row.spent.as_deref(),
            Some("Витрачено 1,2 млн за 5 год · 9,8 млн за тиждень")
        );
        let claude = vec![crate::demo_claude_snapshot(now)];
        let paused = super::row(
            0,
            &advice::assess(&claude, now),
            &claude,
            None,
            [None, None],
            now,
            0,
        );
        assert!(paused.paused);
        assert!(paused.under.starts_with("повернеться "), "{}", paused.under);
        assert_eq!(paused.spent, None);
        let expired = now + 86_401;
        let unknown = super::row(
            1,
            &advice::assess(&codex, expired),
            &codex,
            Some("Немає зв’язку"),
            [None, None],
            expired,
            0,
        );
        assert_eq!(
            (unknown.big.as_str(), unknown.under.as_str()),
            ("—", "Немає зв’язку")
        );
        assert_eq!(short_tokens(640_400), "640 тис.");
        assert_eq!(short_tokens(950), "950");
    }
}
