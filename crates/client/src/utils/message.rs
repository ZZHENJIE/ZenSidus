//! The in-app message center: a bounded buffer of user-facing messages
//! (shown in the main window's status bar) that any code can push to.

use std::collections::VecDeque;

use chrono::Local;
use gpui_kit::{App, BorrowAppContext as _, Global, SharedString};

/// Buffer capacity; the oldest entry is dropped beyond it, keeping memory
/// bounded no matter how chatty the app gets.
const CAPACITY: usize = 200;

/// Severity of a user-visible message; also picks the status-bar dot color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Success,
    Warning,
    Error,
}

/// One user-visible message.
#[derive(Clone, Debug)]
pub struct Message {
    pub level: Level,
    /// Formatted at push time: rendering never pays for formatting and the
    /// history order stays stable even across locale changes.
    pub time: SharedString,
    pub text: SharedString,
}

/// Bounded in-memory message buffer, registered as a `Global`.
///
/// `update_global` automatically notifies `observe_global::<MessageLog>`
/// subscribers (the lease end pushes a NotifyGlobalObservers effect), so the
/// status bar re-renders on every push without extra bookkeeping.
pub struct MessageLog {
    messages: VecDeque<Message>,
}

impl Global for MessageLog {}

impl MessageLog {
    /// Register the global. Call once during startup, before any window.
    pub fn init(cx: &mut App) {
        cx.set_global(Self {
            messages: VecDeque::with_capacity(CAPACITY),
        });
    }

    /// Latest message, for the status bar (newest last).
    pub fn last(&self) -> Option<&Message> {
        self.messages.back()
    }

    /// Newest-first snapshot for the history popover.
    pub fn latest_first(&self) -> Vec<&Message> {
        self.messages.iter().rev().collect()
    }

    /// Whether nothing has been pushed yet.
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }
}

/// Error message: full detail to tracing, friendly text to the buffer.
pub fn error(message: impl Into<SharedString>, detail: impl std::fmt::Display, cx: &mut App) {
    tracing::error!("{detail:#}");
    push(Level::Error, message.into(), cx);
}

/// Warning message: full detail to tracing, friendly text to the buffer.
pub fn warning(message: impl Into<SharedString>, detail: impl std::fmt::Display, cx: &mut App) {
    tracing::warn!("{detail:#}");
    push(Level::Warning, message.into(), cx);
}

/// Info message: buffer only, no log output.
pub fn info(message: impl Into<SharedString>, cx: &mut App) {
    push(Level::Info, message.into(), cx);
}

/// Success message: buffer only, no log output.
pub fn success(message: impl Into<SharedString>, cx: &mut App) {
    push(Level::Success, message.into(), cx);
}

/// Drop the whole history.
pub fn clear(cx: &mut App) {
    cx.update_global::<MessageLog, _>(|log, _| log.messages.clear());
}

fn push(level: Level, text: SharedString, cx: &mut App) {
    let time = Local::now().format("%m-%d %H:%M:%S").to_string();
    cx.update_global::<MessageLog, _>(|log, _| {
        if log.messages.len() == CAPACITY {
            log.messages.pop_front();
        }
        log.messages.push_back(Message {
            level,
            time: time.into(),
            text,
        });
    });
}
