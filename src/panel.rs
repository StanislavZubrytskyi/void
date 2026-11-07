//! Independent provider summaries on a monochrome glass surface.

use crate::{
    advice::{self, Status},
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
const CARD: f32 = 198.0;
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

/// Presentation data belongs to one provider, including both of its quota windows.
pub struct Agent {
    pub name: &'static str,
    pub state: &'static str,
    pub value: String,
    pub caption: String,
    pub windows: Vec<Quota>,
    pub source: String,
}
pub struct Quota {
    pub name: &'static str,
    pub remaining: Option<f32>,
    pub value: String,
    pub reset: String,
}
pub fn agent(
    name: &'static str,
    status: &Status,
    records: &[Record],
    estimated: bool,
    unavailable: Option<&str>,
    now: u64,
    offset: i64,
) -> Agent {
    let (state, value, caption) = if let Some((window, reset)) = status.blocked {
        (
            "ПАУЗА",
            advice::countdown(reset, now),
            format!("до відновлення · ліміт {window}"),
        )
    } else if let Some(risk) = &status.risk {
        (
            "ПРОГНОЗ",
            format!("≈ {}", crate::duration(risk.minutes)),
            risk.prompts
                .map(|n| format!("≈ {n:.0} запитів до ліміту {}", risk.window))
                .unwrap_or_else(|| format!("роботи до ліміту {}", risk.window)),
        )
    } else if let Some((_, minutes, prompts, _)) = status.lasting {
        (
            "ПРОГНОЗ",
            format!("≈ {}", crate::duration(minutes)),
            prompts
                .map(|n| format!("≈ {n:.0} запитів · вистачить до оновлення"))
                .unwrap_or_else(|| "Вистачить до оновлення квоти".into()),
        )
    } else if status.used.is_some_and(|used| used >= 100.0) {
        (
            "ПАУЗА",
            "Ліміт вичерпано".into(),
            "Час відновлення ще невідомий".into(),
        )
    } else if status.used.is_some() {
        (
            "ДОСТУПНО",
            "Можна працювати".into(),
            "Прогноз з’явиться з історією роботи".into(),
        )
    } else if records
        .last()
        .is_some_and(|r| r.sample.session_id == "claude-desktop")
    {
        (
            "ОСТАННІ ДАНІ",
            "Знімок застарів".into(),
            "Чекаю нове вимірювання Claude Desktop".into(),
        )
    } else {
        let speech = advice::speech([(name, status), (name, status)], now, offset);
        (
            "НЕМАЄ ДАНИХ",
            "Очікую квоти".into(),
            unavailable.unwrap_or(&speech.detail).to_owned(),
        )
    };
    let windows = advice::WINDOWS
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
                        value: format!("{}{left:.0}% лишилось", if estimated { "≈ " } else { "" }),
                        reset: if limit.resets_at == 0 {
                            "Час невідомий".into()
                        } else {
                            format!("↻ {}", advice::clock(limit.resets_at, now, offset))
                        },
                    }
                }
                None => Quota {
                    name,
                    remaining: None,
                    value: latest
                        .map(|(limit, _)| format!("{:.0}% лишалось", 100.0 - limit.used_percentage))
                        .unwrap_or_else(|| "Невідомо".into()),
                    reset: if latest.is_some() {
                        "Очікую оновлення"
                    } else {
                        "Ще немає вимірювань"
                    }
                    .into(),
                },
            }
        })
        .collect();
    let seen = records
        .iter()
        .filter(|r| r.sample.rate_limits.is_some())
        .map(|r| r.time)
        .max();
    let age = seen.map(|at| now.saturating_sub(at));
    let freshness = match age {
        Some(0..=59) => "щойно".into(),
        Some(60..=3599) => format!("{} хв тому", age.unwrap() / 60),
        Some(seconds) => format!("{} год тому", seconds / 3600),
        None => "очікування".into(),
    };
    let source = format!(
        "{} · {freshness}",
        if records
            .last()
            .is_some_and(|row| row.sample.session_id == "claude-desktop")
        {
            "Історія Claude Desktop"
        } else if records
            .last()
            .is_some_and(|row| row.sample.session_id == "claude-block")
        {
            "Блокування Claude"
        } else if estimated {
            "Локальна оцінка"
        } else if age.is_some_and(|age| age > 1800) {
            "Останні дані"
        } else {
            "Дані квот"
        }
    );
    Agent {
        name,
        state,
        value,
        caption,
        windows,
        source,
    }
}
pub struct Bubble {
    agents: [Agent; 2],
    pub size: Vec2,
}
pub fn layout(agents: [Agent; 2]) -> Bubble {
    let height = PAD * 2.0 + CARD + 40.0;
    Bubble {
        agents,
        size: Vec2::new(WIDTH, height),
    }
}
pub fn window_size(bubble: &Bubble) -> Vec2 {
    Vec2::new(
        bubble.size.x + MARGIN * 2.0,
        88.0 + bubble.size.y + MARGIN + 6.0,
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

/// Draws the bubble with its bottom edge just above the cat and the tail pointing at it.
pub fn draw(
    ui: &egui::Ui,
    bubble: &Bubble,
    bottom_right: Pos2,
    _anchor: f32,
    selected: &mut usize,
) {
    let painter = ui.painter();
    let rect = Rect::from_min_max(bottom_right - bubble.size, bottom_right);
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
    *selected = (*selected).min(1);
    let tabs = Rect::from_min_size(
        rect.min + Vec2::new(PAD, 14.0),
        Vec2::new(WIDTH - PAD * 2.0, 32.0),
    );
    painter.rect_filled(tabs, 10.0, Color32::from_black_alpha(65));
    for (index, agent) in bubble.agents.iter().enumerate() {
        let cell = Rect::from_min_size(
            tabs.min + Vec2::new(index as f32 * tabs.width() / 2.0, 0.0),
            Vec2::new(tabs.width() / 2.0, tabs.height()),
        )
        .shrink(3.0);
        let response = ui
            .interact(
                cell,
                ui.id().with(("provider-tab", index)),
                egui::Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::SelectableLabel,
                true,
                *selected == index,
                agent.name,
            )
        });
        if response.clicked() {
            *selected = index;
        }
        if *selected == index {
            gradient(
                painter,
                cell,
                8.0,
                Color32::from_white_alpha(38),
                Color32::from_white_alpha(18),
            );
            painter.rect_stroke(
                cell,
                8.0,
                Stroke::new(0.5, Color32::from_white_alpha(30)),
                StrokeKind::Inside,
            );
        } else if response.hovered() {
            painter.rect_filled(cell, 8.0, Color32::from_white_alpha(8));
        }
        if response.has_focus() {
            painter.rect_stroke(cell, 8.0, Stroke::new(1.0, LABEL), StrokeKind::Inside);
        }
        painter.text(
            cell.center(),
            egui::Align2::CENTER_CENTER,
            agent.name,
            strong(12.5),
            if *selected == index { LABEL } else { SECONDARY },
        );
    }
    {
        let agent = &bubble.agents[*selected];
        let top = rect.top() + PAD + 40.0;
        let card = Rect::from_min_size(
            Pos2::new(rect.left() + 10.0, top - 6.0),
            Vec2::new(WIDTH - 20.0, CARD + 2.0),
        );
        gradient(
            painter,
            card,
            16.0,
            Color32::from_white_alpha(10),
            Color32::from_white_alpha(2),
        );
        painter.rect_stroke(
            card,
            16.0,
            Stroke::new(0.5, Color32::from_white_alpha(19)),
            StrokeKind::Inside,
        );
        let left = rect.left() + PAD + 8.0;
        let right = rect.right() - PAD - 8.0;
        let text = |position, content: &str, font, color| {
            painter.text(position, egui::Align2::LEFT_TOP, content, font, color);
        };
        text(Pos2::new(left, top + 4.0), agent.name, strong(15.0), LABEL);
        painter.text(
            Pos2::new(right, top + 7.0),
            egui::Align2::RIGHT_TOP,
            agent.state,
            regular(10.0),
            SECONDARY,
        );
        text(
            Pos2::new(left, top + 31.0),
            &agent.value,
            strong(24.0),
            LABEL,
        );
        // Wrapped supporting copy has its own bounded region, never overlays the quotas.
        let caption = painter.layout(
            agent.caption.clone(),
            regular(11.5),
            SECONDARY,
            right - left,
        );
        painter.galley(Pos2::new(left, top + 64.0), caption, SECONDARY);
        let cell_width = (right - left - 20.0) / 2.0;
        for (i, quota) in agent.windows.iter().enumerate() {
            let x = left + i as f32 * (cell_width + 20.0);
            text(
                Pos2::new(x, top + 100.0),
                quota.name,
                regular(11.0),
                SECONDARY,
            );
            text(Pos2::new(x, top + 117.0), &quota.value, strong(12.0), LABEL);
            let track = Rect::from_min_size(Pos2::new(x, top + 138.0), Vec2::new(cell_width, 3.0));
            painter.rect_filled(track, 2.0, Color32::from_white_alpha(25));
            if let Some(share) = quota.remaining.filter(|share| *share > 0.0) {
                let fill = Rect::from_min_size(track.min, Vec2::new(track.width() * share, 3.0));
                painter.rect_filled(fill, 2.0, LABEL);
            }
            text(
                Pos2::new(x, top + 148.0),
                &quota.reset,
                regular(10.0),
                SECONDARY,
            );
        }
        text(
            Pos2::new(left, top + 178.0),
            &agent.source,
            regular(10.0),
            SECONDARY,
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
    match (count % 10, count % 100) {
        (1, n) if n != 11 => "сесія",
        (2..=4, n) if !(12..=14).contains(&n) => "сесії",
        _ => "сесій",
    }
}

fn elapsed(since: u64, now: u64) -> String {
    let minutes = now.saturating_sub(since) / 60;
    if minutes == 0 {
        "щойно".into()
    } else {
        crate::duration(minutes as f64)
    }
}

/// `mcp__server__tool` reads as `tool`.
fn tool_name(tool: &str) -> &str {
    tool.rsplit("__").next().unwrap_or(tool)
}

pub fn busy_card_size(sessions: usize) -> Vec2 {
    let rows = sessions.min(BUSY_ROWS) as f32;
    let more = if sessions > BUSY_ROWS { 18.0 } else { 0.0 };
    Vec2::new(BUSY_WIDTH, 44.0 + rows * 40.0 + more + 34.0)
}

/// What the hovered agent is doing: one row per busy session and the quota in one line.
pub fn busy_card(
    painter: &egui::Painter,
    rect: Rect,
    agent: usize,
    sessions: &[crate::activity::Session],
    quota: &str,
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
        format!("{name} працює"),
        strong(13.5),
        LABEL,
    );
    painter.text(
        Pos2::new(right, y + 2.0),
        egui::Align2::RIGHT_TOP,
        format!("{} {}", sessions.len(), sessions_word(sessions.len())),
        regular(10.5),
        SECONDARY,
    );
    y += 30.0;
    for session in sessions.iter().take(BUSY_ROWS) {
        let project = if session.project.is_empty() {
            "Без назви"
        } else {
            &session.project
        };
        let time = painter.layout_no_wrap(elapsed(session.since, now), regular(11.0), SECONDARY);
        let room = right - left - time.size().x - 10.0;
        let title = painter.layout(project.to_owned(), strong(12.0), LABEL, room);
        // One line only: long folder names are clipped by the width, not wrapped.
        let title_row = title.rows.first().map_or(0.0, |row| row.height());
        painter
            .with_clip_rect(Rect::from_min_size(
                Pos2::new(left, y),
                Vec2::new(room, title_row),
            ))
            .galley(Pos2::new(left, y), title, LABEL);
        painter.galley(Pos2::new(right - time.size().x, y + 1.0), time, SECONDARY);
        let doing = session.tool.as_deref().map_or_else(
            || "думає".to_owned(),
            |tool| format!("виконує {}", tool_name(tool)),
        );
        painter.text(
            Pos2::new(left, y + 18.0),
            egui::Align2::LEFT_TOP,
            doing,
            regular(10.5),
            SECONDARY,
        );
        y += 40.0;
    }
    if sessions.len() > BUSY_ROWS {
        painter.text(
            Pos2::new(left, y - 2.0),
            egui::Align2::LEFT_TOP,
            format!("і ще {}", sessions.len() - BUSY_ROWS),
            regular(10.5),
            GRAY,
        );
        y += 18.0;
    }
    painter.line_segment(
        [Pos2::new(left, y + 2.0), Pos2::new(right, y + 2.0)],
        Stroke::new(1.0, Color32::from_white_alpha(20)),
    );
    painter.text(
        Pos2::new(left, y + 11.0),
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
        assert_eq!(busy_card_size(5).y, busy_card_size(3).y + 18.0);
    }
    #[test]
    fn provider_cards_keep_their_own_windows_and_estimate_labels() {
        let now = 1_800_000_000;
        let claude = vec![crate::demo_claude_snapshot(now)];
        let codex = crate::demo_codex_history(now);
        let a = agent(
            "Claude",
            &advice::assess(&claude, now),
            &claude,
            true,
            None,
            now,
            0,
        );
        let b = agent(
            "Codex",
            &advice::assess(&codex, now),
            &codex,
            false,
            None,
            now,
            0,
        );
        assert_eq!(a.state, "ПАУЗА");
        assert_eq!(a.windows[0].remaining, None);
        assert_eq!(a.windows[1].remaining, Some(0.0));
        assert!(a.source.starts_with("Локальна оцінка"));
        assert_eq!(b.state, "ПРОГНОЗ");
        assert_eq!(b.windows[0].value, "24% лишилось");
        assert!(b.caption.contains("12 запитів"));
    }
    #[test]
    fn missing_or_expired_measurements_never_render_as_free_quota() {
        let now = 1_800_000_000;
        let rows = crate::demo_codex_history(now);
        let expired = now + 86401;
        let card = agent(
            "Codex",
            &advice::assess(&rows, expired),
            &rows,
            false,
            Some("Немає зв’язку"),
            expired,
            0,
        );
        assert_eq!(card.state, "НЕМАЄ ДАНИХ");
        assert!(card.windows.iter().all(|window| window.remaining.is_none()));
        assert_eq!(card.caption, "Немає зв’язку");
    }
}
