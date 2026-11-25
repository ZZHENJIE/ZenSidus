//! Finviz screener exports (token lives server-side; the client sends the
//! parameter set only).
//!
//! The export arrives **typed** (`Cell { text, value }` — the server parses
//! each cell, keeping the upstream text verbatim for display and exposing
//! the numeric value for sorting/filtering).

use anysdk::finviz_sdk::screener::Parameter;

use super::{ScreenerExport, unpack};
use crate::utils::server::ServerClient;

/// The stock screener's export for `parameter`.
pub async fn export(
    server: &ServerClient,
    parameter: &Parameter,
) -> anyhow::Result<ScreenerExport> {
    let response = server
        .rpc_checked(protocol::RpcRequestPayload::ScreenerExport {
            parameter: parameter_dto(parameter),
        })
        .await?;
    unpack!(response, ScreenerExport)
}

/// The market screener's first-round pool export (same upstream, separate
/// variant so the two can diverge later).
pub async fn market_export(
    server: &ServerClient,
    parameter: &Parameter,
) -> anyhow::Result<ScreenerExport> {
    let response = server
        .rpc_checked(protocol::RpcRequestPayload::MarketScreenerExport {
            parameter: parameter_dto(parameter),
        })
        .await?;
    unpack!(response, MarketScreenerExport)
}

/// The settings-side finviz parameter → the protocol DTO (`screener_type`
/// takes its wire value, e.g. `stocks`/`etf`). Token is server-side, so
/// this only carries the parameter itself.
pub fn parameter_dto(parameter: &Parameter) -> protocol::ScreenerParameter {
    protocol::ScreenerParameter {
        screener_type: parameter.screener_type.as_str().to_string(),
        order_by: parameter.order_by.clone(),
        signal: parameter.signal.clone(),
        filters: parameter.filters.clone(),
        columns: parameter.columns.clone(),
    }
}
