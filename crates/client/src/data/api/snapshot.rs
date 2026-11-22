//! Snapshots (latest trade/quote + minute/daily/prev-daily bars per
//! symbol); the ETF link-calculator anchors its day's opens on these.

use super::{SnapshotItem, unpack};
use crate::utils::server::ServerClient;

/// One snapshot per symbol, in request order.
pub async fn for_symbols(
    server: &ServerClient,
    symbols: Vec<String>,
) -> anyhow::Result<Vec<SnapshotItem>> {
    let response = server
        .rpc_checked(protocol::RpcRequestPayload::Snapshot { symbols })
        .await?;
    unpack!(response, Snapshot)
}
