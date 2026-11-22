//! The news-family dock panels: per-symbol stock news and the market-wide
//! news feed (RPC backfill + the server's NEWS push channel).

pub mod market;
pub mod stock;

pub use market::MarketNewsPanel;
pub use stock::StockNewsPanel;
