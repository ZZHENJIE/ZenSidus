//! The calendar family: economic (one week, Mon–Fri server-side window),
//! earnings (one trading day) and IPO (the full list).

use chrono::NaiveDate;

use super::{EarningsItem, EconomicItem, IpoItem, unpack};
use crate::utils::server::ServerClient;

/// The week `date` falls in (the server snaps the date to its Monday and
/// serves the Mon–Fri window).
pub async fn economic(server: &ServerClient, date: NaiveDate) -> anyhow::Result<Vec<EconomicItem>> {
    let response = server
        .rpc_checked(protocol::RpcRequestPayload::EconomicCalendar {
            date: date.to_string(),
        })
        .await?;
    unpack!(response, EconomicCalendar)
}

/// The trading day `date` (weekends have no session; the server serves what
/// the upstream export has for the day).
pub async fn earnings(server: &ServerClient, date: NaiveDate) -> anyhow::Result<Vec<EarningsItem>> {
    let response = server
        .rpc_checked(protocol::RpcRequestPayload::EarningsCalendar {
            date: date.to_string(),
        })
        .await?;
    unpack!(response, EarningsCalendar)
}

/// The full IPO calendar (iposcoop, scraped server-side).
pub async fn ipo(server: &ServerClient) -> anyhow::Result<Vec<IpoItem>> {
    let response = server
        .rpc_checked(protocol::RpcRequestPayload::IpoCalendar)
        .await?;
    unpack!(response, IpoCalendar)
}
