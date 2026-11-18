//! One exchange's table of the Market Screener panel. The panel builds one
//! table per exchange of the selected preset; this delegate is a **hit log**:
//! whenever a pool symbol's quote arrives with an ask or bid side on *this*
//! exchange meeting the preset's minimum size, the event is appended as a
//! row (symbol + that quote's qualifying size). Rows are never removed by
//! later quotes and symbols may repeat — this is a record of what hit, not
//! a live snapshot of what currently qualifies. Newest rows first.
//!
//! The min_size is evaluated per event from the shared settings, so rule
//! edits apply to the events that arrive after them.

use std::collections::HashSet;

use anysdk::alpaca_stream::stocks::QuoteMessage;
use gpui_kit::{
    App, IntoElement, Pixels, SharedString, Window,
    component::table::{Column, TableDelegate, TableState},
    px,
};

use crate::tr;
use crate::widgets::datatable;

/// Upper bound on recorded hits per table: a busy symbol can qualify many
/// times per second; past this the oldest hits fall off the bottom.
const MAX_HITS: usize = 400;

/// One recorded hit: a symbol whose quote qualified on this exchange, with
/// the size of the qualifying side (the larger one when both sides did).
#[derive(Clone)]
pub(crate) struct Hit {
    pub symbol: String,
    pub size: i64,
}

pub struct ExchangeTableDelegate {
    /// The exchange code (an Alpaca `ax`/`bx` value) this table filters for.
    pub(crate) exchange: String,
    /// Pool symbols in Finviz order. Only quotes of pool symbols are
    /// recorded; the order itself is unused (the log is chronological).
    pub(crate) pool: Vec<String>,
    pool_set: HashSet<String>,
    /// Recorded hits, newest first.
    hits: Vec<Hit>,
    pub(crate) is_loading: bool,
    /// Whether the pool came back with at least one symbol.
    pub(crate) has_pool: bool,
    /// Whether the Alpaca stream is currently connected (empty-state hint).
    pub(crate) connected: bool,
}

impl ExchangeTableDelegate {
    pub(crate) fn new(exchange: String, connected: bool) -> Self {
        Self {
            exchange,
            pool: Vec::new(),
            pool_set: HashSet::new(),
            hits: Vec::new(),
            is_loading: false,
            has_pool: false,
            connected,
        }
    }

    /// The size this quote would be recorded with on this exchange, if its
    /// ask or bid side sits here meeting `min_size` (the larger one when
    /// both sides qualify). `None` = no hit.
    fn matched_size(&self, quote: &QuoteMessage, min_size: i64) -> Option<i64> {
        let side = |exchange: &str, size: i64| -> Option<i64> {
            (exchange.eq_ignore_ascii_case(&self.exchange) && (min_size <= 0 || size >= min_size))
                .then_some(size)
        };
        match (
            side(&quote.ask_exchange, quote.ask_size),
            side(&quote.bid_exchange, quote.bid_size),
        ) {
            (Some(ask), Some(bid)) => Some(ask.max(bid)),
            (Some(size), None) | (None, Some(size)) => Some(size),
            (None, None) => None,
        }
    }

    /// Record `quote` as a hit when its symbol is in the pool and the quote
    /// qualifies on this exchange. `min_size` comes from the panel (the
    /// selected preset's rule). Returns whether a row was appended (the
    /// caller notifies).
    pub(crate) fn apply_quote(&mut self, quote: &QuoteMessage, min_size: i64) -> bool {
        let symbol = quote.symbol.to_ascii_uppercase();
        if !self.pool_set.contains(&symbol) {
            return false;
        }
        let Some(size) = self.matched_size(quote, min_size) else {
            return false;
        };
        self.hits.insert(0, Hit { symbol, size });
        if self.hits.len() > MAX_HITS {
            self.hits.truncate(MAX_HITS);
        }
        true
    }

    /// Replace the pool (uppercased, Finviz order). Recorded hits stay —
    /// they are history, not a snapshot of the pool.
    pub(crate) fn set_pool(&mut self, tickers: &[String]) {
        let pool: Vec<String> = tickers.iter().map(|t| t.to_ascii_uppercase()).collect();
        if pool == self.pool {
            return;
        }
        self.pool_set = pool.iter().cloned().collect();
        self.has_pool = !pool.is_empty();
        self.pool = pool;
    }
}

/// `1,234,567` — sizes read better grouped.
fn format_size(size: i64) -> String {
    let digits = size.abs().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    if size < 0 {
        out.insert(0, '-');
    }
    out
}

impl TableDelegate for ExchangeTableDelegate {
    fn columns_count(&self, _: &App) -> usize {
        2
    }

    fn rows_count(&self, _: &App) -> usize {
        self.hits.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        match col_ix {
            0 => Column::new("symbol", tr!("app.market_screener.symbol")).width(px(72.)),
            _ => Column::new("size", tr!("app.market_screener.size"))
                .width(px(84.))
                .text_right(),
        }
    }

    /// Fit to the widest drawn cell or header; the table adds the cell
    /// padding and clamps to the column's min/max on top of this.
    fn fit_column(
        &mut self,
        col_ix: usize,
        window: &mut Window,
        _cx: &mut gpui_kit::Context<TableState<Self>>,
    ) -> Option<Pixels> {
        let mut width = datatable::measure_text(window, &self.column(col_ix, _cx).name);
        for hit in &self.hits {
            let text = match col_ix {
                0 => hit.symbol.clone(),
                _ => format_size(hit.size),
            };
            width = width.max(datatable::measure_text(window, &text));
        }
        Some(width)
    }

    fn loading(&self, _: &App) -> bool {
        self.is_loading
    }

    fn render_loading(
        &mut self,
        _size: gpui_kit::component::Size,
        _window: &mut Window,
        _cx: &mut gpui_kit::Context<TableState<Self>>,
    ) -> impl IntoElement {
        datatable::loading_view()
    }

    /// Not connected / pool empty / connected but nothing hit yet.
    fn render_empty(
        &mut self,
        _window: &mut Window,
        cx: &mut gpui_kit::Context<TableState<Self>>,
    ) -> impl IntoElement {
        let hint = if !self.connected {
            tr!("app.market_screener.not_connected")
        } else if !self.has_pool {
            tr!("app.market_screener.no_pool")
        } else {
            tr!("app.market_screener.waiting")
        };
        datatable::empty_view(hint, cx)
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _window: &mut Window,
        _cx: &mut gpui_kit::Context<TableState<Self>>,
    ) -> impl IntoElement {
        match self.hits.get(row_ix) {
            Some(hit) if col_ix == 0 => SharedString::from(hit.symbol.clone()).into_any_element(),
            Some(hit) => SharedString::from(format_size(hit.size)).into_any_element(),
            None => SharedString::default().into_any_element(),
        }
    }
}
