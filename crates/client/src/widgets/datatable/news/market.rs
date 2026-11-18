//! Market-wide news from Alpaca. Rows come from two sources that must
//! coexist: an HTTP backfill for history (the stream only carries new
//! articles) and live inserts pushed by the news WebSocket stream. Rows are
//! therefore merged by article id — never replaced wholesale — and kept
//! newest-first. Click behavior matches the stock news delegate: plain
//! click opens the headline in the translate dialog; Ctrl + click opens the
//! article URL.

use relay_protocol::NewsItem;
use gpui_kit::{
    App, Context, Div, InteractiveElement as _, IntoElement, ParentElement as _, Stateful,
    StatefulInteractiveElement as _, Styled as _, Window,
    component::{
        ActiveTheme as _,
        table::{Column, TableDelegate, TableState},
    },
    div,
};

use crate::tr;
use crate::widgets::datatable::Loadable;

#[derive(Clone)]
pub struct MarketNewsDelegate {
    pub(crate) data: Vec<NewsItem>,
    pub(crate) is_loading: bool,
    fetch_generation: u64,
}

impl MarketNewsDelegate {
    pub fn new(data: Vec<NewsItem>) -> Self {
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
            0 => Column::new("headline", tr!("app.market_news.headline"))
                .width(560.)
                .min_width(140.),
            1 => Column::new("symbols", tr!("app.market_news.symbols"))
                .width(110.)
                .min_width(60.),
            2 => Column::new("source", tr!("app.market_news.source"))
                .width(88.)
                .min_width(60.),
            3 => Column::new("updated_at", tr!("app.market_news.updated_at"))
                .width(88.)
                .min_width(80.)
                .resizable(false),
            _ => Column::new("", ""),
        }
    }

    /// Merge articles from any source (HTTP backfill or stream push): only
    /// ids not already in the table are inserted, then the rows are sorted
    /// newest-update first. Returns whether anything changed.
    pub(crate) fn merge_articles(&mut self, incoming: Vec<NewsItem>) -> bool {
        let mut changed = false;
        for article in incoming {
            if !self.data.iter().any(|row| row.id == article.id) {
                self.data.push(article);
                changed = true;
            }
        }
        if changed {
            self.data.sort_by_key(|row| std::cmp::Reverse(row.time_ms));
        }
        changed
    }
}

impl Loadable for MarketNewsDelegate {
    type Row = NewsItem;

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

impl TableDelegate for MarketNewsDelegate {
    fn columns_count(&self, _: &App) -> usize {
        4
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
        crate::widgets::datatable::loading_view()
    }

    fn render_empty(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        crate::widgets::datatable::empty_view(tr!("app.market_news.empty"), cx)
    }

    fn render_tr(
        &mut self,
        row_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        let mut row = div().id(("market-news-row", row_ix));
        if let Some(article) = self.data.get(row_ix) {
            row = row.cursor_pointer().on_click(super::open_news_row(article.clone()));
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
        let Some(article) = self.data.get(row_ix) else {
            return div().into_any_element();
        };
        let muted = cx.theme().muted_foreground;

        match col_ix {
            // 0 = headline, 1 = related symbols, 2 = source, 3 = updated_at.
            0 => div()
                .min_w_0()
                .truncate()
                .text_sm()
                .child(article.headline.clone())
                .into_any_element(),
            1 => {
                let symbols = if article.symbols.is_empty() {
                    "—".to_string()
                } else {
                    article.symbols.join(", ")
                };
                div().truncate().text_xs().text_color(muted).child(symbols).into_any_element()
            }
            2 => div()
                .truncate()
                .text_xs()
                .text_color(muted)
                .child(article.source.clone())
                .into_any_element(),
            3 => div()
                .text_xs()
                .text_color(muted)
                .child(super::format_time(article.time_ms))
                .into_any_element(),
            _ => div().into_any_element(),
        }
    }
}
