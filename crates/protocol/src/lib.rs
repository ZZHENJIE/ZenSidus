//! 线协议:ZenSidus client ↔ server 的唯一通信契约。
//!
//! 传输:一条 TCP 长连接,帧格式 `[u32 LE 长度][postcard 编码的 Frame]`。
//! 设计见仓库根 `App.md` §2.2;D1(订阅携带 symbol 名)/ D2(RestProgress 进度)
//! 已并入。本 crate 只描述协议:全 owned DTO + postcard 派生 + 编解码助手,
//! client 与 server 各自映射到自己的内部类型。
//!
//! 会话流程:client 首发 [`Frame::Hello`](携带 TOTP 动态码)→ server 回
//! [`Frame::Welcome`];动态码无效/过期则 server 回 [`Frame::AuthFail`] 后断开;
//! 之后双向自由——client 周期 [`Frame::Ping`],server 以 [`Frame::Pong`]
//! 作流活性证据;订阅协商 [`Frame::Subscribe`] → [`Frame::Subscribed`];
//! 行情推送(Quote/Trade/Bar/News)与 RPC(请求-响应,按 id 路由)共用同一条
//! 连接。

pub mod rpc;
pub mod screener;

pub use rpc::{
    Bar, EarningsItem, EconomicItem, IpoItem, NewsItem, RpcRequestPayload, RpcResponsePayload,
    ScreenerExport, SearchItem, SnapshotItem,
};
pub use screener::{Cell, ScreenerParameter};

use serde::{Deserialize, Serialize};

// 订阅通道位标。`BARS` = 分钟/更新 bar(盘中逐笔更新,流量大);
// `DAILY` = 日线 bar(Watchlist 的涨跌锚点只需它)——两者分开,
// 客户端按需取用,不白耗带宽。
bitflags::bitflags! {
    /// 订阅通道位标(`BARS` = 分钟/更新 bar,`DAILY` = 日线,两者分开按需订阅)。
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Channels: u8 {
        const TRADES = 1;
        const QUOTES = 2;
        const BARS   = 4;
        const DAILY  = 8;
        const NEWS   = 16;
    }
}

/// 推送:top-of-book 报价(单边报价时缺失侧价格为 0、size 为 0)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuoteData {
    pub symbol: String,
    pub bid_exchange: String,
    pub bid_price: f64,
    pub bid_size: i64,
    pub ask_exchange: String,
    pub ask_price: f64,
    pub ask_size: i64,
    pub time_ms: i64,
}

/// 推送:逐笔成交。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TradeData {
    pub symbol: String,
    pub id: i64,
    pub exchange: String,
    pub price: f64,
    pub size: i64,
    pub conditions: Vec<String>,
    pub time_ms: i64,
}

/// 推送 bar 的种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BarKind {
    /// 分钟 bar(盘中滚动更新)。
    Minute,
    /// 迟到成交更新的 bar。
    Updated,
    /// 当日累积日线 bar。
    Daily,
}

/// 推送:bar(种类由 [`BarKind`] 区分;不带 vwap/trade_count,历史数据走 RPC)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BarData {
    pub symbol: String,
    pub kind: BarKind,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: i64,
    pub time_ms: i64,
}

/// 推送:市场新闻(全市场流,无关联标的过滤)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewsData {
    pub id: i64,
    pub symbols: Vec<String>,
    pub headline: String,
    pub source: String,
    pub url: String,
    pub time_ms: i64,
}

/// 客户端首帧携带的凭据:首连是认证器生成的 TOTP 动态码;重连是此前认证
/// 成功时服务端下发的会话令牌(服务端轮换下发新令牌,旧令牌即作废)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HelloAuth {
    Totp(String),
    Session(String),
}

/// 线上唯一帧类型。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Frame {
    // ── 会话 ──
    /// client 首帧。`auth` 两种凭据:TOTP 动态码(服务端 ±1 窗口校验)或
    /// 会话令牌(断线重连免输码)。
    Hello {
        auth: HelloAuth,
        app_version: String,
    },
    /// server 确认(顺带对时,替代旧 timestamp 端点);同时下发本次会话的
    /// 令牌(每次认证轮换),客户端在内存里持有它用于后续重连。
    Welcome {
        server_time_ms: i64,
        session_token: String,
    },
    /// server 拒绝认证(动态码无效/过期、会话令牌失效):发完即断开。客户端
    /// 收到后停止重试(认证失败不是瞬态错误),等人工输入新码。
    AuthFail {
        reason: String,
    },

    // ── 心跳 ──
    /// client → server,10s 间隔。
    Ping,
    /// server → client(流活性证据)。
    Pong,

    // ── 订阅协商(D1:symbols 为交易所符号名,服务端 intern 成内部 id)──
    Subscribe {
        id: u32,
        symbols: Vec<String>,
        channels: Channels,
    },
    Unsubscribe {
        id: u32,
        symbols: Vec<String>,
        channels: Channels,
    },
    /// server 对 Subscribe 的确认。
    Subscribed {
        id: u32,
    },

    // ── 行情推送(server → client)──
    Quote(QuoteData),
    Trade(TradeData),
    Bar(BarData),
    News(NewsData),

    // ── RPC(请求-响应,复用同一条 TCP;id 关联)──
    RpcRequest {
        id: u32,
        payload: rpc::RpcRequestPayload,
    },
    RpcResponse {
        id: u32,
        result: Result<rpc::RpcResponsePayload, String>,
    },
    /// bars RPC 的翻页进度(D2):`fetched` 为截至当前的累计条数;
    /// 终值仍经 [`Frame::RpcResponse`] 送达。
    RestProgress {
        req_id: u32,
        fetched: u64,
    },
}

/// 单帧长度上限(64 MiB):screener 全量导出是最大的合法帧,远小于此值;
/// 超限说明流已坏或被注入垃圾,直接报错弃连。
pub const MAX_FRAME_SIZE: u32 = 64 * 1024 * 1024;

/// 编码一帧:`[u32 LE 长度][postcard 帧]`。
pub fn encode_frame(frame: &Frame) -> Vec<u8> {
    let body = postcard::to_allocvec(frame).expect("Frame encode is infallible");
    let mut out = Vec::with_capacity(4 + body.len());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

/// 帧解码错误。
#[derive(Debug)]
pub enum FrameDecodeError {
    /// 长度字段超过 [`MAX_FRAME_SIZE`]。
    FrameTooLarge(u32),
    /// 帧体不是合法的 postcard `Frame`。
    BadFrame(postcard::Error),
}

impl std::fmt::Display for FrameDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameDecodeError::FrameTooLarge(len) => write!(f, "frame too large: {len} bytes"),
            FrameDecodeError::BadFrame(err) => write!(f, "bad frame: {err}"),
        }
    }
}

impl std::error::Error for FrameDecodeError {}

/// 增量式帧解码器:喂原始字节,吐完整帧。容忍半包与粘包,client/server
/// 共用同一实现。
#[derive(Debug, Default)]
pub struct FrameDecoder {
    buf: Vec<u8>,
}

impl FrameDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// 追加收到的字节,解码出其中所有完整帧(保持顺序)。
    pub fn feed(&mut self, bytes: &[u8]) -> Result<Vec<Frame>, FrameDecodeError> {
        self.buf.extend_from_slice(bytes);
        let mut frames = Vec::new();
        loop {
            if self.buf.len() < 4 {
                return Ok(frames);
            }
            let len = u32::from_le_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]]);
            if len > MAX_FRAME_SIZE {
                return Err(FrameDecodeError::FrameTooLarge(len));
            }
            let len = len as usize;
            if self.buf.len() < 4 + len {
                return Ok(frames);
            }
            let body = self.buf[4..4 + len].to_vec();
            self.buf.drain(..4 + len);
            let frame: Frame = postcard::from_bytes(&body).map_err(FrameDecodeError::BadFrame)?;
            frames.push(frame);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    /// 每个变体编码 → 解码逐字段相等。
    #[test]
    fn frame_roundtrips() {
        let frames = vec![
            Frame::Hello {
                auth: HelloAuth::Totp("123456".into()),
                app_version: "0.1.0".into(),
            },
            Frame::Hello {
                auth: HelloAuth::Session("uuid-token".into()),
                app_version: "0.1.0".into(),
            },
            Frame::Welcome {
                server_time_ms: 1_733_000_000_000,
                session_token: "uuid-token".into(),
            },
            Frame::AuthFail {
                reason: "invalid auth code".into(),
            },
            Frame::Ping,
            Frame::Pong,
            Frame::Subscribe {
                id: 7,
                symbols: vec!["AAPL".into(), "MSFT".into()],
                channels: Channels::TRADES | Channels::DAILY,
            },
            Frame::Unsubscribe {
                id: 7,
                symbols: vec!["AAPL".into()],
                channels: Channels::QUOTES,
            },
            Frame::Subscribed { id: 7 },
            Frame::Quote(QuoteData {
                symbol: "AAPL".into(),
                bid_exchange: "P".into(),
                bid_price: 189.97,
                bid_size: 300,
                ask_exchange: "K".into(),
                ask_price: 189.98,
                ask_size: 500,
                time_ms: 1_733_000_000_123,
            }),
            Frame::Trade(TradeData {
                symbol: "AAPL".into(),
                id: 6913,
                exchange: "D".into(),
                price: 182.13,
                size: 10,
                conditions: vec!["@".into(), "F".into()],
                time_ms: 1_733_000_000_123,
            }),
            Frame::Bar(BarData {
                symbol: "AAPL".into(),
                kind: BarKind::Daily,
                open: 178.26,
                high: 182.5,
                low: 177.7,
                close: 182.13,
                volume: 60_937,
                time_ms: 1_733_000_000_000,
            }),
            Frame::News(NewsData {
                id: 24_843_171,
                symbols: vec!["AAPL".into()],
                headline: "Headline".into(),
                source: "benzinga".into(),
                url: "https://example.com/a".into(),
                time_ms: 1_733_000_000_000,
            }),
            Frame::RpcRequest {
                id: 42,
                payload: rpc::RpcRequestPayload::Bars {
                    symbols: vec!["AAPL".into()],
                    timeframe: "1Day".into(),
                    start: Some("2026-01-02".into()),
                    end: None,
                    limit: 1000,
                },
            },
            Frame::RpcResponse {
                id: 42,
                result: Ok(rpc::RpcResponsePayload::Bars(BTreeMap::from([(
                    "AAPL".to_string(),
                    vec![rpc::Bar {
                        time_ms: 1_733_000_000_000,
                        open: 178.26,
                        high: 182.5,
                        low: 177.7,
                        close: 182.13,
                        volume: 60_937,
                        vwap: 180.1,
                        trade_count: 1727,
                    }],
                )]))),
            },
            Frame::RpcResponse {
                id: 43,
                result: Err("HTTP 403: invalid credentials".into()),
            },
            Frame::RestProgress {
                req_id: 42,
                fetched: 3000,
            },
        ];

        for frame in frames {
            let bytes = encode_frame(&frame);
            let mut decoder = FrameDecoder::new();
            let mut out = decoder.feed(&bytes).unwrap();
            assert_eq!(out.len(), 1, "{frame:?}");
            assert_eq!(out.pop().unwrap(), frame);
        }
    }

    /// 半包:字节逐段喂入,只在最后一 segment 出帧。
    #[test]
    fn decoder_handles_partial_frames() {
        let frame = Frame::Trade(TradeData {
            symbol: "SPY".into(),
            id: 1,
            exchange: "P".into(),
            price: 500.0,
            size: 100,
            conditions: vec![],
            time_ms: 0,
        });
        let bytes = encode_frame(&frame);
        let mut decoder = FrameDecoder::new();
        let chunks: Vec<&[u8]> = bytes.chunks(3).collect();
        let last = chunks.len() - 1;
        for (index, chunk) in chunks.iter().enumerate() {
            let frames = decoder.feed(chunk).unwrap();
            if index == last {
                assert_eq!(
                    frames,
                    vec![frame.clone()],
                    "final chunk completes the frame"
                );
            } else {
                assert!(frames.is_empty(), "chunk {index} cannot complete a frame");
            }
        }
    }

    /// 粘包:两帧一次喂入,按序出两帧;前导垃圾长度超限报 FrameTooLarge。
    #[test]
    fn decoder_handles_batched_frames_and_garbage() {
        let a = Frame::Ping;
        let b = Frame::Pong;
        let mut bytes = encode_frame(&a);
        bytes.extend_from_slice(&encode_frame(&b));

        let mut decoder = FrameDecoder::new();
        assert_eq!(decoder.feed(&bytes).unwrap(), vec![a, b]);

        // 垃圾长度字段:0xFFFF_FFFF 超上限。
        let err = decoder.feed(&0xFFFF_FFFFu32.to_le_bytes()).unwrap_err();
        assert!(matches!(err, FrameDecodeError::FrameTooLarge(0xFFFF_FFFF)));
    }

    /// Channels 位标往返。
    #[test]
    fn channels_roundtrip() {
        let all =
            Channels::TRADES | Channels::QUOTES | Channels::BARS | Channels::DAILY | Channels::NEWS;
        let bytes = postcard::to_allocvec(&all).unwrap();
        let back: Channels = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(back, all);
        assert_eq!(back, Channels::from_bits_truncate(0b1_1111));
    }
}
