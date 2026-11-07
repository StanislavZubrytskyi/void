//! Turns measurements into an answer: a verdict, a clock time and, when it helps, one action.

use crate::{
    duration,
    i18n::s,
    usage::{self, Limit, Limits, Record, SelectLimit},
};

fn five_hour(limits: &Limits) -> Option<Limit> {
    limits.five_hour
}

fn seven_day(limits: &Limits) -> Option<Limit> {
    limits.seven_day
}

/// The two quota windows, named in the interface language.
pub fn windows() -> [(&'static str, SelectLimit); 2] {
    [
        (s("5 год", "5 ч", "5 h"), five_hour),
        (s("7 днів", "7 дней", "7 days"), seven_day),
    ]
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tone {
    Unknown,
    Calm,
    Warn,
    Blocked,
}

pub struct Risk {
    pub window: &'static str,
    pub minutes: f64,
    pub reset_at: u64,
    pub prompts: Option<f64>,
}

pub struct Status {
    pub tone: Tone,
    /// Highest current usage over both windows; None when nothing is known.
    pub used: Option<f64>,
    pub blocked: Option<(&'static str, u64)>,
    pub risk: Option<Risk>,
    /// Window, minutes of work left at the usual pace, typical prompts and its reset.
    pub lasting: Option<(&'static str, f64, Option<f64>, u64)>,
    pub opus: bool,
}

pub fn assess(records: &[Record], time: u64) -> Status {
    let blocked = windows()
        .iter()
        .filter_map(|(name, select)| {
            usage::blocked_until(records, *select, time).map(|reset| (*name, reset))
        })
        .max_by_key(|(_, reset)| *reset);
    let forecasts = windows().map(|(name, select)| (name, usage::forecast(records, select, time)));
    let risk = forecasts
        .iter()
        .filter_map(|(window, forecast)| {
            let forecast = forecast.as_ref()?;
            let minutes = forecast.minutes?;
            let reset_at = forecast.limit.resets_at;
            (minutes * 60.0 < reset_at.saturating_sub(time) as f64).then_some(Risk {
                window,
                minutes,
                reset_at,
                prompts: forecast.typical_prompts,
            })
        })
        .min_by(|a, b| a.minutes.total_cmp(&b.minutes))
        .filter(|_| blocked.is_none());
    let known = forecasts
        .iter()
        .all(|(_, forecast)| forecast.as_ref().is_some_and(|f| f.minutes.is_some()));
    let lasting = (blocked.is_none() && risk.is_none() && known)
        .then(|| {
            forecasts
                .iter()
                .filter_map(|(window, forecast)| {
                    let forecast = forecast.as_ref()?;
                    Some((
                        *window,
                        forecast.minutes?,
                        forecast.typical_prompts,
                        forecast.limit.resets_at,
                    ))
                })
                .min_by(|a, b| a.1.total_cmp(&b.1))
        })
        .flatten();
    let used = windows()
        .iter()
        .filter_map(|(_, select)| usage::last_window(records, *select))
        .filter(|(limit, seen)| {
            (limit.resets_at == 0 || limit.resets_at > time)
                && ((limit.used_percentage >= 100.0 && limit.resets_at > 0)
                    || time.saturating_sub(*seen) <= 1800)
        })
        .map(|(limit, _)| limit.used_percentage)
        .max_by(f64::total_cmp);
    let opus = records
        .last()
        .and_then(|record| record.sample.model.as_ref()?.id.as_deref())
        .is_some_and(|id| id.contains("opus"));
    let tone = if blocked.is_some() {
        Tone::Blocked
    } else if risk.is_some() {
        Tone::Warn
    } else if lasting.is_some() {
        Tone::Calm
    } else {
        Tone::Unknown
    };
    Status {
        tone,
        used,
        blocked,
        risk,
        lasting,
        opus,
    }
}

/// What the cat says: one thought in two short lines.
#[derive(Debug, PartialEq)]
pub struct Speech {
    pub title: String,
    pub detail: String,
    pub tone: Tone,
}

fn say(title: String, detail: String, tone: Tone) -> Speech {
    Speech {
        title,
        detail,
        tone,
    }
}

/// "18:50", "завтра 09:00" or "пт 12:00": the clock without the preposition.
fn until(at: u64, time: u64, offset: i64) -> String {
    moment(at, time, offset, false)
}

/// The single most useful thing to say about both agents right now.
pub fn speech(agents: [(&str, &Status); 2], time: u64, offset: i64) -> Speech {
    // Another agent is worth suggesting only when it is not running out itself.
    let free = |status: &Status| {
        status.used.is_some() && matches!(status.tone, Tone::Calm | Tone::Unknown)
    };
    let other = |name: &str| {
        agents
            .into_iter()
            .find(|(other, _)| *other != name)
            .unwrap()
    };

    let blocked: Vec<_> = agents
        .into_iter()
        .filter_map(|(name, status)| status.blocked.map(|(_, reset)| (name, reset)))
        .collect();
    match blocked.as_slice() {
        [(a, reset_a), (b, reset_b)] => {
            return say(
                s("Обидва на паузі", "Оба на паузе", "Both are paused").into(),
                t!(
                    "{a} до {} · {b} до {}",
                    "{a} до {} · {b} до {}",
                    "{a} until {} · {b} until {}",
                    until(*reset_a, time, offset),
                    until(*reset_b, time, offset)
                ),
                Tone::Blocked,
            );
        }
        [(name, reset)] => {
            let (other_name, other_status) = other(name);
            return say(
                t!(
                    "{name} на паузі до {}",
                    "{name} на паузе до {}",
                    "{name} paused until {}",
                    until(*reset, time, offset)
                ),
                if free(other_status) {
                    t!(
                        "{other_name} вільний — працюй там",
                        "{other_name} свободен — работай там",
                        "{other_name} is free — work there"
                    )
                } else {
                    t!(
                        "Доступ повернеться через {}",
                        "Доступ вернётся через {}",
                        "Access returns in {}",
                        countdown(*reset, time)
                    )
                },
                Tone::Blocked,
            );
        }
        _ => {}
    }

    if let Some((name, status, risk)) = agents
        .into_iter()
        .filter_map(|(name, status)| status.risk.as_ref().map(|risk| (name, status, risk)))
        .min_by(|a, b| a.2.minutes.total_cmp(&b.2.minutes))
    {
        let reset_minutes = risk.reset_at.saturating_sub(time) as f64 / 60.0;
        let (other_name, other_status) = other(name);
        let reset = clock(risk.reset_at, time, offset);
        // Opus spends the quota about 5/3 as fast as Sonnet; suggest it only with a clear margin.
        let detail = if status.opus && risk.minutes * 5.0 / 3.0 >= reset_minutes * 1.1 {
            t!(
                "На Sonnet вистачить до {}",
                "На Sonnet хватит до {}",
                "On Sonnet it lasts until {}",
                until(risk.reset_at, time, offset)
            )
        } else if free(other_status) {
            t!(
                "{other_name} вільний — переходь туди",
                "{other_name} свободен — переходи туда",
                "{other_name} is free — switch there"
            )
        } else if let Some(count) = risk.prompts {
            t!(
                "Це ≈ {count:.0} запитів, оновлення {reset}",
                "Это ≈ {count:.0} запросов, обновление {reset}",
                "That is ≈ {count:.0} prompts, resets {reset}"
            )
        } else {
            t!(
                "Квота оновиться {reset}",
                "Квота обновится {reset}",
                "Quota resets {reset}"
            )
        };
        return say(
            t!(
                "{name} закінчиться {}",
                "{name} закончится {}",
                "{name} runs out {}",
                clock(time + (risk.minutes * 60.0) as u64, time, offset)
            ),
            detail,
            Tone::Warn,
        );
    }

    if let Some(reset) = agents
        .into_iter()
        .filter_map(|(_, status)| status.lasting.map(|lasting| lasting.3))
        .min()
    {
        return say(
            s(
                "Все гаразд, працюй",
                "Всё хорошо, работай",
                "All good, keep working",
            )
            .into(),
            t!(
                "Квоти вистачить до {}",
                "Квоты хватит до {}",
                "Quota lasts until {}",
                until(reset, time, offset)
            ),
            Tone::Calm,
        );
    }
    if agents.into_iter().any(|(_, status)| status.used.is_some()) {
        return say(
            s("Ще придивляюся", "Ещё присматриваюсь", "Still watching").into(),
            s(
                "Прогноз буде за кілька запитів",
                "Прогноз будет через несколько запросов",
                "A forecast comes after a few prompts",
            )
            .into(),
            Tone::Unknown,
        );
    }
    say(
        s(
            "Поки нічого не бачу",
            "Пока ничего не вижу",
            "Nothing to see yet",
        )
        .into(),
        s(
            "Попрацюй у Claude Code чи Codex",
            "Поработай в Claude Code или Codex",
            "Work in Claude Code or Codex",
        )
        .into(),
        Tone::Unknown,
    )
}

/// How much of the remaining weekly quota fits into one day until the reset.
pub fn per_day(limit: Limit, time: u64) -> f64 {
    let days = limit.resets_at.saturating_sub(time).div_ceil(86_400).max(1);
    (100.0 - limit.used_percentage).max(0.0) / days as f64
}

pub fn countdown(reset: u64, now: u64) -> String {
    let seconds = reset.saturating_sub(now);
    if seconds < 60 {
        return t!("{seconds} с", "{seconds} с", "{seconds} s");
    }
    duration(seconds.div_ceil(60) as f64)
}

/// "о 18:40", "завтра о 09:00" or "пт о 12:00" in local time.
pub fn clock(at: u64, now: u64, offset: i64) -> String {
    moment(at, now, offset, true)
}

fn moment(at: u64, now: u64, offset: i64, preposition: bool) -> String {
    let local = |t: u64| t as i64 + offset;
    let (at_local, now_local) = (local(at), local(now));
    let minutes = at_local.rem_euclid(86_400) / 60;
    let hm = format!("{:02}:{:02}", minutes / 60, minutes % 60);
    let time = if preposition {
        t!("о {hm}", "в {hm}", "at {hm}")
    } else {
        hm
    };
    match at_local.div_euclid(86_400) - now_local.div_euclid(86_400) {
        0 => time,
        1 => t!("завтра {time}", "завтра {time}", "tomorrow {time}"),
        days if days < 8 => {
            // 1970-01-01 was a Thursday.
            let weekday = (at_local.div_euclid(86_400) + 3).rem_euclid(7) as usize;
            let name = match crate::i18n::lang() {
                crate::i18n::Lang::Uk => ["пн", "вт", "ср", "чт", "пт", "сб", "нд"],
                crate::i18n::Lang::Ru => ["пн", "вт", "ср", "чт", "пт", "сб", "вс"],
                crate::i18n::Lang::En => ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"],
            }[weekday];
            format!("{name} {time}")
        }
        _ => t!("через {}", "через {}", "in {}", countdown(at, now)),
    }
}

/// Seconds east of UTC right now.
// shortcut: ignores a DST switch between now and the shown time; upgrade if users report hour-off resets.
#[cfg(windows)]
pub fn utc_offset() -> i64 {
    use windows_sys::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
    let mut zone: TIME_ZONE_INFORMATION = unsafe { std::mem::zeroed() };
    let id = unsafe { GetTimeZoneInformation(&mut zone) };
    let bias = zone.Bias
        + if id == 2 {
            zone.DaylightBias
        } else {
            zone.StandardBias
        };
    -(bias as i64) * 60
}

#[cfg(not(windows))]
pub fn utc_offset() -> i64 {
    std::process::Command::new("date")
        .arg("+%z")
        .output()
        .ok()
        .and_then(|output| {
            let text = String::from_utf8(output.stdout).ok()?;
            let text = text.trim();
            let sign = if text.starts_with('-') { -1 } else { 1 };
            let digits = text.get(1..5)?;
            Some(
                sign * (digits[..2].parse::<i64>().ok()? * 3600
                    + digits[2..].parse::<i64>().ok()? * 60),
            )
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(tone: Tone) -> Status {
        Status {
            tone,
            used: Some(40.0),
            blocked: None,
            risk: None,
            lasting: None,
            opus: false,
        }
    }

    #[test]
    fn clock_reads_like_a_person() {
        let noon = 20_000 * 86_400 + 12 * 3600; // a Friday, UTC
        assert_eq!(clock(noon + 3 * 3600 + 40 * 60, noon, 0), "о 15:40");
        assert_eq!(clock(noon + 3 * 3600, noon, 2 * 3600), "о 17:00");
        assert_eq!(clock(noon + 86_400, noon, 0), "завтра о 12:00");
        assert_eq!(clock(noon + 3 * 86_400, noon, 0), "пн о 12:00");
    }

    fn risky(time: u64, minutes: f64, opus: bool) -> Status {
        Status {
            risk: Some(Risk {
                window: "5 год",
                minutes,
                reset_at: time + 120 * 60,
                prompts: None,
            }),
            opus,
            ..status(Tone::Warn)
        }
    }

    #[test]
    fn risk_names_the_end_and_one_way_out() {
        let time = 1_000_000;
        let calm = status(Tone::Calm);
        let said = speech(
            [("Claude", &risky(time, 100.0, true)), ("Codex", &calm)],
            time,
            0,
        );
        assert!(
            said.title.starts_with("Claude закінчиться о "),
            "{}",
            said.title
        );
        // 100 min on Opus is about 167 min on Sonnet: enough for 120 min with margin.
        assert!(said.detail.starts_with("На Sonnet вистачить до "));
        assert_eq!(said.tone, Tone::Warn);

        let said = speech(
            [("Claude", &risky(time, 100.0, false)), ("Codex", &calm)],
            time,
            0,
        );
        assert_eq!(said.detail, "Codex вільний — переходь туди");

        let busy = risky(time, 400.0, false);
        let said = speech(
            [("Claude", &risky(time, 100.0, false)), ("Codex", &busy)],
            time,
            0,
        );
        assert!(said.detail.starts_with("Квота оновиться о "));
    }

    #[test]
    fn blocked_agent_rests_and_the_free_one_works() {
        let time = 1_000_000;
        let blocked = Status {
            blocked: Some(("5 год", time + 3600)),
            ..status(Tone::Blocked)
        };
        let said = speech(
            [("Claude", &blocked), ("Codex", &status(Tone::Calm))],
            time,
            0,
        );
        assert!(
            said.title.starts_with("Claude на паузі до "),
            "{}",
            said.title
        );
        assert_eq!(said.detail, "Codex вільний — працюй там");
        let unknown = Status {
            used: None,
            ..status(Tone::Unknown)
        };
        let said = speech([("Claude", &blocked), ("Codex", &unknown)], time, 0);
        assert_eq!(said.detail, "Доступ повернеться через 1 год 00 хв");
        let said = speech([("Claude", &blocked), ("Codex", &blocked)], time, 0);
        assert_eq!(said.title, "Обидва на паузі");
    }

    #[test]
    fn calm_and_empty_states_stay_short() {
        let time = 1_000_000;
        let calm = Status {
            lasting: Some(("5 год", 300.0, None, time + 3600)),
            ..status(Tone::Calm)
        };
        let unknown = Status {
            used: None,
            ..status(Tone::Unknown)
        };
        let said = speech([("Claude", &calm), ("Codex", &unknown)], time, 0);
        assert_eq!(said.title, "Все гаразд, працюй");
        assert_eq!(said.tone, Tone::Calm);
        let said = speech([("Claude", &unknown), ("Codex", &unknown)], time, 0);
        assert_eq!(said.title, "Поки нічого не бачу");
    }

    #[test]
    fn countdown_rolls_hours_into_days() {
        assert_eq!(countdown(2 * 86400 + 23 * 3600 + 59 * 60, 0), "3 дн 0 год");
        assert_eq!(countdown(86400, 0), "1 дн 0 год");
        assert_eq!(countdown(86400 + 1, 0), "1 дн 1 год");
        assert_eq!(countdown(59, 0), "59 с");
    }

    #[test]
    fn weekly_budget_spreads_the_rest_over_days() {
        let limit = Limit {
            used_percentage: 30.0,
            resets_at: 5 * 86_400,
        };
        assert!((per_day(limit, 0) - 14.0).abs() < 1e-9);
    }
}
