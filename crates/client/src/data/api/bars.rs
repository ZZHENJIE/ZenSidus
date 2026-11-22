//! Historical bars (the chart panels' backbone; live bars come through the
//! stream, not here).

use super::{Bar, unpack};
use crate::utils::server::ServerClient;

/// One symbol's history, ascending, at most `limit` bars (the server
/// truncates the tail and returns ascending). A symbol the server has no
/// bars for yields an empty vector, not an error.
pub async fn history(
    server: &ServerClient,
    symbol: String,
    timeframe: String,
    limit: u32,
) -> anyhow::Result<Vec<Bar>> {
    let response = server
        .rpc_checked(protocol::RpcRequestPayload::Bars {
            symbols: vec![symbol.clone()],
            timeframe,
            start: None,
            end: None,
            limit,
        })
        .await?;
    Ok(unpack!(response, Bars)?.remove(&symbol).unwrap_or_default())
}
