//! RPC 请求-响应载荷与数据 DTO(全 owned;postcard 按位置编码,字段名不上
//! wire)。时间统一 `i64` 毫秒(epoch UTC)。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::screener::ScreenerParameter;
use crate::{QuoteData, TradeData};

/// RPC 请求载荷(client → server)。同一连接上以 [`crate::Frame::RpcRequest`]
/// 的 `id` 关联响应;`bars` 期间另有 [`crate::Frame::RestProgress`] 进度帧。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RpcRequestPayload {
    /// 个股新闻(最新在前)。
    News { symbols: Vec<String>, limit: u32 },
    /// 历史bars(multi-symbol;`timeframe` 形如 `1Day`/`5Min`)。
    Bars {
        symbols: Vec<String>,
        timeframe: String,
        start: Option<String>,
        end: Option<String>,
        limit: u32,
    },
    /// 快照(latest trade/quote + minute/daily/prev-daily bar)。
    Snapshot { symbols: Vec<String> },
    /// Finviz 筛选导出(token 在服务端)。
    ScreenerExport { parameter: ScreenerParameter },
    /// 市场筛选器第一轮圈池(同为 Finviz 导出;独立变体便于日后分岔)。
    MarketScreenerExport { parameter: ScreenerParameter },
    /// 标的搜索(futunn 代理)。
    Search { keyword: String, lang: String },
    /// 翻译(DeepSeek 代理)。
    Translate { from: String, to: String, text: String },
    /// 经济日历:`date` 所在周(服务端吸附到周一、取 Mon–Fri 窗口)。
    EconomicCalendar { date: String },
    /// 财报日历:单个交易日。
    EarningsCalendar { date: String },
    /// IPO 日历(全量)。
    IpoCalendar,
}

/// RPC 响应载荷(server → client),变体与 [`RpcRequestPayload`] 一一对应。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RpcResponsePayload {
    News(Vec<NewsItem>),
    Bars(BTreeMap<String, Vec<Bar>>),
    Snapshot(Vec<SnapshotItem>),
    ScreenerExport(ScreenerExport),
    MarketScreenerExport(ScreenerExport),
    Search(Vec<SearchItem>),
    Translate(String),
    EconomicCalendar(Vec<EconomicItem>),
    EarningsCalendar(Vec<EarningsItem>),
    IpoCalendar(Vec<IpoItem>),
}

/// 一条新闻(RPC 版:比推送帧多 `url` 可空与 `symbols` 关联列表,字段与
/// 推送 [`NewsData`] 对齐,`time_ms` 为发布时间)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewsItem {
    pub id: i64,
    pub symbols: Vec<String>,
    pub headline: String,
    pub source: String,
    pub url: Option<String>,
    pub time_ms: i64,
}

impl From<crate::NewsData> for NewsItem {
    fn from(news: crate::NewsData) -> Self {
        Self {
            id: news.id,
            symbols: news.symbols,
            headline: news.headline,
            source: news.source,
            url: (!news.url.is_empty()).then_some(news.url),
            time_ms: news.time_ms,
        }
    }
}

/// 一根历史 bar(REST 版:保留 vwap / trade_count;推送版
/// [`BarData`] 按协议草图不带这两个字段)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bar {
    pub time_ms: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: i64,
    pub vwap: f64,
    pub trade_count: i64,
}

/// 一个标的的快照(ETF 联价器锚定用 `daily_bar` 的 open)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotItem {
    pub symbol: String,
    pub latest_trade: Option<TradeData>,
    pub latest_quote: Option<QuoteData>,
    pub minute_bar: Option<Bar>,
    pub daily_bar: Option<Bar>,
    pub prev_daily_bar: Option<Bar>,
}

/// Finviz 导出表:表头 + 行(单元格保留导出原文)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenerExport {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

/// 一条搜索建议。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchItem {
    pub name: String,
    pub symbol: String,
}

/// 经济日历一行(US-Eastern 墙钟已由服务端解析为毫秒时间戳;
/// `impact` 为 Finviz 影响级别 1-3)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EconomicItem {
    pub event: String,
    pub time_ms: i64,
    pub impact: u8,
    pub r#for: Option<String>,
    pub actual: Option<String>,
    pub expected: Option<String>,
    pub prior: Option<String>,
}

/// 财报日历一行(数值列为百万美元/百分比;空或不可解析的单元格为 `None`)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EarningsItem {
    /// 毫秒时间戳;导出缺时间部分时锚定当日零点(US-Eastern)。
    pub time_ms: i64,
    pub symbol: String,
    pub company: String,
    pub market_cap: Option<f64>,
    pub eps_estimate: Option<f64>,
    pub eps_actual: Option<f64>,
    pub eps_surprise: Option<f64>,
    pub eps_gaap_estimate: Option<f64>,
    pub eps_gaap_actual: Option<f64>,
    pub eps_gaap_surprise: Option<f64>,
    pub revenue_estimate: Option<f64>,
    pub revenue_actual: Option<f64>,
    pub revenue_surprise: Option<f64>,
    pub one_day_price_reaction: Option<f64>,
}

/// 一条 IPO 记录(源:iposcoop.com 抓取)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IpoItem {
    pub company: String,
    pub company_des_url: String,
    pub symbol: String,
    pub managers: String,
    pub shares_millions: f64,
    pub price_low: f64,
    pub price_high: f64,
    pub expected_date: String,
}
