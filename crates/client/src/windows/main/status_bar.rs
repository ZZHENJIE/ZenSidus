//! Bottom bar of the main window: the latest message on the left (click for
//! the full history), the server connection dot on the right (click for the
//! connection popover: status, TCP latency, server address, connect /
//! disconnect).

use gpui_kit::{
    Anchor, App, Context, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    ReadGlobal as _, ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled as _,
    base::{h_flex, v_flex},
    component::{
        ActiveTheme as _, Disableable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        popover::Popover,
        scroll::Scrollbar,
        status_bar::StatusBar,
    },
    div, px,
};

use crate::tr;
use crate::utils::message::{Level, MessageLog};
use crate::utils::state::{AppState, ServerStatus};
use crate::widgets::dialog::open_auth_code_dialog;
use crate::windows::MainWindow;

impl MainWindow {
    /// Bottom bar of the main window: latest message on the left (click for
    /// the full history), live server connection status on the right. The
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
        // Right: server connection status, pinned to the edge.
        bar.right(render_connection_status(cx))
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

/// "服务器" label + severity dot for the current server connection. The
/// tooltip carries the live status label, with the upstream reason appended
/// when the connection failed (failed / connected events also land in the
/// message area on the left). Clicking opens the connection popover: live
/// status, TCP latency (the heartbeat round-trip), the configured server
/// address and the connect / disconnect actions (same semantics as the
/// Server settings page).
fn render_connection_status(cx: &mut App) -> impl IntoElement {
    let status = cx.global::<AppState>().hub.read(cx).status().clone();
    let mut tooltip = status.label();
    if let Some(reason) = status.reason().filter(|reason| !reason.is_empty()) {
        tooltip = format!("{tooltip} — {reason}").into();
    }
    let trigger = Button::new("connection-status")
        .label(tr!("app.settings.server"))
        .ghost()
        .xsmall()
        .child(div().size(px(6.)).rounded_full().bg(status.color(cx)))
        .tooltip(tooltip);

    Popover::new("connection-popover")
        .trigger(trigger)
        .anchor(Anchor::BottomRight)
        .content(move |_state, _window, cx| {
            let hub = cx.global::<AppState>().hub.clone();
            let status = hub.read(cx).status().clone();
            let rtt = hub
                .read(cx)
                .clock()
                .and_then(|clock| clock.get())
                .map(|(_, rtt)| rtt);
            let up = hub.read(cx).connected();
            let connecting = matches!(status, ServerStatus::Connecting);
            let reason = status.reason().filter(|reason| !reason.is_empty());
            let addr = cx.global::<AppState>().settings.server.addr.clone();

            v_flex()
                .min_w(px(280.))
                .text_sm()
                .gap_1()
                // Status: dot + label (+ failure reason, muted).
                .child(
                    h_flex()
                        .gap_2()
                        .child(div().size(px(8.)).rounded_full().bg(status.color(cx)))
                        .child(div().child(status.label()))
                        .children(reason.map(|reason| {
                            div()
                                .truncate()
                                .text_color(cx.theme().muted_foreground)
                                .child(reason)
                        })),
                )
                // TCP latency: the latest heartbeat round-trip; "—" before
                // the first clock sample (also covers disconnected).
                .child(
                    h_flex()
                        .justify_between()
                        .child(
                            div()
                                .text_color(cx.theme().muted_foreground)
                                .child(tr!("app.settings.status.latency")),
                        )
                        .child(match rtt {
                            Some(rtt) => SharedString::from(format!("{rtt} ms")),
                            None => SharedString::from("—"),
                        }),
                )
                // Server address as configured in the settings.
                .child(
                    h_flex()
                        .justify_between()
                        .child(
                            div()
                                .text_color(cx.theme().muted_foreground)
                                .child(tr!("app.settings.server_addr")),
                        )
                        .child(div().max_w(px(180.)).truncate().child(if addr.is_empty() {
                            SharedString::from("—")
                        } else {
                            addr.into()
                        })),
                )
                // Connect / disconnect, same semantics as the settings page.
                .child(
                    h_flex()
                        .mt_1()
                        .gap_2()
                        .child(
                            Button::new("connection-connect")
                                .primary()
                                .small()
                                .label(tr!("app.settings.connect"))
                                .disabled(connecting || up)
                                .on_click({
                                    let hub = hub.clone();
                                    move |_, window, cx| {
                                        // The auth code is typed per connect in
                                        // dialog (never persisted).
                                        open_auth_code_dialog(window, cx, {
                                            let hub = hub.clone();
                                            move |code, cx| {
                                                hub.update(cx, |hub, cx| {
                                                    hub.connect(code.to_string(), cx)
                                                })
                                            }
                                        });
                                    }
                                }),
                        )
                        .child(
                            Button::new("connection-disconnect")
                                .outline()
                                .small()
                                .label(tr!("app.settings.disconnect"))
                                .disabled(!up && !connecting)
                                .on_click(move |_, _, cx| {
                                    hub.update(cx, |hub, cx| hub.disconnect(cx))
                                }),
                        ),
                )
        })
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
