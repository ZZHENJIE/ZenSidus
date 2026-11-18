//! Settings → Logs: browse the daily log files in-app (tail view) or open
//! one with the OS default program.

use chrono::NaiveDate;
use gpui_kit::{
    App, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Styled, Window,
    assets::IconName,
    base::{h_flex, v_flex},
    component::{
        ActiveTheme as _, Disableable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        setting::{SettingGroup, SettingItem, SettingPage},
    },
    div,
    prelude::FluentBuilder as _,
    px,
};

use crate::tr;
use crate::utils::{AppIcon, logging, message};
use crate::windows::settings::SettingsWindow;

/// The viewer shows the tail of the file; the full content stays on disk and
/// is reachable through "Open file".
const LOG_VIEW_MAX_LINES: usize = 500;

/// State of the Logs page. Lives in its own entity so the header controls and
/// the viewer's render closures (which only get `&mut App`) can update it and
/// re-trigger the settings window through the `observe` subscription.
pub(super) struct LogsState {
    days_loaded: bool,
    days: Vec<NaiveDate>,
    selected: Option<NaiveDate>,
    /// Which day the cached `content`/`tail`/`error` were loaded for.
    loaded_for: Option<NaiveDate>,
    content: Option<String>,
    /// The truncated tail shown by the viewer, computed once at load time —
    /// counting/joining a multi-megabyte log on every render would stall the
    /// settings window (which renders on every global notification).
    tail: Option<String>,
    tail_truncated: bool,
    error: Option<String>,
}

impl LogsState {
    pub(super) fn new() -> Self {
        Self {
            days_loaded: false,
            days: Vec::new(),
            selected: None,
            loaded_for: None,
            content: None,
            tail: None,
            tail_truncated: false,
            error: None,
        }
    }

    /// Lazily refresh the day list and (re)load the selected file. Called on
    /// every render; both steps are no-ops when the cache is current.
    pub(super) fn prepare(&mut self) {
        if !self.days_loaded {
            self.refresh_days();
        }
        self.ensure_loaded();
    }

    /// List available days (newest first). Keeps the selection when it still
    /// exists, otherwise falls back to the newest day.
    fn refresh_days(&mut self) {
        self.days = list_log_days();
        if let Some(selected) = self.selected
            && !self.days.contains(&selected)
        {
            self.selected = None;
        }
        if self.selected.is_none() {
            self.selected = self.days.first().copied();
        }
        self.loaded_for = None;
        self.days_loaded = true;
    }

    /// Move the selection `delta` positions in the (newest-first) day list.
    fn step(&mut self, delta: isize) {
        let Some(selected) = self.selected else {
            return;
        };
        let Some(ix) = self.days.iter().position(|&day| day == selected) else {
            return;
        };
        let next = ix as isize + delta;
        if next >= 0 && (next as usize) < self.days.len() {
            self.selected = Some(self.days[next as usize]);
        }
    }

    /// (Re)load the selected day's file when the cache is stale.
    fn ensure_loaded(&mut self) {
        if self.loaded_for == self.selected {
            return;
        }
        self.loaded_for = self.selected;
        self.content = None;
        self.tail = None;
        self.tail_truncated = false;
        self.error = None;
        if let Some(day) = self.selected {
            match logging::read_day_log(day) {
                Ok(Some(content)) => {
                    let total = content.lines().count();
                    self.tail_truncated = total > LOG_VIEW_MAX_LINES;
                    self.tail = Some(if self.tail_truncated {
                        content
                            .lines()
                            .skip(total - LOG_VIEW_MAX_LINES)
                            .collect::<Vec<_>>()
                            .join("\n")
                    } else {
                        content.clone()
                    });
                    self.content = Some(content);
                }
                Ok(None) => {}
                Err(err) => self.error = Some(format!("{err:#}")),
            }
        }
    }
}

fn list_log_days() -> Vec<NaiveDate> {
    let Ok(dir) = logging::log_dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut days: Vec<NaiveDate> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            NaiveDate::parse_from_str(name.trim_end_matches(".log"), "%Y-%m-%d").ok()
        })
        .collect();
    days.sort_unstable();
    days.reverse();
    days
}

/// Day navigation, refresh and open-file controls. Rendered as the page
/// title's suffix — the header row above the description — so the content
/// area holds nothing but the log body.
fn log_toolbar(logs: &Entity<LogsState>, cx: &App) -> impl IntoElement {
    let state = logs.read(cx);
    let index = state
        .selected
        .and_then(|day| state.days.iter().position(|&day_| day_ == day));
    let date_label = state
        .selected
        .map(|day| day.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "-".to_string());
    let has_selection = index.is_some();
    let prev_disabled = index.is_none_or(|ix| ix + 1 >= state.days.len());
    let next_disabled = index.is_none_or(|ix| ix == 0);

    let step_by = {
        let logs = logs.clone();
        move |delta: isize| {
            let logs = logs.clone();
            move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut App| {
                logs.update(cx, |state, cx| {
                    state.step(delta);
                    state.ensure_loaded();
                    cx.notify();
                });
            }
        }
    };

    let refresh = {
        let logs = logs.clone();
        move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut App| {
            logs.update(cx, |state, cx| {
                state.refresh_days();
                state.ensure_loaded();
                cx.notify();
            });
        }
    };

    let open_file = {
        let logs = logs.clone();
        move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut App| {
            let Some(day) = logs.read(cx).selected else {
                return;
            };
            if let Err(err) = logging::open_day_log(day) {
                message::error(tr!("app.notify.open_log_failed"), err, cx);
            }
        }
    };

    h_flex()
        .gap_1()
        .child(
            Button::new("log-older")
                .ghost()
                .small()
                .icon(IconName::ArrowLeft)
                .disabled(!has_selection || prev_disabled)
                .on_click(step_by(1)),
        )
        .child(div().px_2().child(date_label))
        .child(
            Button::new("log-newer")
                .ghost()
                .small()
                .icon(IconName::ArrowRight)
                .disabled(!has_selection || next_disabled)
                .on_click(step_by(-1)),
        )
        .child(div().flex_1())
        .child(
            Button::new("log-refresh")
                .small()
                .label(tr!("app.settings.refresh"))
                .on_click(refresh),
        )
        .child(
            Button::new("log-open")
                .small()
                .label(tr!("app.settings.open_file"))
                .disabled(state.selected.is_none())
                .on_click(open_file),
        )
}

impl SettingsWindow {
    pub(super) fn logs_page(&self) -> SettingPage {
        let viewer = SettingItem::render({
            let logs = self.logs.clone();
            move |_, _, cx| {
                let state = logs.read(cx);
                if state.error.is_some() {
                    return div()
                        .text_color(cx.theme().danger)
                        .child(tr!("app.settings.logs_error"))
                        .into_any_element();
                }
                let Some(body) = &state.tail else {
                    return div()
                        .text_color(cx.theme().muted_foreground)
                        .child(tr!("app.settings.logs_empty"))
                        .into_any_element();
                };

                v_flex()
                    .gap_1()
                    .when(state.tail_truncated, |view| {
                        view.child(div().text_color(cx.theme().muted_foreground).child(tr!(
                            "app.settings.logs_truncated",
                            count = LOG_VIEW_MAX_LINES
                        )))
                    })
                    .child(
                        div()
                            .id("log-viewer")
                            .font_family("Consolas")
                            .text_size(px(12.))
                            .child(body.clone()),
                    )
                    .into_any_element()
            }
        });

        super::settings_page("app.settings.logs", AppIcon::Log)
            .title_suffix({
                let logs = self.logs.clone();
                move |_, cx| log_toolbar(&logs, cx).into_any_element()
            })
            .description(tr!("app.settings.logs_desc"))
            .group(SettingGroup::new().item(viewer))
    }
}
