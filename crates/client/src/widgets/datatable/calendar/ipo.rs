//! IPO calendar rows, scraped from iposcoop.com by the server.

use gpui_kit::{
    App, Context, IntoElement, Window,
    component::table::{Column, TableDelegate, TableState},
};

use crate::tr;
use crate::widgets::datatable::{self, Loadable};

#[derive(Clone)]
pub struct IPODelegate {
    pub(crate) data: Vec<protocol::IpoItem>,
    pub(crate) is_loading: bool,
    fetch_generation: u64,
}

impl IPODelegate {
    pub fn new(data: Vec<protocol::IpoItem>) -> Self {
        Self {
            data,
            is_loading: false,
            fetch_generation: 0,
        }
    }

    /// Columns are built per call so a language switch retranslates the
    /// headers on the next render.
    fn column(col_ix: usize) -> Column {
        match col_ix {
            0 => Column::new("company", tr!("app.calendar.ipo.company")).width(90.),
            1 => Column::new("symbol", tr!("app.calendar.ipo.symbol"))
                .width(90.)
                .resizable(false),
            2 => Column::new("managers", tr!("app.calendar.ipo.managers")).width(160.),
            3 => Column::new("shares_millions", tr!("app.calendar.ipo.shares_millions"))
                .width(140.)
                .resizable(false),
            4 => Column::new("price_low", tr!("app.calendar.ipo.price_low"))
                .width(80.)
                .resizable(false),
            5 => Column::new("price_high", tr!("app.calendar.ipo.price_high"))
                .width(80.)
                .resizable(false),
            6 => Column::new("expected_date", tr!("app.calendar.ipo.expected_date"))
                .width(180.)
                .resizable(false),
            _ => Column::new("", ""),
        }
    }
}

impl Loadable for IPODelegate {
    type Row = protocol::IpoItem;

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

impl TableDelegate for IPODelegate {
    fn columns_count(&self, _: &App) -> usize {
        7
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
        datatable::empty_view(tr!("app.calendar.ipo.empty").to_string().into(), cx)
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let row = &self.data[row_ix];

        match col_ix {
            0 => row.company.to_string(),
            1 => row.symbol.clone(),
            2 => row.managers.clone(),
            3 => row.shares_millions.to_string(),
            4 => row.price_low.to_string(),
            5 => row.price_high.to_string(),
            6 => row.expected_date.clone(),
            _ => String::new(),
        }
    }
}
