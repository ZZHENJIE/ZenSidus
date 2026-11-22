//! The news-family table delegates: per-symbol stock news and the
//! market-wide feed, plus the helpers they share.

pub mod market;
pub mod stock;

pub use market::MarketNewsDelegate;
pub use stock::StockNewsDelegate;

use crate::data::api::NewsItem;
use gpui_kit::{App, ClickEvent, Window};

// The news timestamp formatter is the shared [`crate::data::format`]
// helper (same `MM-DD HH:MM` shape); it stays callable as
// `super::format_time` for the two delegates.
pub(super) use crate::data::format::format_ts_local as format_time;

/// Click behavior shared by the news delegates: plain click opens the
/// headline in the translate dialog; Ctrl + click opens the article URL.
pub(super) fn open_news_row(article: NewsItem) -> impl Fn(&ClickEvent, &mut Window, &mut App) {
    move |event, window, app| {
        let ClickEvent::Mouse(mouse) = event else {
            return;
        };
        if mouse.down.modifiers.control {
            if let Some(url) = &article.url {
                app.open_url(url);
            }
        } else {
            crate::widgets::dialog::translate::open_translate_dialog(
                window,
                app,
                article.headline.clone(),
            );
        }
    }
}
