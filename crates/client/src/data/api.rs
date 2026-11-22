//! Typed RPC queries — one function per RPC, one file per domain. A query
//! takes the server client and its arguments, sends the request and unpacks
//! the matching response variant; nothing else (no generation, no loading
//! flags, no message center — wrap queries in [`crate::data::DataRequest`]
//! for that).
//!
//! The DTOs are re-exported here so callers never import `protocol` items
//! directly.

pub mod bars;
pub mod calendar;
pub mod news;
pub mod screener;
pub mod snapshot;
pub mod text;

pub use protocol::{
    Bar, Cell, EarningsItem, EconomicItem, IpoItem, NewsItem, ScreenerExport, SearchItem,
    SnapshotItem,
};

/// Send one RPC and unpack its matching response variant; any other shape
/// is an error (protocol bug, never user-visible in normal operation).
macro_rules! unpack {
    ($response:expr, $variant:ident) => {
        match $response {
            protocol::RpcResponsePayload::$variant(value) => Ok(value),
            other => Err(anyhow::anyhow!("unexpected rpc response: {other:?}")),
        }
    };
}

pub(crate) use unpack;
