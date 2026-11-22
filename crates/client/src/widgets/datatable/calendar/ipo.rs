//! IPO calendar rows, scraped from iposcoop.com by the server.

use gpui_kit::{
    App, Context, IntoElement, Window,
    component::table::{Column, TableDelegate, TableState},
};

use crate::tr;
use crate::widgets::datatable::{self, DataPage, Loadable};

#[derive(Clone)]
pub struct IPODelegate {
    page: DataPage<crate::data::api::IpoItem>,
}

impl IPODelegate {
    pub fn new(data: Vec<crate::data::api::IpoItem>) -> Self {
        Self {
            page: DataPage {
                rows: data,
                ..Default::default()
            },
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
    type Row = crate::data::api::IpoItem;

    fn page_mut(&mut self) -> &mut DataPage<Self::Row> {
        &mut self.page
    }
}

impl TableDelegate for IPODelegate {
    fn columns_count(&self, _: &App) -> usize {
        7
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
        datatable::empty_view(tr!("app.calendar.ipo.empty").to_string().into(), cx)
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let row = &self.page.rows[row_ix];

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
