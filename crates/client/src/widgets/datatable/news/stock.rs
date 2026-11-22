//! News articles for one symbol from the Alpaca news API. A plain click
//! opens the headline in the translate dialog; Ctrl + click opens the
//! article URL.

use crate::data::api::NewsItem;
use gpui_kit::{
    App, Context, Div, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    Stateful, StatefulInteractiveElement as _, Styled as _, Window,
    component::{
        ActiveTheme as _, h_flex,
        table::{Column, TableDelegate, TableState},
    },
    div,
};

use crate::tr;
use crate::widgets::datatable::{self, DataPage, Loadable};

#[derive(Clone)]
pub struct StockNewsDelegate {
    /// The symbol this panel follows; `None` = nothing selected yet.
    pub(crate) symbol: Option<String>,
    page: DataPage<NewsItem>,
}

impl StockNewsDelegate {
    pub fn new(symbol: Option<String>, data: Vec<NewsItem>) -> Self {
        Self {
            symbol,
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
            0 => Column::new("headline", tr!("app.stock_news.headline"))
                .width(480.)
                .min_width(140.),
            1 => Column::new("source", tr!("app.stock_news.source"))
                .width(88.)
                .min_width(60.),
            2 => Column::new("updated_at", tr!("app.stock_news.updated_at"))
                .width(88.)
                .min_width(80.)
                .resizable(false),
            _ => Column::new("", ""),
        }
    }

    /// Switch the followed symbol; clears the current articles and
    /// invalidates any in-flight fetch for the old one.
    pub fn set_symbol(&mut self, symbol: Option<String>) {
        self.symbol = symbol;
        self.page.rows.clear();
        self.page.fetch_generation += 1;
    }
}

impl Loadable for StockNewsDelegate {
    type Row = NewsItem;

    fn page_mut(&mut self) -> &mut DataPage<Self::Row> {
        &mut self.page
    }
}

impl TableDelegate for StockNewsDelegate {
    fn columns_count(&self, _: &App) -> usize {
        3
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

    fn render_empty(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        datatable::empty_view(tr!("app.stock_news.empty"), cx)
    }

    fn render_tr(
        &mut self,
        row_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        let mut row = div().id(("stock-news-row", row_ix));

        if let Some(data) = self.page.rows.get(row_ix) {
            row = row
                .cursor_pointer()
                .on_click(super::open_news_row(data.clone()));
        }
        row
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let Some(article) = self.page.rows.get(row_ix) else {
            return div().into_any_element();
        };
        let theme = cx.theme();
        let muted = theme.muted_foreground;

        match col_ix {
            // 0 = headline, 1 = source, 2 = updated_at.
            0 => h_flex()
                .min_w_0()
                .gap_1()
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .truncate()
                        .text_sm()
                        .child(SharedString::from(article.headline.clone())),
                )
                .into_any_element(),
            1 => div()
                .truncate()
                .text_xs()
                .text_color(muted)
                .child(SharedString::from(article.source.clone()))
                .into_any_element(),
            2 => div()
                .text_xs()
                .text_color(muted)
                .child(SharedString::from(super::format_time(article.time_ms)))
                .into_any_element(),
            _ => div().into_any_element(),
        }
    }
}
