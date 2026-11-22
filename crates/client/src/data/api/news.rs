//! News queries (the live feed arrives through the stream's NEWS channel;
//! these cover the backfill/history pulls).

use super::{NewsItem, unpack};
use crate::utils::server::ServerClient;

/// The latest news mentioning any of `symbols` (empty = market-wide feed),
/// newest first.
pub async fn for_symbols(
    server: &ServerClient,
    symbols: Vec<String>,
    limit: u32,
) -> anyhow::Result<Vec<NewsItem>> {
    let response = server
        .rpc_checked(protocol::RpcRequestPayload::News { symbols, limit })
        .await?;
    unpack!(response, News)
}

/// The market-wide feed (no symbol filter), newest first.
pub async fn market(server: &ServerClient, limit: u32) -> anyhow::Result<Vec<NewsItem>> {
    for_symbols(server, Vec::new(), limit).await
}
