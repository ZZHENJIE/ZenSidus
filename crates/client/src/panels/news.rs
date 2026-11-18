//! The news-family dock panels: per-symbol stock news and the market-wide
//! news feed (RPC backfill + the relay's NEWS push channel).

pub mod market;
pub mod stock;

pub use market::MarketNewsPanel;
pub use stock::StockNewsPanel;

use gpui_kit::App;

use crate::tr;
use crate::utils::relay::RelayClient;
use crate::utils::{message, state::AppState};

/// The app-wide relay client, or `None` — with a message pointing at the
/// connection state — while the relay is down. Shared by both news panels
/// (and every other RPC consumer that wants the same guard).
pub(crate) fn relay_rpc(cx: &mut App) -> Option<RelayClient> {
    let found = AppState::relay(cx).ok();
    if found.is_none() {
        message::error(
            tr!("app.stock_news.no_credentials"),
            tr!("app.log.relay_not_connected"),
            cx,
        );
    }
    found
}
