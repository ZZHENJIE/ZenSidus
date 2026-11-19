//! RPC 处理器:每个 [`RpcRequestPayload`] 变体一个分支,统一返回
//! `Result<RpcResponsePayload, String>`(错误串原样进 RpcResponse)。
//! `Bars` 用 `bars_all_with_progress`,每页拉完经 `progress_tx` 发
//! `RestProgress`(D2),终值照常走 RpcResponse。

use anysdk::alpaca_sdk::{bars, news, snapshots};
use tokio::sync::mpsc;

use crate::finviz_data;
use crate::state::AppState;
use crate::web;
use protocol::{
    Bar, EconomicItem, EarningsItem, Frame, NewsItem, RpcRequestPayload, RpcResponsePayload,
    ScreenerExport, ScreenerParameter, SnapshotItem,
};
use protocol::rpc::SearchItem;

/// 处理一个 RPC 请求。`rpc_id` 是请求 id(RestProgress 帧用它回填 req_id);
/// `progress_tx` 是该会话的下行出口(bars 进度帧专用)。
pub async fn handle(
    state: &AppState,
    rpc_id: u32,
    progress_tx: &mpsc::UnboundedSender<Frame>,
    payload: RpcRequestPayload,
) -> Result<RpcResponsePayload, String> {
    match payload {
        RpcRequestPayload::News { symbols, limit } => {
            let page = state
                .alpaca
                .news(&news::Params {
                    symbols,
                    limit: Some(limit.clamp(1, 50)),
                    ..Default::default()
                })
                .await
                .map_err(rpc_error("news"))?;
            Ok(RpcResponsePayload::News(
                page.news
                    .into_iter()
                    .map(|article| NewsItem {
                        id: article.id,
                        symbols: article.symbols,
                        headline: article.headline,
                        source: article.source,
                        url: article.url,
                        time_ms: article.created_at.timestamp_millis(),
                    })
                    .collect(),
            ))
        }

        RpcRequestPayload::Bars {
            symbols,
            timeframe,
            start,
            end,
            limit,
        } => {
            let params = bars::Params {
                symbols,
                timeframe,
                start,
                end,
                limit: (limit > 0).then_some(limit),
                ..Default::default()
            };
            let progress_tx = progress_tx.clone();
            let all = state
                .alpaca
                .bars_all_with_progress(&params, |fetched| {
                    let _ = progress_tx.send(Frame::RestProgress {
                        req_id: rpc_id,
                        fetched: fetched as u64,
                    });
                })
                .await
                .map_err(rpc_error("bars"))?;
            Ok(RpcResponsePayload::Bars(
                all.into_iter()
                    .map(|(symbol, bars)| (symbol, bars.into_iter().map(bar_from_rest).collect()))
                    .collect(),
            ))
        }

        RpcRequestPayload::Snapshot { symbols } => {
            let map = state
                .alpaca
                .snapshots(&snapshots::Params {
                    symbols,
                    ..Default::default()
                })
                .await
                .map_err(rpc_error("snapshot"))?;
            Ok(RpcResponsePayload::Snapshot(
                map.into_iter()
                    .map(|(symbol, snapshot)| SnapshotItem {
                        latest_trade: snapshot.latest_trade.map(|trade| trade_from_rest(symbol.clone(), trade)),
                        latest_quote: snapshot.latest_quote.map(|quote| quote_from_rest(symbol.clone(), quote)),
                        minute_bar: snapshot.minute_bar.map(bar_from_rest),
                        daily_bar: snapshot.daily_bar.map(bar_from_rest),
                        prev_daily_bar: snapshot.prev_daily_bar.map(bar_from_rest),
                        symbol,
                    })
                    .collect(),
            ))
        }

        RpcRequestPayload::ScreenerExport { parameter } => {
            let parameter = parameter_from_dto(parameter);
            let result = state
                .finviz
                .screener(parameter)
                .await
                .map_err(rpc_error("screener"))?;
            Ok(RpcResponsePayload::ScreenerExport(export_from_csv(result)))
        }

        RpcRequestPayload::MarketScreenerExport { parameter } => {
            let parameter = parameter_from_dto(parameter);
            let result = state
                .finviz
                .market_screener(parameter)
                .await
                .map_err(rpc_error("market screener"))?;
            Ok(RpcResponsePayload::MarketScreenerExport(
                export_from_csv(result),
            ))
        }

        RpcRequestPayload::Search { keyword, lang } => {
            let items: Vec<SearchItem> =
                web::search(&state.http, &keyword, &lang).await.map_err(rpc_error("search"))?;
            Ok(RpcResponsePayload::Search(items))
        }

        RpcRequestPayload::Translate { from, to, text } => {
            let text = web::translate(&state.http, &state.config.deepseek.api_key, &from, &to, &text)
                .await
                .map_err(rpc_error("translate"))?;
            Ok(RpcResponsePayload::Translate(text))
        }

        RpcRequestPayload::EconomicCalendar { date } => {
            let parameter = finviz_data::economic_week_for_date(&date).map_err(rpc_error("economic calendar"))?;
            let items = state
                .finviz
                .economic_calendar(parameter)
                .await
                .map_err(rpc_error("economic calendar"))?;
            let rows: Vec<EconomicItem> = finviz_data::economic_rows(items).map_err(rpc_error("economic calendar"))?;
            Ok(RpcResponsePayload::EconomicCalendar(rows))
        }

        RpcRequestPayload::EarningsCalendar { date } => {
            let items = state
                .finviz
                .earnings_calendar(&date)
                .await
                .map_err(rpc_error("earnings calendar"))?;
            let rows: Vec<EarningsItem> =
                finviz_data::earnings_rows(items).map_err(rpc_error("earnings calendar"))?;
            Ok(RpcResponsePayload::EarningsCalendar(rows))
        }

        RpcRequestPayload::IpoCalendar => {
            let items = web::ipo(&state.http).await.map_err(rpc_error("ipo"))?;
            Ok(RpcResponsePayload::IpoCalendar(items))
        }
    }
}

fn rpc_error(what: &'static str) -> impl Fn(anyhow::Error) -> String {
    move |err| format!("{what}: {err:#}")
}

fn bar_from_rest(bar: anysdk::alpaca_sdk::bars::Bar) -> Bar {
    Bar {
        time_ms: bar.time.timestamp_millis(),
        open: bar.open,
        high: bar.high,
        low: bar.low,
        close: bar.close,
        volume: bar.volume,
        vwap: bar.vwap,
        trade_count: bar.trade_count,
    }
}

fn quote_from_rest(
    symbol: String,
    quote: anysdk::alpaca_sdk::quotes::Quote,
) -> protocol::QuoteData {
    protocol::QuoteData {
        symbol,
        bid_exchange: quote.bid_exchange,
        bid_price: quote.bid_price,
        bid_size: quote.bid_size,
        ask_exchange: quote.ask_exchange,
        ask_price: quote.ask_price,
        ask_size: quote.ask_size,
        time_ms: quote.time.timestamp_millis(),
    }
}

fn trade_from_rest(
    symbol: String,
    trade: anysdk::alpaca_sdk::trades::Trade,
) -> protocol::TradeData {
    protocol::TradeData {
        symbol,
        id: trade.id,
        exchange: trade.exchange,
        price: trade.price,
        size: trade.size,
        conditions: trade.conditions,
        time_ms: trade.time.timestamp_millis(),
    }
}

fn export_from_csv(result: anysdk::finviz_sdk::CSVResult) -> ScreenerExport {
    ScreenerExport {
        headers: result.headers,
        rows: result.rows,
    }
}

/// DTO → finviz 参数(`screener_type` 线值反解析)。
fn parameter_from_dto(dto: ScreenerParameter) -> anysdk::finviz_sdk::screener::Parameter {
    anysdk::finviz_sdk::screener::Parameter {
        screener_type: match dto.screener_type.as_str() {
            "etf" => anysdk::finviz_sdk::screener::ScreenerType::ETF,
            _ => anysdk::finviz_sdk::screener::ScreenerType::Stocks,
        },
        order_by: dto.order_by,
        signal: dto.signal,
        filters: dto.filters,
        columns: dto.columns,
    }
}
