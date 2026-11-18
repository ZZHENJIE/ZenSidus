//! Table delegates — the data side of the table panels — and the helpers
//! they share: construction, rendering, the fetch-into-table flow and the
//! loading/empty placeholders.

pub mod calendar;
pub mod news;
pub mod screener;
pub mod trades;
pub mod watchlist;

pub use calendar::{EconomicDelegate, EarningsDelegate, IPODelegate};
pub use news::{MarketNewsDelegate, StockNewsDelegate};
pub use screener::{ExchangeTableDelegate, StockScreenerDelegate};
pub use trades::TradesDelegate;
pub use watchlist::WatchlistDelegate;


use chrono::{TimeZone, Utc};
use gpui_kit::{
    App, AppContext as _, Div, Entity, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, SharedString, Stateful, Styled as _, TextRun, WeakEntity, Window,
    base::h_flex,
    component::{
        ActiveTheme as _, Sizable as _,
        spinner::Spinner,
        table::{DataTable, TableDelegate, TableState},
    },
    div,
};

use crate::utils::message;

/// The width of `text` as a table draws it: the window's UI font at
/// `text_sm` (the cell text size for the tables' default density), shaped
/// unwrapped. Shared by the delegates' `fit_column` implementations.
pub(crate) fn measure_text(window: &mut Window, text: &str) -> Pixels {
    if text.is_empty() {
        return Pixels::ZERO;
    }
    let run = TextRun {
        len: text.len(),
        font: window.text_style().font(),
        color: Hsla::default(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let font_size = window.rem_size() * 0.875;
    window
        .text_system()
        .shape_text(text.to_string().into(), font_size, &[run], None, None)
        .ok()
        .and_then(|lines| lines.first().map(|line| line.width()))
        .unwrap_or_default()
}

/// A delegate whose rows come from a remote source; [`load_rows`] drives its
/// loading flag and row storage, and guards write-backs with the fetch
/// generation so a slow earlier response can never overwrite a newer one.
pub trait Loadable: TableDelegate {
    type Row;
    fn loading_mut(&mut self) -> &mut bool;
    fn rows_mut(&mut self) -> &mut Vec<Self::Row>;
    /// Monotonic counter bumped by [`load_rows`] on every new fetch; the
    /// delegate owns the storage so reset paths (e.g. a symbol switch) can
    /// also invalidate in-flight requests by bumping it themselves.
    fn fetch_generation_mut(&mut self) -> &mut u64;
}

/// Fetch rows into a table: flag it loading, run `request` with the app's
/// relay client, then store the rows — or clear them and report `error_title`
/// to the message center. A table that is already gone is ignored, and a
/// response whose fetch has been superseded (a newer `load_rows` or a
/// delegate reset started meanwhile) is discarded without touching the rows.
pub fn load_rows<D, Fut>(
    table: WeakEntity<TableState<D>>,
    error_title: SharedString,
    request: impl FnOnce(crate::utils::relay::RelayClient) -> Fut + 'static,
    cx: &mut App,
) where
    D: Loadable + 'static,
    Fut: Future<Output = anyhow::Result<Vec<D::Row>>> + 'static,
{
    let Some(table) = table.upgrade() else {
        return;
    };
    let Ok(relay) = crate::utils::state::AppState::relay(cx) else {
        message::error(error_title, anyhow::anyhow!("relay is not connected"), cx);
        return;
    };
    let generation = table.update(cx, |state, cx| {
        let delegate = state.delegate_mut();
        *delegate.loading_mut() = true;
        *delegate.fetch_generation_mut() += 1;
        cx.notify();
        *delegate.fetch_generation_mut()
    });

    cx.spawn(async move |cx| {
        let result = request(relay).await;
        table.update(cx, |state, cx| {
            let delegate = state.delegate_mut();
            if *delegate.fetch_generation_mut() != generation {
                // A newer fetch (or a delegate reset) started while this
                // request was in flight; its outcome is obsolete.
                return;
            }
            *delegate.loading_mut() = false;
            match result {
                Ok(rows) => *delegate.rows_mut() = rows,
                Err(err) => {
                    delegate.rows_mut().clear();
                    message::error(error_title, err, cx);
                }
            }
            cx.notify();
        });
    })
    .detach();
}

/// A table whose columns stay in their declared order.
pub fn new_table_state<T: TableDelegate>(
    cx: &mut App,
    window: &mut Window,
    new_entity: impl FnOnce(&mut App) -> T,
) -> Entity<TableState<T>> {
    cx.new(|cx| TableState::new(new_entity(cx), window, cx).col_movable(false))
}

/// The table element every table panel renders.
pub fn table_render<T: TableDelegate>(state: &Entity<TableState<T>>) -> impl IntoElement {
    DataTable::new(state)
        .bordered(false)
        .scrollbar_visible(true, true)
}

/// Centered spinner for `TableDelegate::render_loading`.
pub fn loading_view() -> impl IntoElement {
    h_flex()
        .justify_center()
        .size_full()
        .child(Spinner::new().large())
}

/// Centered muted hint for `TableDelegate::render_empty`.
pub fn empty_view(text: SharedString, cx: &App) -> impl IntoElement {
    div()
        .flex()
        .justify_center()
        .py_4()
        .text_color(cx.theme().muted_foreground)
        .child(text)
}

/// A calendar row's timestamp (epoch ms) shown in the viewer's local time
/// (`MM-DD HH:MM`), shared by the economic and earnings delegates.
pub fn format_ts_local(time_ms: i64) -> String {
    match chrono::Local.timestamp_millis_opt(time_ms) {
        chrono::LocalResult::Single(time) => time.format("%m-%d %H:%M").to_string(),
        _ => String::new(),
    }
}

/// A calendar row whose timestamp is in the future gets a subtle tinted
/// background, so the upcoming side of the fetched window stands out. Shared
/// by the economic and earnings delegates.
pub fn tint_upcoming_row(
    id: &'static str,
    row_ix: usize,
    time_ms: i64,
    cx: &App,
) -> Stateful<Div> {
    let row = div().id((id, row_ix));
    if time_ms > Utc::now().timestamp_millis() {
        row.bg(cx.theme().secondary)
    } else {
        row
    }
}
