//! Text queries: symbol search (futunn proxy) and translation (DeepSeek
//! proxy). These serve the dialogs, which own their state and show errors
//! inline — the message center is not involved here.

use super::{SearchItem, unpack};
use crate::utils::server::ServerClient;

/// Search suggestions for `keyword` (`lang` = the UI locale; the upstream
/// localizes names).
pub async fn search(
    server: &ServerClient,
    keyword: String,
    lang: String,
) -> anyhow::Result<Vec<SearchItem>> {
    let response = server
        .rpc_checked(protocol::RpcRequestPayload::Search { keyword, lang })
        .await?;
    unpack!(response, Search)
}

/// `text` translated from `from` to `to` (rust-i18n locale strings, e.g.
/// `en-us`/`zh-cn` — the server maps them to the upstream's codes).
pub async fn translate(
    server: &ServerClient,
    from: String,
    to: String,
    text: String,
) -> anyhow::Result<String> {
    let response = server
        .rpc_checked(protocol::RpcRequestPayload::Translate { from, to, text })
        .await?;
    unpack!(response, Translate)
}
