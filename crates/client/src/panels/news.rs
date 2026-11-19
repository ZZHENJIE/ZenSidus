//! The news-family dock panels: per-symbol stock news and the market-wide
//! news feed (RPC backfill + the server's NEWS push channel).

pub mod market;
pub mod stock;

pub use market::MarketNewsPanel;
pub use stock::StockNewsPanel;

use gpui_kit::App;

use crate::tr;
use crate::utils::server::ServerClient;
use crate::utils::{message, state::AppState};

/// The app-wide server client, or `None` — with a message pointing at the
/// connection state — while the server is down. Shared by both news panels
/// (and every other RPC consumer that wants the same guard).
pub(crate) fn server_rpc(cx: &mut App) -> Option<ServerClient> {
    let found = AppState::server(cx).ok();
    if found.is_none() {
        message::error(
            tr!("app.stock_news.no_credentials"),
            tr!("app.log.server_not_connected"),
            cx,
        );
    }
    found
}
