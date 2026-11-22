//! One side of the top of book (Bid or Ask) as a three-column table:
//! price / size / MPID. The Quote panel mounts two of these side by side;
//! rows update in place as quotes stream in.

use gpui_kit::{
    App, Context, Hsla, IntoElement, ParentElement as _, Styled as _, Window,
    component::{
        ActiveTheme as _,
        table::{Column, TableDelegate, TableState},
    },
    div, px,
};

use crate::data::format::format_size;
use crate::tr;

/// Which side of the spread the table renders — the price cell's color.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum QuoteSide {
    Bid,
    Ask,
}

/// The current top-of-book values of one side; `None` fields render dashes
/// (a one-sided quote is legal, e.g. at the opening auction).
#[derive(Clone, Default)]
pub struct QuoteRow {
    pub(crate) price: Option<f64>,
    pub(crate) size: Option<i64>,
    pub(crate) mpid: Option<String>,
}

pub struct QuoteSideDelegate {
    side: QuoteSide,
    row: QuoteRow,
}

impl QuoteSideDelegate {
    pub fn new(side: QuoteSide) -> Self {
        Self {
            side,
            row: QuoteRow::default(),
        }
    }

    /// Replace the row (a fresh quote on this side); missing fields render
    /// dashes.
    pub fn set_row(&mut self, price: Option<f64>, size: Option<i64>, mpid: Option<String>) -> bool {
        let changed = self.row.price != price || self.row.size != size || self.row.mpid != mpid;
        if changed {
            self.row = QuoteRow { price, size, mpid };
        }
        changed
    }

    /// The price cell's color: the user's bullish tone for the bid, bearish
    /// for the ask (red/green per the app theme).
    fn price_color(&self, cx: &App) -> Hsla {
        let theme = cx.theme();
        match self.side {
            QuoteSide::Bid => theme.chart_bullish,
            QuoteSide::Ask => theme.chart_bearish,
        }
    }
}

impl TableDelegate for QuoteSideDelegate {
    fn columns_count(&self, _: &App) -> usize {
        3
    }

    /// Always exactly one row: the live top of book (dashes before the
    /// first quote arrives).
    fn rows_count(&self, _: &App) -> usize {
        1
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        match col_ix {
            0 => Column::new("price", tr!("app.quote.price"))
                .width(px(76.))
                .text_right(),
            1 => Column::new("size", tr!("app.quote.size"))
                .width(px(72.))
                .text_right(),
            _ => Column::new("mpid", tr!("app.quote.mpid"))
                .width(px(56.))
                .text_right(),
        }
    }

    fn render_td(
        &mut self,
        _row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        match col_ix {
            0 => match self.row.price {
                Some(price) => div()
                    .text_color(self.price_color(cx))
                    .child(format!("{price:.2}"))
                    .into_any_element(),
                None => div().text_color(muted).child("—").into_any_element(),
            },
            1 => match self.row.size {
                Some(size) => format_size(size).into_any_element(),
                None => div().text_color(muted).child("—").into_any_element(),
            },
            _ => match &self.row.mpid {
                Some(mpid) => mpid.clone().into_any_element(),
                None => div().text_color(muted).child("—").into_any_element(),
            },
        }
    }
}
