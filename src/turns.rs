//! Finished agent turns and what they predict about the next ones.
//!
//! Each turn is one local JSON line in `~/.void/turns.jsonl`: agent, project folder, start, end
//! and the prompt text (unless `VOID_NO_PROMPTS` is set). Quota cost is not stored: it is read
//! from quota measurements around the turn, so it improves as measurements do.

use crate::{
    advice::Status,
    i18n::s,
    usage::{self, Record, median},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    error::Error,
    fs::File,
    io::{self, BufRead, BufReader},
    path::PathBuf,
};

/// Turns older than this no longer describe how the user works.
const KEEP: u64 = 30 * 86_400;
/// How far a quota measurement may sit from a turn edge and still describe it.
const NEAR: u64 = 30 * 60;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Turn {
    /// 0 is Claude, 1 is Codex.
    pub agent: usize,
    pub project: String,
    pub start: u64,
    pub end: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

fn path() -> io::Result<PathBuf> {
    Ok(usage::data_dir()?.join("turns.jsonl"))
}

pub fn save(turn: &Turn) -> Result<(), Box<dyn Error>> {
    let mut turn = turn.clone();
    if std::env::var_os("VOID_NO_PROMPTS").is_some() {
        turn.prompt = None;
    }
    usage::append_line(&serde_json::to_vec(&turn)?, &path()?)
}

pub fn load() -> Vec<Turn> {
    let Ok(file) = path().and_then(File::open) else {
        return Vec::new();
    };
    if file.lock_shared().is_err() {
        return Vec::new();
    }
    let cutoff = usage::now().saturating_sub(KEEP);
    BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str::<Turn>(&line).ok())
        .filter(|turn| turn.agent < 2 && turn.end >= turn.start && turn.start >= cutoff)
        .collect()
}

/// Five-hour quota spent during a turn: the measurement just before it against the first one
/// at its end, in the same window. None when either side is missing.
// shortcut: parallel sessions of one agent share the delta; upgrade by splitting overlapping turns.
pub fn cost(records: &[Record], start: u64, end: u64) -> Option<f64> {
    let five = |record: &Record| record.sample.rate_limits.as_ref()?.five_hour;
    let before = records
        .iter()
        .rev()
        .filter(|r| r.time <= start && start - r.time <= NEAR)
        .find_map(five)?;
    // The agent's own last measurement can land a moment before the turn's final line.
    let after = records
        .iter()
        .filter(|r| r.time + 120 >= end && r.time <= end + NEAR)
        .find_map(five)?;
    (before.resets_at.abs_diff(after.resets_at) <= 60
        && after.used_percentage >= before.used_percentage)
        .then_some(after.used_percentage - before.used_percentage)
}

fn words(text: &str) -> HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| word.chars().count() >= 3)
        .map(str::to_lowercase)
        .collect()
}

fn similarity(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    let union = a.union(b).count();
    if union == 0 {
        0.0
    } else {
        a.intersection(b).count() as f64 / union as f64
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Prediction {
    /// Typical length of such a turn.
    pub minutes: f64,
    /// Typical five-hour quota it spends, in percentage points.
    pub cost: Option<f64>,
    /// Built from turns with a similar prompt, not just this project's average.
    pub similar: bool,
}

/// How long and how expensive this turn is likely to be: from past turns with a similar prompt,
/// else from this project's recent turns, else from the agent's recent turns.
pub fn predict(
    turns: &[Turn],
    agent: usize,
    project: &str,
    prompt: Option<&str>,
    records: &[Record],
) -> Option<Prediction> {
    let own: Vec<&Turn> = turns.iter().filter(|turn| turn.agent == agent).collect();
    let asked = prompt.map(words).unwrap_or_default();
    let mut scored: Vec<(f64, &Turn)> = own
        .iter()
        .filter(|_| !asked.is_empty())
        .filter_map(|turn| {
            let score = similarity(&asked, &words(turn.prompt.as_deref()?))
                + if turn.project == project { 0.1 } else { 0.0 };
            (score >= 0.25).then_some((score, *turn))
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    let similar = scored.len() >= 2;
    let basis: Vec<&Turn> = if similar {
        scored.into_iter().take(5).map(|(_, turn)| turn).collect()
    } else {
        let here: Vec<&Turn> = own
            .iter()
            .rev()
            .filter(|turn| turn.project == project)
            .take(20)
            .copied()
            .collect();
        if here.len() >= 3 {
            here
        } else {
            own.iter().rev().take(20).copied().collect()
        }
    };
    if basis.len() < 2 || (!similar && basis.len() < 3) {
        return None;
    }
    let minutes = median(
        basis
            .iter()
            .map(|turn| (turn.end - turn.start) as f64 / 60.0)
            .collect(),
    )?;
    let costs: Vec<f64> = basis
        .iter()
        .filter_map(|turn| cost(records, turn.start, turn.end))
        .collect();
    let cost = if costs.len() >= 2 {
        median(costs)
    } else {
        None
    };
    Some(Prediction {
        minutes,
        cost,
        similar,
    })
}

/// "схожі: ще ≈ 6 хв · ≈ 4% квоти" for a turn that has run `elapsed` minutes so far.
pub fn describe(prediction: &Prediction, elapsed: f64) -> String {
    let left = prediction.minutes - elapsed;
    let basis = if prediction.similar {
        s("схожі: ", "похожие: ", "similar: ")
    } else {
        s("зазвичай: ", "обычно: ", "usually: ")
    };
    let time = if left >= 1.0 {
        let rest = crate::duration(left);
        t!(
            "{basis}ще ≈ {rest}",
            "{basis}ещё ≈ {rest}",
            "{basis}≈ {rest} more"
        )
    } else {
        // Past the typical length: say so once, with what typical was.
        let typical = crate::duration(prediction.minutes.max(1.0));
        t!(
            "довше, ніж зазвичай (≈ {typical})",
            "дольше обычного (≈ {typical})",
            "longer than usual (≈ {typical})"
        )
    };
    let cost = prediction
        .cost
        .filter(|cost| *cost >= 0.5)
        .map(|cost| {
            t!(
                " · ≈ {cost:.0}% квоти",
                " · ≈ {cost:.0}% квоты",
                " · ≈ {cost:.0}% of quota"
            )
        })
        .unwrap_or_default();
    format!("{time}{cost}")
}

/// How many typical turns the agent's five-hour quota still holds.
pub fn turns_left(turns: &[Turn], agent: usize, records: &[Record], now: u64) -> Option<f64> {
    let costs: Vec<f64> = turns
        .iter()
        .rev()
        .filter(|turn| turn.agent == agent)
        .take(20)
        .filter_map(|turn| cost(records, turn.start, turn.end))
        .collect();
    if costs.len() < 3 {
        return None;
    }
    let per_turn = median(costs).filter(|cost| *cost > 0.0)?;
    let (limit, seen) = usage::last_window(records, |limits| limits.five_hour)?;
    (limit.resets_at > now && now.saturating_sub(seen) <= NEAR)
        .then(|| ((100.0 - limit.used_percentage).max(0.0) / per_turn).floor())
}

/// One line on where the next task should go.
pub fn hint(agent: usize, left: [Option<f64>; 2], statuses: [&Status; 2]) -> Option<String> {
    const NAMES: [&str; 2] = ["Claude", "Codex"];
    let other = 1 - agent;
    let free = |status: &Status| {
        status.blocked.is_none() && status.risk.is_none() && status.used.is_some()
    };
    let here = left[agent].map(|n| {
        t!(
            "≈ {n:.0} таких ходів до ліміту 5 год",
            "≈ {n:.0} таких ходов до лимита 5 ч",
            "≈ {n:.0} such turns before the 5 h limit"
        )
    });
    if !free(statuses[other]) {
        return here;
    }
    let there = left[other]
        .map(|n| t!(" ≈ {n:.0} ходів", " ≈ {n:.0} ходов", " ≈ {n:.0} turns"))
        .unwrap_or_default();
    if statuses[agent].blocked.is_some() || statuses[agent].risk.is_some() {
        return Some(t!(
            "Наступне — в {}: там запас{there}",
            "Следующее — в {}: там запас{there}",
            "Next one in {}: it has room{there}",
            NAMES[other]
        ));
    }
    match (left[agent], left[other]) {
        (Some(a), Some(b)) if a < 10.0 && b >= 2.0 * a => Some(t!(
            "Велике — краще в {}: ≈ {b:.0} ходів проти {a:.0}",
            "Большое — лучше в {}: ≈ {b:.0} ходов против {a:.0}",
            "Big task — better in {}: ≈ {b:.0} turns vs {a:.0}",
            NAMES[other]
        )),
        _ => here,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(time: u64, used: f64) -> Record {
        serde_json::from_value(serde_json::json!({
            "time": time, "session_id": "s",
            "rate_limits": { "five_hour": { "used_percentage": used, "resets_at": 10_000 } }
        }))
        .unwrap()
    }

    fn turn(start: u64, minutes: u64, project: &str, prompt: &str) -> Turn {
        Turn {
            agent: 0,
            project: project.into(),
            start,
            end: start + minutes * 60,
            prompt: Some(prompt.into()),
        }
    }

    #[test]
    fn cost_reads_the_quota_around_a_turn() {
        let records = [record(1000, 10.0), record(1600, 14.0)];
        assert_eq!(cost(&records, 1100, 1550), Some(4.0));
        // Nothing measured after the turn.
        assert_eq!(cost(&records, 1700, 1800), None);
    }

    #[test]
    fn similar_prompts_beat_the_project_average() {
        let turns = vec![
            turn(100, 30, "site", "fix the checkout payment form validation"),
            turn(
                5000,
                32,
                "site",
                "checkout payment form validation broken again",
            ),
            turn(9000, 2, "site", "rename a variable"),
            turn(9500, 3, "site", "typo in footer"),
            turn(9900, 2, "site", "update copyright year"),
        ];
        let similar = predict(
            &turns,
            0,
            "site",
            Some("payment form validation fails"),
            &[],
        )
        .unwrap();
        assert!(similar.similar);
        assert_eq!(similar.minutes, 31.0);
        let usual = predict(&turns, 0, "site", Some("make the logo bigger"), &[]).unwrap();
        assert!(!usual.similar);
        assert_eq!(usual.minutes, 3.0);
        assert!(predict(&turns[..2], 0, "site", None, &[]).is_none());
        assert_eq!(describe(&similar, 10.0), "схожі: ще ≈ 21 хв");
        assert_eq!(describe(&usual, 5.0), "довше, ніж зазвичай (≈ 3 хв)");
    }

    #[test]
    fn hint_sends_big_work_where_the_quota_is() {
        let status = |used: Option<f64>| Status {
            tone: crate::advice::Tone::Calm,
            used,
            blocked: None,
            risk: None,
            lasting: None,
            opus: false,
        };
        let (a, b) = (status(Some(80.0)), status(Some(10.0)));
        assert_eq!(
            hint(0, [Some(4.0), Some(30.0)], [&a, &b]).as_deref(),
            Some("Велике — краще в Codex: ≈ 30 ходів проти 4")
        );
        assert_eq!(
            hint(0, [Some(20.0), Some(30.0)], [&a, &b]).as_deref(),
            Some("≈ 20 таких ходів до ліміту 5 год")
        );
        let blocked = Status {
            blocked: Some(("5 год", 99)),
            ..status(Some(100.0))
        };
        assert_eq!(
            hint(0, [None, Some(30.0)], [&blocked, &b]).as_deref(),
            Some("Наступне — в Codex: там запас ≈ 30 ходів")
        );
        assert_eq!(
            hint(1, [None, Some(5.0)], [&blocked, &b]).as_deref(),
            Some("≈ 5 таких ходів до ліміту 5 год")
        );
    }
}
