//! Table delegates — the data side of the table panels — and the helpers
//! they share: construction, rendering, the fetch-into-table flow and the
//! loading/empty placeholders.

pub mod calendar;
pub mod news;
pub mod quote;
pub mod screener;
pub mod trades;
pub mod watchlist;

pub use calendar::{EarningsDelegate, EconomicDelegate, IPODelegate};
pub use news::{MarketNewsDelegate, StockNewsDelegate};
pub use quote::{QuoteSide, QuoteSideDelegate};
pub use screener::{ExchangeTableDelegate, StockScreenerDelegate};
pub use trades::TradesDelegate;
pub use watchlist::WatchlistDelegate;

use chrono::Utc;
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
    /// The delegate's fetched-data page (rows + fetch handshake + tri-state).
    fn page_mut(&mut self) -> &mut DataPage<Self::Row>;

    fn loading_mut(&mut self) -> &mut bool {
        &mut self.page_mut().is_loading
    }

    fn rows_mut(&mut self) -> &mut Vec<Self::Row> {
        &mut self.page_mut().rows
    }

    /// Monotonic counter bumped by [`load_rows`] on every new fetch; the
    /// delegate owns the storage so reset paths (e.g. a symbol switch) can
    /// also invalidate in-flight requests by bumping it themselves.
    fn fetch_generation_mut(&mut self) -> &mut u64 {
        &mut self.page_mut().fetch_generation
    }
}

/// One fetch's state page: the rows plus the fetch handshake and the
/// loading/error flags — the tri-state every RPC table renders from
/// (loading → spinner via `loading_view`, rows → the table, empty →
/// `empty_view`; a failure keeps old rows cleared and lands in the message
/// center, `last_error` only records it for a table that opts into
/// rendering `error_view`).
#[derive(Clone)]
pub struct DataPage<Row> {
    pub rows: Vec<Row>,
    pub(crate) is_loading: bool,
    pub(crate) fetch_generation: u64,
    pub(crate) last_error: Option<SharedString>,
}

// Manual impl: the derive would demand `Row: Default`, and the row types
// have no reason to be default-constructible.
impl<Row> Default for DataPage<Row> {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            is_loading: false,
            fetch_generation: 0,
            last_error: None,
        }
    }
}

/// Fetch rows into a table: flag it loading, run `request` with the app's
/// server client, then store the rows — or clear them and report `error_title`
/// to the message center. A table that is already gone is ignored, and a
/// response whose fetch has been superseded (a newer `load_rows` or a
/// delegate reset started meanwhile) is discarded without touching the rows.
///
/// The orchestration (offline guard, generation handshake, error report)
/// lives in [`crate::data::DataRequest`]; this wrapper only supplies the
/// table's loading/rows callbacks.
pub fn load_rows<D, Fut>(
    table: WeakEntity<TableState<D>>,
    error_title: SharedString,
    request: impl FnOnce(crate::utils::server::ServerClient) -> Fut + 'static,
    cx: &mut App,
) where
    D: Loadable + 'static,
    Fut: Future<Output = anyhow::Result<Vec<D::Row>>> + 'static,
{
    crate::data::DataRequest::new(table, error_title.clone()).run(
        cx,
        |state, cx| {
            let delegate = state.delegate_mut();
            *delegate.loading_mut() = true;
            *delegate.fetch_generation_mut() += 1;
            cx.notify();
            Some(*delegate.fetch_generation_mut())
        },
        |state, generation| *state.delegate_mut().fetch_generation_mut() == generation,
        request,
        |state, _cx, _generation, rows| {
            let delegate = state.delegate_mut();
            *delegate.loading_mut() = false;
            delegate.page_mut().last_error = None;
            *delegate.rows_mut() = rows;
        },
        |state, _cx, _generation, err| {
            let delegate = state.delegate_mut();
            *delegate.loading_mut() = false;
            delegate.rows_mut().clear();
            delegate.page_mut().last_error = Some(format!("{err:#}").into());
        },
    );
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

/// Centered muted hint carrying a fetch failure, for delegates that opt
/// into rendering their [`DataPage`] error state in-table (the default is
/// message-center only). Same shape as `empty_view`, in the danger tone.
pub fn error_view(error: SharedString, cx: &App) -> impl IntoElement {
    div()
        .flex()
        .justify_center()
        .py_4()
        .text_color(cx.theme().danger)
        .child(error)
}

/// The calendar delegates' local-time stamp formatting lives in
/// [`crate::data::format`] (shared with the news rows).
///
/// A calendar row whose timestamp is in the future gets a subtle tinted
/// background, so the upcoming side of the fetched window stands out. Shared
/// by the economic and earnings delegates.
pub fn tint_upcoming_row(id: &'static str, row_ix: usize, time_ms: i64, cx: &App) -> Stateful<Div> {
    let row = div().id((id, row_ix));
    if time_ms > Utc::now().timestamp_millis() {
        row.bg(cx.theme().secondary)
    } else {
        row
    }
}
