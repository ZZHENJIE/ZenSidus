//! The chart card for scrolling lists: the ticker name over a K-line, with
//! a spinner while the card's history is still in flight. A composition of
//! GPUI Kit containers plus the project's one custom element ([`Kline`]) —
//! nothing new is drawn here.
//!
//! Layout contract: the card is `w_full` — virtual-list entries size from
//! the measured entry width, and without an explicit width the card
//! collapses to content width, the chart's plot degenerates to zero and the
//! axis clamp panics (see the kline skill notes). The entry height
//! ([`CARD_H`]) is the caller's sizing knob for the list; the canvas takes
//! the rest ([`CARD_CHART_H`]).

use gpui_kit::{
    AnyElement, App, Entity, IntoElement as _, ParentElement as _, SharedString, Styled as _,
    base::v_flex, component::ActiveTheme as _, div, px,
};

use crate::widgets::datatable;

use super::{Kline, KlineState};

/// One chart card's height: ticker label + the K-line canvas.
pub const CARD_H: f32 = 260.;
/// The K-line canvas inside a card (the label row takes the rest).
pub const CARD_CHART_H: f32 = 240.;

/// The card for `ticker`: the chart of `state` once its history arrived, a
/// spinner while it is in flight (or not yet fetched). The mouse wheel is
/// left to the list's scrolling; drag-pan and crosshair stay on the chart.
pub fn kline_card(ticker: SharedString, state: Option<Entity<KlineState>>, cx: &App) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    let chart = match state {
        // 已有历史:渲染 K 线(滚轮让位给列表滚动;拖拽平移/十字光标保留)。
        Some(state) if !state.read(cx).bars().is_empty() => {
            let id = SharedString::from(format!("kline-card-{ticker}"));
            div()
                .h(px(CARD_CHART_H))
                .child(Kline::new(state.clone()).wheel_zoom(false).with_id(id))
        }
        // 历史在途/未拉:转圈。
        _ => div()
            .h(px(CARD_CHART_H))
            .flex()
            .items_center()
            .justify_center()
            .child(datatable::loading_view()),
    };
    // w_full:虚拟列表按测量宽度布局条目,卡片不带显式宽度会退化成
    // 内容宽,图表宽度归零(实测崩溃)。
    v_flex()
        .w_full()
        .child(
            div()
                .px_2()
                .pt_1()
                .text_xs()
                .text_color(muted)
                .child(ticker),
        )
        .child(chart)
        .into_any_element()
}
