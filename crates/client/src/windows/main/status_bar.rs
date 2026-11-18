//! Bottom bar of the main window: the latest message on the left (click for
//! the full history), the Alpaca connection dot on the right.

use gpui_kit::{
    Anchor, App, Context, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    ReadGlobal as _, ScrollHandle, StatefulInteractiveElement as _, Styled as _,
    base::{h_flex, v_flex},
    component::{
        ActiveTheme as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        popover::Popover,
        scroll::Scrollbar,
        status_bar::StatusBar,
    },
    div, px,
};

use crate::tr;
use crate::utils::message::{Level, MessageLog};
use crate::utils::state::AppState;
use crate::windows::MainWindow;

impl MainWindow {
    /// Bottom bar of the main window: latest message on the left (click for
    /// the full history), live Alpaca connection status on the right. The
    /// container (background, top border, text scale, region alignment) is
    /// the shared [`StatusBar`] component.
    pub(super) fn render_status_bar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        // Left: the latest message. Nothing renders until one arrives, so
        // a quiet bar shows no placeholder at all.
        let mut bar = StatusBar::new();
        if MessageLog::global(cx).last().is_some() {
            bar = bar.left(
                div()
                    .min_w_0()
                    .child(render_message_area(&self.message_scroll, cx)),
            );
        }
        // Right: Alpaca connection status, pinned to the edge.
        bar.right(render_alpaca_status(cx))
    }
}

/// Latest message with its severity dot and time; clicking opens a popover
/// with the whole history and a clear action.
fn render_message_area(scroll: &ScrollHandle, cx: &mut App) -> impl IntoElement {
    let latest = MessageLog::global(cx)
        .last()
        .cloned()
        .expect("message area rendered without a message");
    let scroll = scroll.clone();

    let trigger = Button::new("message-trigger")
        .ghost()
        .xsmall()
        .child(
            div()
                .size(px(6.))
                .rounded_full()
                .flex_shrink_0()
                .bg(level_color(latest.level, cx)),
        )
        .child(div().max_w(px(520.)).truncate().child(latest.text.clone()))
        .child(
            div()
                .flex_shrink_0()
                .text_color(cx.theme().muted_foreground)
                .child(latest.time.clone()),
        );

    Popover::new("message-history")
        .trigger(trigger)
        .anchor(Anchor::BottomLeft)
        .content(move |_state, _window, cx| {
            let messages = MessageLog::global(cx).latest_first();
            let mut column = v_flex()
                .min_w(px(300.))
                .max_w(px(480.))
                .text_sm()
                .gap_1()
                .child(
                    h_flex()
                        .justify_between()
                        .child(
                            div()
                                .text_color(cx.theme().muted_foreground)
                                .child(tr!("app.messages.title")),
                        )
                        .child(
                            Button::new("message-clear")
                                .ghost()
                                .xsmall()
                                .label(tr!("app.messages.clear"))
                                .on_click(|_, _, cx| crate::utils::message::clear(cx)),
                        ),
                );
            if messages.is_empty() {
                column = column.child(
                    div()
                        .py_1()
                        .text_color(cx.theme().muted_foreground)
                        .child(tr!("app.messages.empty")),
                );
            } else {
                // The list keeps its own `max_h` so the scroll area has a
                // definite bound inside the auto-sized popover; the scrollbar
                // is a sibling overlay driven by the same handle.
                column = column.child(
                    div()
                        .relative()
                        .child(
                            div()
                                .id("message-list")
                                .max_h(px(320.))
                                .overflow_y_scroll()
                                .track_scroll(&scroll)
                                .flex_col()
                                .gap_1()
                                .children(messages.into_iter().map(|message| {
                                    h_flex()
                                        .gap_2()
                                        .child(
                                            div()
                                                .size(px(6.))
                                                .mt_1()
                                                .rounded_full()
                                                .flex_shrink_0()
                                                .bg(level_color(message.level, cx)),
                                        )
                                        .child(
                                            div()
                                                .flex_shrink_0()
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(message.time.clone()),
                                        )
                                        .child(div().child(message.text.clone()))
                                })),
                        )
                        .child(Scrollbar::vertical(&scroll)),
                );
            }
            column
        })
}

/// Severity dot for the current hub connection. The dot alone: failed /
/// connected already land in the message area on the left, so repeating the
/// text here would be duplication — the full status lives on the tooltip,
/// with the upstream reason appended when the connection failed.
fn render_alpaca_status(cx: &mut App) -> impl IntoElement {
    let status = cx.global::<AppState>().relay_hub.read(cx).status().clone();
    let mut tooltip = status.label();
    if let Some(reason) = status.reason().filter(|reason| !reason.is_empty()) {
        tooltip = format!("{tooltip} — {reason}").into();
    }
    Button::new("alpaca-status")
        .ghost()
        .xsmall()
        .label("Alpaca")
        .child(div().size(px(6.)).rounded_full().bg(status.color(cx)))
        .tooltip(tooltip)
}

fn level_color(level: Level, cx: &App) -> Hsla {
    let theme = cx.theme();
    match level {
        Level::Info => theme.info,
        Level::Success => theme.success,
        Level::Warning => theme.warning,
        Level::Error => theme.danger,
    }
}
