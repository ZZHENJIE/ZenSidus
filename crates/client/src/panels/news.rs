//! The news-family dock panels: per-symbol stock news and the market-wide
//! news feed (RPC backfill + the server's NEWS push channel).

pub mod market;
pub mod stock;

pub use market::MarketNewsPanel;
pub use stock::StockNewsPanel;

use gpui_kit::App;

use crate::utils::server::ServerClient;
use crate::utils::state::AppState;

/// The app-wide server client, or `None` while the server is down. Callers
/// treat `None` as "skip silently": auto-load paths wait for the hub's
/// `Connected` event (`panels::reload_on_connect`), user-triggered refreshes
/// simply do nothing while offline — nothing lands in the message center.
pub(crate) fn server_rpc(cx: &mut App) -> Option<ServerClient> {
    AppState::server(cx).ok()
}
