//! The news-family table delegates: per-symbol stock news and the
//! market-wide feed, plus the helpers they share.

pub mod market;
pub mod stock;

pub use market::MarketNewsDelegate;
pub use stock::StockNewsDelegate;

use chrono::Local;
use gpui_kit::{App, ClickEvent, Window};
use relay_protocol::NewsItem;

/// A news timestamp (epoch ms) shown as `MM-DD HH:MM` in the viewer's local
/// time; shared by both delegates.
pub(super) fn format_time(time_ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(time_ms)
        .map(|time| time.with_timezone(&Local).format("%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

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
