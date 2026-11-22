//! 上游(anysdk)消息 → 线协议帧的映射。推送帧字段按 App.md §2.2 草图:
//! Quote 丢 conditions/tape(带宽),Trade 保 conditions 丢 tape,
//! Bar 由变体区分钟/更新/日线。非行情消息(订阅 echo、corrections、
//! LULD 等)一期不下发。

use protocol::{BarData, BarKind, Frame, NewsData, QuoteData, TradeData};

/// 上游股票流消息 → 下行帧;`None` = 该消息不下发。
pub fn stock_message_to_frame(message: anysdk::alpaca_stream::StockMessage) -> Option<Frame> {
    use anysdk::alpaca_stream::StockMessage;
    let frame = match message {
        StockMessage::Quote(quote) => Frame::Quote(QuoteData {
            symbol: quote.symbol,
            bid_exchange: quote.bid_exchange,
            bid_price: quote.bid_price,
            bid_size: quote.bid_size,
            ask_exchange: quote.ask_exchange,
            ask_price: quote.ask_price,
            ask_size: quote.ask_size,
            time_ms: ms(quote.time),
        }),
        StockMessage::Trade(trade) => Frame::Trade(TradeData {
            symbol: trade.symbol,
            id: trade.id,
            exchange: trade.exchange,
            price: trade.price,
            size: trade.size,
            conditions: trade.conditions,
            time_ms: ms(trade.time),
        }),
        StockMessage::MinuteBar(bar) => Frame::Bar(bar_data(BarKind::Minute, bar)),
        StockMessage::UpdatedBar(bar) => Frame::Bar(bar_data(BarKind::Updated, bar)),
        StockMessage::DailyBar(bar) => Frame::Bar(bar_data(BarKind::Daily, bar)),
        // 订阅 echo / corrections / LULD / 状态 / ping 等不下发。
        _ => return None,
    };
    Some(frame)
}

/// 上游新闻流消息 → 下行帧;非文章消息(订阅 echo / success / error)不下发
/// (error 由状态日志承担)。
pub fn news_message_to_frame(message: anysdk::alpaca_stream::NewsMessage) -> Option<Frame> {
    let anysdk::alpaca_stream::NewsMessage::News(article) = message else {
        return None;
    };
    Some(Frame::News(NewsData {
        id: article.id,
        symbols: article.symbols,
        headline: article.headline,
        source: article.source.unwrap_or_default(),
        url: article.url.unwrap_or_default(),
        time_ms: ms(article.created_at),
    }))
}

fn bar_data(kind: BarKind, bar: anysdk::alpaca_stream::stocks::BarMessage) -> BarData {
    BarData {
        symbol: bar.symbol,
        kind,
        open: bar.open,
        high: bar.high,
        low: bar.low,
        close: bar.close,
        volume: bar.volume,
        time_ms: ms(bar.time),
    }
}

fn ms(time: chrono::DateTime<chrono::Utc>) -> i64 {
    time.timestamp_millis()
}

/// 保留时间戳的可测样例:直接用固定时间断言映射正确性。
#[cfg(test)]
mod tests {
    use super::*;
    use anysdk::alpaca_stream::news::NewsArticle;
    use anysdk::alpaca_stream::stocks::{BarMessage, QuoteMessage, TradeMessage};
    use anysdk::alpaca_stream::{NewsMessage, StockMessage};
    use chrono::{DateTime, TimeZone, Utc};

    fn t() -> DateTime<Utc> {
        Utc.timestamp_millis_opt(1_733_000_000_123).unwrap()
    }

    #[test]
    fn quote_maps_and_drops_conditions_tape() {
        let message = StockMessage::Quote(QuoteMessage {
            symbol: "AAPL".into(),
            ask_exchange: "P".into(),
            ask_price: 189.98,
            ask_size: 500,
            bid_exchange: "K".into(),
            bid_price: 189.97,
            bid_size: 300,
            conditions: vec!["@".into()],
            time: t(),
            tape: Some("C".into()),
        });
        let Some(Frame::Quote(quote)) = stock_message_to_frame(message) else {
            panic!("expected quote frame");
        };
        assert_eq!(quote.symbol, "AAPL");
        assert_eq!(quote.bid_price, 189.97);
        assert_eq!(quote.bid_size, 300);
        assert_eq!(quote.ask_price, 189.98);
        assert_eq!(quote.ask_size, 500);
        assert_eq!(quote.time_ms, 1_733_000_000_123);
    }

    #[test]
    fn trade_keeps_conditions_and_drops_tape() {
        let message = StockMessage::Trade(TradeMessage {
            symbol: "SPY".into(),
            id: 6913,
            exchange: "D".into(),
            price: 500.5,
            size: 42,
            conditions: vec!["@".to_string(), "F".to_string()],
            time: t(),
            tape: Some("C".into()),
        });
        let Some(Frame::Trade(trade)) = stock_message_to_frame(message) else {
            panic!("expected trade frame");
        };
        assert_eq!(trade.conditions, vec!["@", "F"]);
        assert_eq!(trade.price, 500.5);
        assert_eq!(trade.size, 42);
        assert_eq!(trade.time_ms, 1_733_000_000_123);
    }

    #[test]
    fn bars_map_with_kind() {
        let bar = BarMessage {
            symbol: "AAPL".into(),
            open: 178.0,
            high: 179.0,
            low: 177.0,
            close: 178.5,
            volume: 1000,
            vwap: 178.2,
            trade_count: 42,
            time: t(),
        };
        for (message, kind) in [
            (StockMessage::MinuteBar(bar.clone()), BarKind::Minute),
            (StockMessage::UpdatedBar(bar.clone()), BarKind::Updated),
            (StockMessage::DailyBar(bar.clone()), BarKind::Daily),
        ] {
            let Some(Frame::Bar(data)) = stock_message_to_frame(message) else {
                panic!("expected bar frame");
            };
            assert_eq!(data.kind, kind);
            assert_eq!(data.close, 178.5);
            assert_eq!(data.time_ms, 1_733_000_000_123);
        }
    }

    /// 非行情消息一律不下发。
    #[test]
    fn control_messages_map_to_none() {
        assert!(stock_message_to_frame(StockMessage::Ping {}).is_none());
        assert!(stock_message_to_frame(StockMessage::Success { msg: "ok".into() }).is_none());
        assert!(
            stock_message_to_frame(StockMessage::ServerError {
                code: "401".into(),
                msg: "nope".into()
            })
            .is_none()
        );
        assert!(news_message_to_frame(NewsMessage::Success { msg: "ok".into() }).is_none());
    }

    #[test]
    fn news_maps_with_defaults_for_missing_source_url() {
        let message = NewsMessage::News(NewsArticle {
            id: 24_843_171,
            headline: "Headline".into(),
            summary: String::new(),
            author: "a".into(),
            created_at: t(),
            updated_at: t(),
            content: String::new(),
            url: None,
            symbols: vec!["AAPL".into()],
            source: None,
        });
        let Some(Frame::News(news)) = news_message_to_frame(message) else {
            panic!("expected news frame");
        };
        assert_eq!(news.id, 24_843_171);
        assert_eq!(news.symbols, vec!["AAPL"]);
        assert_eq!(news.source, "");
        assert_eq!(news.url, "");
        assert_eq!(news.time_ms, 1_733_000_000_123);
    }
}
