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

use crate::tr;
use crate::widgets::datatable::{self, Loadable};

/// A trading day's rows; the fetch window is the picked day itself. The row
/// is the export item with its (optional) time part already resolved to unix
/// seconds (see `utils::finviz`).
pub(crate) type EarningsItem = protocol::EarningsItem;

#[derive(Clone)]
pub struct EarningsDelegate {
    pub(crate) data: Vec<EarningsItem>,
    pub(crate) is_loading: bool,
    /// The day the user picked in the calendar; `None` = the latest trading
    /// day.
    pub(crate) selected_date: Option<NaiveDate>,
    fetch_generation: u64,
}

impl EarningsDelegate {
    pub fn new(data: Vec<EarningsItem>) -> Self {
        Self {
            data,
            is_loading: false,
            selected_date: None,
            fetch_generation: 0,
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
            7 => Column::new("revenue_estimate", tr!("app.calendar.earnings.revenue_estimate"))
                .width(130.),
            8 => Column::new("revenue_actual", tr!("app.calendar.earnings.revenue_actual"))
                .width(130.),
            9 => Column::new("revenue_surprise", tr!("app.calendar.earnings.revenue_surprise"))
                .width(130.),
            10 => Column::new("reaction", tr!("app.calendar.earnings.reaction")).width(150.),
            _ => Column::new("", ""),
        }
    }
}

impl Loadable for EarningsDelegate {
    type Row = EarningsItem;

    fn loading_mut(&mut self) -> &mut bool {
        &mut self.is_loading
    }

    fn rows_mut(&mut self) -> &mut Vec<Self::Row> {
        &mut self.data
    }

    fn fetch_generation_mut(&mut self) -> &mut u64 {
        &mut self.fetch_generation
    }
}

impl TableDelegate for EarningsDelegate {
    fn columns_count(&self, _: &App) -> usize {
        11
    }

    fn rows_count(&self, _: &App) -> usize {
        self.data.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        Self::column(col_ix)
    }

    fn loading(&self, _cx: &App) -> bool {
        self.is_loading
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
        let row = &self.data[row_ix];
        let theme = cx.theme();

        match col_ix {
            0 => datatable::format_date_local(row.time_ms).into_any_element(),
            1 => row.symbol.clone().into_any_element(),
            2 => row.company.clone().into_any_element(),
            3 => tinted(millions(row.market_cap), None).into_any_element(),
            4 => tinted(dollars(row.eps_estimate), None).into_any_element(),
            // Reported actuals are tinted cyan, like the source calendar.
            5 => tinted(dollars(row.eps_actual), Some(theme.cyan)).into_any_element(),
            6 => tinted(signed_pct(row.eps_surprise), pct_color(row.eps_surprise, cx))
                .into_any_element(),
            7 => tinted(millions(row.revenue_estimate), None).into_any_element(),
            8 => tinted(millions(row.revenue_actual), Some(theme.cyan)).into_any_element(),
            9 => tinted(signed_pct(row.revenue_surprise), pct_color(row.revenue_surprise, cx))
                .into_any_element(),
            10 => tinted(
                signed_pct(row.one_day_price_reaction),
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

/// Money in the export is in millions: `$12.86M`, `≥1000` rolls to `$2.59B`.
fn millions(value: Option<f64>) -> String {
    match value {
        Some(v) if v.abs() >= 1000. => format!("${:.2}B", v / 1000.),
        Some(v) => format!("${v:.2}M"),
        None => String::new(),
    }
}

/// EPS dollars: `$0.12`, negative as `-$0.21`.
fn dollars(value: Option<f64>) -> String {
    match value {
        Some(v) if v < 0. => format!("-${:.2}", -v),
        Some(v) => format!("${v:.2}"),
        None => String::new(),
    }
}

/// Signed percentage: `+5.04%` / `-4.43%` / `0.00%`.
fn signed_pct(value: Option<f64>) -> String {
    match value {
        Some(v) if v > 0. => format!("+{v:.2}%"),
        Some(v) => format!("{v:.2}%"),
        None => String::new(),
    }
}

/// Bullish/bearish tint for a signed percentage, uncolored at zero.
fn pct_color(value: Option<f64>, cx: &App) -> Option<Hsla> {
    let theme = cx.theme();
    match value {
        Some(v) if v > 0. => Some(theme.chart_bullish),
        Some(v) if v < 0. => Some(theme.chart_bearish),
        _ => None,
    }
}
