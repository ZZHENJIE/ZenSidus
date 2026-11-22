//! Earnings calendar rows from the Finviz Elite export, fetched through the
//! app's Finviz client.

use chrono::NaiveDate;
use gpui_kit::{
    App, Context, Div, Hsla, IntoElement, ParentElement as _, Styled as _, Window,
    component::{
        ActiveTheme as _,
        table::{Column, TableDelegate, TableState},
    },
    div,
};

use crate::data::format;
use crate::tr;
use crate::widgets::datatable::{self, DataPage, Loadable};

/// A trading day's rows; the fetch window is the picked day itself. The row
/// is the export item with its (optional) time part already resolved to unix
/// seconds (see `utils::finviz`).
pub(crate) type EarningsItem = crate::data::api::EarningsItem;

#[derive(Clone)]
pub struct EarningsDelegate {
    page: DataPage<EarningsItem>,
    /// The day the user picked in the calendar; `None` = the latest trading
    /// day.
    pub(crate) selected_date: Option<NaiveDate>,
}

impl EarningsDelegate {
    pub fn new(data: Vec<EarningsItem>) -> Self {
        Self {
            page: DataPage {
                rows: data,
                ..Default::default()
            },
            selected_date: None,
        }
    }

    /// Columns are built per call so a language switch retranslates the
    /// headers on the next render.
    fn column(col_ix: usize) -> Column {
        match col_ix {
            0 => Column::new("date", tr!("app.calendar.earnings.date"))
                .width(70.)
                .resizable(false),
            1 => Column::new("symbol", tr!("app.calendar.earnings.symbol")).width(90.),
            2 => Column::new("company", tr!("app.calendar.earnings.company")).width(240.),
            3 => Column::new("market_cap", tr!("app.calendar.earnings.market_cap")).width(100.),
            4 => Column::new("eps_estimate", tr!("app.calendar.earnings.eps_estimate")).width(110.),
            5 => Column::new("eps_actual", tr!("app.calendar.earnings.eps_actual")).width(100.),
            6 => Column::new("eps_surprise", tr!("app.calendar.earnings.eps_surprise")).width(110.),
            7 => Column::new(
                "revenue_estimate",
                tr!("app.calendar.earnings.revenue_estimate"),
            )
            .width(130.),
            8 => Column::new(
                "revenue_actual",
                tr!("app.calendar.earnings.revenue_actual"),
            )
            .width(130.),
            9 => Column::new(
                "revenue_surprise",
                tr!("app.calendar.earnings.revenue_surprise"),
            )
            .width(130.),
            10 => Column::new("reaction", tr!("app.calendar.earnings.reaction")).width(150.),
            _ => Column::new("", ""),
        }
    }
}

impl Loadable for EarningsDelegate {
    type Row = EarningsItem;

    fn page_mut(&mut self) -> &mut DataPage<Self::Row> {
        &mut self.page
    }
}

impl TableDelegate for EarningsDelegate {
    fn columns_count(&self, _: &App) -> usize {
        11
    }

    fn rows_count(&self, _: &App) -> usize {
        self.page.rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        Self::column(col_ix)
    }

    fn loading(&self, _cx: &App) -> bool {
        self.page.is_loading
    }

    fn render_loading(
        &mut self,
        _size: gpui_kit::component::Size,
        _window: &mut Window,
        _cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        datatable::loading_view()
    }

    /// A failed fetch clears the rows; say so instead of the bare icon.
    fn render_empty(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        datatable::empty_view(tr!("app.calendar.earnings.empty").to_string().into(), cx)
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let row = &self.page.rows[row_ix];
        let theme = cx.theme();

        match col_ix {
            0 => format::format_date_local(row.time_ms).into_any_element(),
            1 => row.symbol.clone().into_any_element(),
            2 => row.company.clone().into_any_element(),
            3 => tinted(format::millions(row.market_cap), None).into_any_element(),
            4 => tinted(format::dollars(row.eps_estimate), None).into_any_element(),
            // Reported actuals are tinted cyan, like the source calendar.
            5 => tinted(format::dollars(row.eps_actual), Some(theme.cyan)).into_any_element(),
            6 => tinted(
                format::signed_pct(row.eps_surprise),
                pct_color(row.eps_surprise, cx),
            )
            .into_any_element(),
            7 => tinted(format::millions(row.revenue_estimate), None).into_any_element(),
            8 => tinted(format::millions(row.revenue_actual), Some(theme.cyan)).into_any_element(),
            9 => tinted(
                format::signed_pct(row.revenue_surprise),
                pct_color(row.revenue_surprise, cx),
            )
            .into_any_element(),
            10 => tinted(
                format::signed_pct(row.one_day_price_reaction),
                pct_color(row.one_day_price_reaction, cx),
            )
            .into_any_element(),
            _ => String::new().into_any_element(),
        }
    }
}

/// A formatted cell, optionally tinted (`None` keeps the default color).
fn tinted(text: String, color: Option<Hsla>) -> Div {
    match color {
        Some(color) => div().text_color(color).child(text),
        None => div().child(text),
    }
}

// The money/percent cell formats (`millions`/`dollars`/`signed_pct`) are
// shared display formatters now: [`crate::data::format`].

/// Bullish/bearish tint for a signed percentage, uncolored at zero.
fn pct_color(value: Option<f64>, cx: &App) -> Option<Hsla> {
    let theme = cx.theme();
    match value {
        Some(v) if v > 0. => Some(theme.chart_bullish),
        Some(v) if v < 0. => Some(theme.chart_bearish),
        _ => None,
    }
}
