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
}

pub struct Status {
    pub tone: Tone,
    /// Highest current usage over both windows; None when nothing is known.
    pub used: Option<f64>,
    pub blocked: Option<(&'static str, u64)>,
    pub risk: Option<Risk>,
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
            })
        })
        .min_by(|a, b| a.minutes.total_cmp(&b.minutes))
        .filter(|_| blocked.is_none());
    let known = forecasts
        .iter()
        .all(|(_, forecast)| forecast.as_ref().is_some_and(|f| f.minutes.is_some()));
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
    let tone = if blocked.is_some() {
        Tone::Blocked
    } else if risk.is_some() {
        Tone::Warn
    } else if blocked.is_none() && known {
        Tone::Calm
    } else {
        Tone::Unknown
    };
    Status {
        tone,
        used,
        blocked,
        risk,
    }
}

/// "18:50", "завтра 09:00" or "пт 12:00": the clock without the preposition.
pub fn until(at: u64, time: u64, offset: i64) -> String {
    moment(at, time, offset, false)
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

    #[test]
    fn clock_reads_like_a_person() {
        let noon = 20_000 * 86_400 + 12 * 3600; // a Friday, UTC
        assert_eq!(clock(noon + 3 * 3600 + 40 * 60, noon, 0), "о 15:40");
        assert_eq!(clock(noon + 3 * 3600, noon, 2 * 3600), "о 17:00");
        assert_eq!(clock(noon + 86_400, noon, 0), "завтра о 12:00");
        assert_eq!(clock(noon + 3 * 86_400, noon, 0), "пн о 12:00");
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
