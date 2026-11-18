//! The screener-family table delegates: the Finviz stock screener export
//! and the Market Screener's per-exchange live tables.

pub mod market;
pub mod stock;

pub use market::ExchangeTableDelegate;
pub use stock::StockScreenerDelegate;
