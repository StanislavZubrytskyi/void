//! Independent background sources. The UI receives snapshots without performing I/O.
use crate::{
    claude, codex, estimate,
    usage::{self, Record},
};
use std::{
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};
pub enum Update {
    Claude(Vec<Record>, Vec<Record>),
    ClaudeAccount(Result<Record, claude::Failure>),
    Codex(Vec<Record>, bool),
    /// Busy sessions: Claude, then Codex.
    Activity([Vec<crate::activity::Session>; 2]),
    /// An agent (0 Claude, 1 Codex) just finished a turn.
    Finished(usize),
    /// Recent project folders per agent, newest first: choices for a new task.
    Projects([Vec<crate::projects::Project>; 2]),
    /// Tokens an agent spent, per minute, over the last eight days.
    Tokens(usize, Vec<(u64, u64)>),
}
pub fn start(ctx: eframe::egui::Context) -> Receiver<Update> {
    let (send, receive) = mpsc::sync_channel(8);
    let busy = send.clone();
    let repaint = ctx.clone();
    thread::spawn(move || {
        let mut last: Option<[Vec<crate::activity::Session>; 2]> = None;
        let mut recorded = std::collections::HashSet::new();
        let mut listed = None::<Instant>;
        let mut deepened = std::collections::HashSet::new();
        loop {
            if listed.is_none_or(|at| at.elapsed() >= Duration::from_secs(30)) {
                listed = Some(Instant::now());
                let found = [0, 1].map(crate::projects::list);
                if busy.send(Update::Projects(found)).is_err() {
                    break;
                }
            }
            let mut sessions = crate::activity::scan();
            // A turn already long when first seen lost its prompt from the tail: look further
            // back once, which also finds its true start and token count.
            for (agent, list) in sessions.iter_mut().enumerate() {
                for session in list.iter_mut().filter(|s| s.prompt.is_none()) {
                    if deepened.insert(session.id.clone())
                        && let Some(deep) = crate::activity::read_turn_deep(agent, &session.id)
                            .filter(|deep| deep.busy && deep.prompt.is_some())
                    {
                        session.since = session.since.min(deep.since);
                        session.prompt = deep.prompt;
                        session.tokens = session.tokens.max(deep.tokens);
                    }
                }
            }
            // A long turn scrolls out of the transcript tail: keep what we saw earlier.
            if let Some(previous) = &last {
                for (now, before) in sessions.iter_mut().zip(previous) {
                    for session in now {
                        if let Some(seen) = before.iter().find(|seen| seen.id == session.id) {
                            session.since = session.since.min(seen.since);
                            if session.prompt.is_none() {
                                session.prompt.clone_from(&seen.prompt);
                            }
                            // Responses scrolled out of the tail still count.
                            session.tokens = session.tokens.max(seen.tokens);
                        }
                    }
                }
                // A session that stopped being busy finished its turn: remember how it went.
                // shortcut: turns that finish while Void is closed are not recorded.
                let mut finished = Vec::new();
                for (agent, before) in previous.iter().enumerate() {
                    for gone in before
                        .iter()
                        .filter(|seen| !sessions[agent].iter().any(|s| s.id == seen.id))
                    {
                        if let Some(turn) = crate::activity::read_turn(agent, &gone.id)
                            .filter(|turn| !turn.busy && turn.end >= gone.since)
                            && recorded.insert((gone.id.clone(), gone.since))
                        {
                            finished.push(agent);
                            let _ = crate::turns::save(&crate::turns::Turn {
                                agent,
                                project: turn.project,
                                start: gone.since,
                                end: turn.end,
                                prompt: turn.prompt.or_else(|| gone.prompt.clone()),
                            })
                            .is_ok();
                        }
                    }
                }
                if finished
                    .into_iter()
                    .any(|agent| busy.send(Update::Finished(agent)).is_err())
                {
                    break;
                }
            }
            if last.as_ref() != Some(&sessions) {
                if busy.send(Update::Activity(sessions.clone())).is_err() {
                    break;
                }
                repaint.request_repaint();
                last = Some(sessions);
            }
            thread::sleep(Duration::from_secs(2));
        }
    });
    let local = send.clone();
    let repaint = ctx.clone();
    thread::spawn(move || {
        let mut estimator = estimate::Estimator::default();
        let mut estimates = Vec::new();
        let mut refreshed = None::<Instant>;
        let mut history = Vec::new();
        loop {
            if refreshed.is_none_or(|at| at.elapsed() >= Duration::from_secs(30)) {
                estimates = estimator.confirmed_blocks(usage::now());
                refreshed = Some(Instant::now());
                if local
                    .send(Update::Tokens(0, estimator.token_minutes(usage::now())))
                    .is_err()
                {
                    break;
                }
            }
            if let Ok(rows) = usage::history() {
                history = rows;
            }
            let mut combined = history.clone();
            combined.extend(claude::desktop_history());
            combined.sort_by_key(|r| r.time);
            if local
                .send(Update::Claude(combined, estimates.clone()))
                .is_err()
            {
                break;
            }
            repaint.request_repaint();
            thread::sleep(Duration::from_secs(2));
        }
    });
    let account = send.clone();
    let repaint = ctx.clone();
    thread::spawn(move || {
        loop {
            let result = claude::account_sample().map(|sample| {
                let _ = usage::save(sample.clone());
                Record {
                    time: usage::now(),
                    sample,
                }
            });
            let delay = if result.is_ok() { 30 } else { 60 };
            if account.send(Update::ClaudeAccount(result)).is_err() {
                break;
            }
            repaint.request_repaint();
            thread::sleep(Duration::from_secs(delay));
        }
    });
    thread::spawn(move || {
        let mut connection = None;
        let mut last_poll = None::<Instant>;
        let mut snapshots = Vec::new();
        let mut online = false;
        let mut ledger = codex::TokenLedger::default();
        let mut spent = None::<Instant>;
        loop {
            if last_poll.is_none_or(|at| at.elapsed() >= Duration::from_secs(30)) {
                if connection.is_none() {
                    connection = codex::Connection::open();
                }
                let snapshot = connection.as_mut().and_then(|client| client.snapshot());
                online = snapshot.is_some();
                if let Some(row) = snapshot {
                    snapshots.push(row);
                } else {
                    connection = None;
                }
                last_poll = Some(Instant::now());
            }
            if let Some(row) = connection.as_ref().and_then(|client| client.updates()) {
                snapshots.push(row);
                online = true;
            }
            snapshots.retain(|row| usage::now().saturating_sub(row.time) <= 1800);
            if spent.is_none_or(|at: Instant| at.elapsed() >= Duration::from_secs(30)) {
                spent = Some(Instant::now());
                if send
                    .send(Update::Tokens(1, ledger.refresh(usage::now())))
                    .is_err()
                {
                    break;
                }
            }
            let mut rows = codex::history();
            rows.extend(snapshots.iter().cloned());
            rows.sort_by_key(|row| row.time);
            if send.send(Update::Codex(rows, online)).is_err() {
                break;
            }
            ctx.request_repaint();
            thread::sleep(Duration::from_secs(2));
        }
    });
    receive
}
