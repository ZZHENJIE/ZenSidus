//! The time & sales tape for one followed symbol: a **hit log** of the
//! trades the shared Alpaca stream delivers, newest first. Rows are never
//! removed by later trades — this is a record of what printed, capped at
//! [`MAX_TRADES`] with the oldest falling off the bottom. Only the followed
//! symbol's trades are recorded; a symbol switch starts a fresh tape.

use anysdk::alpaca_stream::stocks::TradeMessage;
use chrono::Local;
use gpui_kit::{
    App, IntoElement, Pixels, SharedString, Window,
    component::table::{Column, TableDelegate, TableState},
    px,
};

use crate::tr;
use crate::widgets::datatable;

/// Upper bound on recorded trades: a liquid symbol can print many times per
/// second; past this the oldest rows fall off the bottom.
const MAX_TRADES: usize = 500;

/// One recorded trade.
#[derive(Clone)]
pub(crate) struct TradeRow {
    pub time: SharedString,
    pub price: f64,
    pub size: i64,
    pub exchange: String,
}

pub struct TradesDelegate {
    /// The followed symbol (uppercased); `None` = nothing followed yet.
    pub(crate) symbol: Option<String>,
    /// Recorded trades, newest first.
    trades: Vec<TradeRow>,
    /// Whether the Alpaca stream is currently connected (empty-state hint).
    pub(crate) connected: bool,
}

impl TradesDelegate {
    pub(crate) fn new(symbol: Option<String>, connected: bool) -> Self {
        Self {
            symbol: symbol.map(|s| s.to_ascii_uppercase()),
            trades: Vec::new(),
            connected,
        }
    }

    /// Follow `symbol`; a switch starts a fresh tape. Returns whether the
    /// followed symbol changed (the caller resubscribes).
    pub(crate) fn set_symbol(&mut self, symbol: Option<String>) -> bool {
        let symbol = symbol.map(|s| s.to_ascii_uppercase());
        if symbol == self.symbol {
            return false;
        }
        self.symbol = symbol;
        self.trades.clear();
        true
    }

    /// Record `trade` when it belongs to the followed symbol. Returns
    /// whether a row was appended (the caller notifies).
    pub(crate) fn apply_trade(&mut self, trade: &TradeMessage) -> bool {
        let Some(symbol) = &self.symbol else {
            return false;
        };
        if !trade.symbol.eq_ignore_ascii_case(symbol) {
            return false;
        }
        let time = trade
            .time
            .with_timezone(&Local)
            .format("%H:%M:%S%.3f")
            .to_string();
        self.trades.insert(
            0,
            TradeRow {
                time: time.into(),
                price: trade.price,
                size: trade.size,
                exchange: trade.exchange.clone(),
            },
        );
        if self.trades.len() > MAX_TRADES {
            self.trades.truncate(MAX_TRADES);
        }
        true
    }
}

impl TableDelegate for TradesDelegate {
    fn columns_count(&self, _: &App) -> usize {
        4
    }

    fn rows_count(&self, _: &App) -> usize {
        self.trades.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        match col_ix {
            0 => Column::new("time", tr!("app.trades.time")).width(px(96.)),
            1 => Column::new("price", tr!("app.trades.price"))
                .width(px(76.))
                .text_right(),
            2 => Column::new("size", tr!("app.trades.size"))
                .width(px(72.))
                .text_right(),
            _ => Column::new("exchange", tr!("app.trades.exchange"))
                .width(px(76.))
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
        for row in &self.trades {
            let text = match col_ix {
                0 => row.time.clone(),
                1 => format!("{:.2}", row.price).into(),
                2 => format!("{}", row.size).into(),
                _ => row.exchange.clone().into(),
            };
            width = width.max(datatable::measure_text(window, &text));
        }
        Some(width)
    }

    fn loading(&self, _: &App) -> bool {
        false
    }

    /// No symbol followed / not connected / connected but nothing printed
    /// yet.
    fn render_empty(
        &mut self,
        _window: &mut Window,
        cx: &mut gpui_kit::Context<TableState<Self>>,
    ) -> impl IntoElement {
        let hint = if self.symbol.is_none() {
            tr!("app.trades.no_symbol")
        } else if !self.connected {
            tr!("app.trades.not_connected")
        } else {
            tr!("app.trades.waiting")
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
        let cell = match self.trades.get(row_ix) {
            Some(row) => match col_ix {
                0 => row.time.clone(),
                1 => SharedString::from(format!("{:.2}", row.price)),
                2 => SharedString::from(format!("{}", row.size)),
                _ => SharedString::from(row.exchange.clone()),
            },
            None => SharedString::default(),
        };
        cell.into_any_element()
    }
}
