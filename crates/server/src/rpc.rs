//! RPC 处理器:每个 [`RpcRequestPayload`] 变体一个分支,统一返回
//! `Result<RpcResponsePayload, String>`(错误串原样进 RpcResponse)。
//! `Bars` 用 `bars_all_with_progress`,每页拉完经 `progress_tx` 发
//! `RestProgress`(D2),终值照常走 RpcResponse。

use anysdk::alpaca_sdk::{bars, news, snapshots};
use tokio::sync::mpsc;

use crate::finviz_data;
use crate::state::AppState;
use crate::web;
use protocol::rpc::SearchItem;
use protocol::{
    Bar, EarningsItem, EconomicItem, Frame, NewsItem, RpcRequestPayload, RpcResponsePayload,
    ScreenerExport, ScreenerParameter, SnapshotItem,
};

/// 处理一个 RPC 请求。`rpc_id` 是请求 id(RestProgress 帧用它回填 req_id);
/// `progress_tx` 是该会话的下行出口(bars 进度帧专用,有界通道,与推送同源)。
pub async fn handle(
    state: &AppState,
    rpc_id: u32,
    progress_tx: &mpsc::Sender<Frame>,
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
            // 实测:Alpaca 的 bars 端点缺 `start` 时不报错而是返回空
            // (`{"bars":{}}`)。客户端没传 start(Chart 面板默认不带)时,
            // 服务端按 timeframe+limit 推导一个覆盖所需根数的起点,并用
            // `sort=desc` 单页拉取——desc 顺序下第一页就是最新的 limit 根,
            // 无需翻页(继续翻只会更旧);显式传了 start/end 的调用方维持
            // 原分页语义(bars_all_with_progress + RestProgress 进度帧)。
            let recent = start.is_none() && end.is_none() && limit > 0;
            // start 字符串先算好再构造 Params(timeframe 会被 move 进去)。
            let derived_start =
                recent.then(|| bars_start_default(&timeframe, limit, chrono::Utc::now()));
            let mut params = bars::Params {
                symbols,
                end,
                limit: (limit > 0).then_some(limit),
                timeframe,
                ..Default::default()
            };
            let all = if recent {
                params.sort = Some("desc".into());
                params.start = derived_start;
                let page = state
                    .alpaca
                    .bars(&params)
                    .await
                    .map_err(rpc_error("bars"))?;
                let _ = progress_tx.try_send(Frame::RestProgress {
                    req_id: rpc_id,
                    fetched: page.bars.values().map(Vec::len).sum::<usize>() as u64,
                });
                page.bars
            } else {
                params.start = start;
                let progress_tx = progress_tx.clone();
                state
                    .alpaca
                    .bars_all_with_progress(&params, |fetched| {
                        let _ = progress_tx.try_send(Frame::RestProgress {
                            req_id: rpc_id,
                            fetched: fetched as u64,
                        });
                    })
                    .await
                    .map_err(rpc_error("bars"))?
            };
            Ok(RpcResponsePayload::Bars(
                all.into_iter()
                    .map(|(symbol, mut bars)| {
                        // desc 拉回是"新→旧":截到 limit 根(单标的请求精确;
                        // 多标的共享 limit 时近似)再翻回升序(协议按 time_ms
                        // 升序约定,客户端 K 线依赖升序)。
                        if recent {
                            bars.truncate(limit as usize);
                            bars.reverse();
                        }
                        (symbol, bars.into_iter().map(bar_from_rest).collect())
                    })
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
                        latest_trade: snapshot
                            .latest_trade
                            .map(|trade| trade_from_rest(symbol.clone(), trade)),
                        latest_quote: snapshot
                            .latest_quote
                            .map(|quote| quote_from_rest(symbol.clone(), quote)),
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
            let mut parameter = parameter_from_dto(parameter);
            // 上游 SDK 已把 market 变体并入单一 `screener` 入口;市场版固定只要 No.+Ticker 两列
            parameter.columns = Some("0,1".to_string());
            let result = state
                .finviz
                .screener(parameter)
                .await
                .map_err(rpc_error("market screener"))?;
            Ok(RpcResponsePayload::MarketScreenerExport(export_from_csv(
                result,
            )))
        }

        RpcRequestPayload::Search { keyword, lang } => {
            let items: Vec<SearchItem> = web::search(&state.http, &keyword, &lang)
                .await
                .map_err(rpc_error("search"))?;
            Ok(RpcResponsePayload::Search(items))
        }

        RpcRequestPayload::Translate { from, to, text } => {
            let text = web::translate(
                &state.http,
                &state.config.deepseek.api_key,
                &from,
                &to,
                &text,
            )
            .await
            .map_err(rpc_error("translate"))?;
            Ok(RpcResponsePayload::Translate(text))
        }

        RpcRequestPayload::EconomicCalendar { date } => {
            let parameter = finviz_data::economic_week_for_date(&date)
                .map_err(rpc_error("economic calendar"))?;
            let items = state
                .finviz
                .economic_calendar(parameter)
                .await
                .map_err(rpc_error("economic calendar"))?;
            let rows: Vec<EconomicItem> =
                finviz_data::economic_rows(items).map_err(rpc_error("economic calendar"))?;
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
    crate::finviz_data::screener_cells(result)
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

/// 无 `start` 时的默认起点:按 timeframe 的每根跨度 × limit 估算所需
/// 跨度,再放大留出周末/节假日(日线及以上)与盘外休市(分钟周期)余量。
/// 返回 RFC-3339(UTC)字符串,直接进 Alpaca 的 `start` 参数。
fn bars_start_default(timeframe: &str, limit: u32, now: chrono::DateTime<chrono::Utc>) -> String {
    let start = match parse_timeframe_minutes(timeframe) {
        // 分钟周期:limit×bucket 是"市场时间"分钟,按 390 分钟/交易日折算
        // 成交易日,×2 覆盖周末,再 +1 天余量。
        Some(minutes) if minutes > 0 => {
            let market_minutes = (limit as i64) * minutes;
            let trading_days = (market_minutes + 389) / 390;
            now - chrono::Duration::days(trading_days * 2 + 1)
        }
        // 日/周/月/年:每根覆盖的日历天数 × limit,再加 60 天节假日余量。
        _ => {
            let days_per_bar = parse_timeframe_days(timeframe).unwrap_or(1);
            let days = ((limit as i64) * days_per_bar) * 12 / 10 + 60;
            now - chrono::Duration::days(days)
        }
    };
    start.to_rfc3339()
}

/// Alpaca timeframe 字符串 → 桶宽分钟("1Min"/"5Min"/"15Min"/"30Min" 原样;
/// "1Hour" 等 → 60 倍数;"Day/Week/Month/Year" → None)。
fn parse_timeframe_minutes(timeframe: &str) -> Option<i64> {
    if let Some(value) = timeframe.strip_suffix("Min") {
        return value.parse().ok();
    }
    if let Some(value) = timeframe.strip_suffix("Hour") {
        return value.parse::<i64>().ok().map(|value| value * 60);
    }
    None
}

/// Alpaca timeframe 字符串 → 每根 bar 覆盖的日历天数(天/周/月/年;
/// 分钟周期返回 None)。
fn parse_timeframe_days(timeframe: &str) -> Option<i64> {
    if let Some(value) = timeframe.strip_suffix("Day") {
        return value.parse().ok();
    }
    if let Some(value) = timeframe.strip_suffix("Week") {
        return value.parse::<i64>().ok().map(|value| value * 7);
    }
    if let Some(value) = timeframe.strip_suffix("Month") {
        return value.parse::<i64>().ok().map(|value| value * 30);
    }
    if let Some(value) = timeframe.strip_suffix("Year") {
        return value.parse::<i64>().ok().map(|value| value * 365);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike as _;

    fn utc(y: i32, m: u32, d: u32) -> chrono::DateTime<chrono::Utc> {
        chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, y, m, d, 0, 0, 0)
            .single()
            .unwrap()
    }

    #[test]
    fn timeframe_minutes() {
        assert_eq!(parse_timeframe_minutes("1Min"), Some(1));
        assert_eq!(parse_timeframe_minutes("5Min"), Some(5));
        assert_eq!(parse_timeframe_minutes("15Min"), Some(15));
        assert_eq!(parse_timeframe_minutes("30Min"), Some(30));
        assert_eq!(parse_timeframe_minutes("1Hour"), Some(60));
        assert_eq!(parse_timeframe_minutes("4Hour"), Some(240));
        assert_eq!(parse_timeframe_minutes("1Day"), None);
        assert_eq!(parse_timeframe_minutes("bogus"), None);
    }

    #[test]
    fn timeframe_days() {
        assert_eq!(parse_timeframe_days("1Day"), Some(1));
        assert_eq!(parse_timeframe_days("1Week"), Some(7));
        assert_eq!(parse_timeframe_days("1Month"), Some(30));
        assert_eq!(parse_timeframe_days("1Year"), Some(365));
        assert_eq!(parse_timeframe_days("5Min"), None);
        assert_eq!(parse_timeframe_days("bogus"), None);
    }

    #[test]
    fn bars_start_covers_requested_bars() {
        // 5m × 500 = 2500 市场分钟 ≈ 7 个交易日 → 15 个日历日。
        let now = utc(2026, 11, 20);
        let start = bars_start_default("5Min", 500, now);
        let dt = chrono::DateTime::parse_from_rfc3339(&start).unwrap();
        assert_eq!(dt.num_days_from_ce() - now.num_days_from_ce(), -15);

        // 1m × 1000 ≈ 3 个交易日 → 7 个日历日。
        let start = bars_start_default("1Min", 1000, now);
        let dt = chrono::DateTime::parse_from_rfc3339(&start).unwrap();
        assert_eq!(dt.num_days_from_ce() - now.num_days_from_ce(), -7);

        // 日线 × 500 ≈ 600 日历日 + 60 余量。
        let start = bars_start_default("1Day", 500, now);
        let dt = chrono::DateTime::parse_from_rfc3339(&start).unwrap();
        assert_eq!(dt.num_days_from_ce() - now.num_days_from_ce(), -660);

        // 周 × 500 = 3500 日历日 × 1.2 + 60。
        let start = bars_start_default("1Week", 500, now);
        let dt = chrono::DateTime::parse_from_rfc3339(&start).unwrap();
        assert_eq!(dt.num_days_from_ce() - now.num_days_from_ce(), -4260);

        // 月 × 500 = 15000 × 1.2 + 60。
        let start = bars_start_default("1Month", 500, now);
        let dt = chrono::DateTime::parse_from_rfc3339(&start).unwrap();
        assert_eq!(dt.num_days_from_ce() - now.num_days_from_ce(), -18060);
    }
}
