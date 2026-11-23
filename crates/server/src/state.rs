//! 服务端共享状态与常驻任务:上游订阅 actor(有序执行订阅命令)、
//! 推送泵(上游广播 → Frame → 按客户端订阅纯路由扇出)、状态日志。

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Utc;
use tokio::sync::mpsc;

use crate::config::Config;
use crate::executor::{AlpacaClient, FinvizClient};
use crate::hub::{HubDiff, SubscriptionHub};
use crate::mapping;
use protocol::{BarKind, Channels, Frame};

/// 上游订阅命令(actor 顺序执行,保序)。
#[derive(Debug)]
pub enum UpstreamCommand {
    Subscribe(anysdk::alpaca_stream::StockSubscription),
    Unsubscribe(anysdk::alpaca_stream::StockSubscription),
}

/// 会话令牌的存活时长:客户端断线后在这个窗口内重连可凭令牌免输动态码。
/// 超时(或服务端重启、令牌被轮换)则必须重新过 TOTP。
pub const SESSION_TTL: Duration = Duration::from_secs(30 * 60);

pub struct AppState {
    pub config: Config,
    pub http: reqwest::Client,
    pub alpaca: AlpacaClient,
    pub finviz: FinvizClient,
    /// 订阅命令入口(actor 顺序执行)。
    pub upstream_tx: mpsc::UnboundedSender<UpstreamCommand>,
    /// per-client 意愿聚合。
    pub hub: Mutex<SubscriptionHub>,
    /// 已认证客户端的下行出口(id → 有界 sender),推送扇出用。满了由
    /// [`Self::fan_out`] 丢新帧并计 `dropped_frames`——慢客户端吃丢帧,
    /// 而不是拖出分钟级延迟堆积。
    pub clients: Mutex<HashMap<u64, mpsc::Sender<Frame>>>,
    /// fan_out 因出站通道满而丢弃的帧数(30s 汇总进 ingest health 日志)。
    pub dropped_frames: AtomicU64,
    /// 会话令牌(认证成功时下发,客户端重连免输动态码)→ 最近使用时刻。
    /// 仅内存:服务端重启即全部失效,回到 TOTP 认证。
    pub sessions: Mutex<HashMap<String, Instant>>,
    pub next_client_id: AtomicU64,
}

impl AppState {
    pub fn new_client_id(&self) -> u64 {
        self.next_client_id.fetch_add(1, Ordering::Relaxed)
    }

    /// 签发一个新会话令牌(UUID v4)。轮换语义:旧令牌由调用方移除。
    pub fn new_session_token(&self) -> String {
        let token = uuid::Uuid::new_v4().to_string();
        self.sessions
            .lock()
            .unwrap()
            .insert(token.clone(), Instant::now());
        token
    }

    /// 客户端凭会话令牌请求重连:令牌有效(且未超 TTL)则**轮换**——移除
    /// 旧令牌、签发新令牌返回,被盗用的旧令牌只能用一次;无效返回 `None`
    /// (调用方回 `AuthFail`)。顺手惰性清理超 TTL 的过期令牌。
    pub fn resume_session(&self, token: &str) -> Option<String> {
        let mut sessions = self.sessions.lock().unwrap();
        let now = Instant::now();
        sessions.retain(|_, seen| now.duration_since(*seen) < SESSION_TTL);
        sessions.remove(token)?;
        let fresh = uuid::Uuid::new_v4().to_string();
        sessions.insert(fresh.clone(), Instant::now());
        Some(fresh)
    }

    /// 把差量发往上游(两个方向各自非空才发,避免空订阅帧打到上游)。
    pub fn apply_diff(&self, diff: HubDiff) {
        if !crate::hub::subscription_is_empty(&diff.subscribe)
            && let Err(err) = self
                .upstream_tx
                .send(UpstreamCommand::Subscribe(diff.subscribe))
        {
            tracing::warn!(error = %err, "upstream actor gone");
        }
        if !crate::hub::subscription_is_empty(&diff.unsubscribe)
            && let Err(err) = self
                .upstream_tx
                .send(UpstreamCommand::Unsubscribe(diff.unsubscribe))
        {
            tracing::warn!(error = %err, "upstream actor gone");
        }
    }

    /// 扇出一帧给已认证客户端。**纯路由分发**:行情帧只发给订阅了该
    /// 符号对应通道的客户端(hub 查表,一次 HashMap 查找,无任何规则
    /// 计算——server 定位是数据分发,VPS 不做行情计算);`News` 只发给
    /// 订阅了 NEWS 通道的客户端(客户端 attach 时发送订阅帧)。
    ///
    /// 有界通道 + `try_send`:客户端消费跟不上时丢掉这一帧(行情 Quote
    /// 本就 latest-wins,Trade/Bar 在极端突发下偶缺可接受),换来的是
    /// 队列深度有界 = 延迟上界有界。锁序固定 hub → clients(与
    /// session.rs 一致,无反向路径)。
    pub fn fan_out(&self, frame: &Frame) {
        // 行情帧的 (symbol, 通道位):Bar 按 kind 分流(分钟/更新 bar 走
        // BARS 位,日线走 DAILY 位——客户端分开订阅,互不串扰)。
        let routed = match frame {
            Frame::Quote(quote) => Some((quote.symbol.as_str(), Channels::QUOTES)),
            Frame::Trade(trade) => Some((trade.symbol.as_str(), Channels::TRADES)),
            Frame::Bar(bar) => Some((
                bar.symbol.as_str(),
                match bar.kind {
                    BarKind::Minute | BarKind::Updated => Channels::BARS,
                    BarKind::Daily => Channels::DAILY,
                },
            )),
            _ => None,
        };
        let is_news = matches!(frame, Frame::News(_));

        let hub = self.hub.lock().unwrap();
        let clients = self.clients.lock().unwrap();
        for (client_id, sender) in clients.iter() {
            let deliver = if let Some((symbol, channels)) = routed {
                hub.subscribed(*client_id, symbol, channels)
            } else if is_news {
                hub.news_wants(*client_id)
            } else {
                true
            };
            if !deliver {
                continue;
            }
            match sender.try_send(frame.clone()) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(_)) => {
                    self.dropped_frames.fetch_add(1, Ordering::Relaxed);
                }
                // 会话已死但注销尚未跑完;写循环退出路径会移除它。
                Err(mpsc::error::TrySendError::Closed(_)) => {}
            }
        }
    }
}

/// 常驻任务:上游订阅 actor + 推送泵 + 状态日志。
///
/// `stock` 句柄移交给订阅 actor(actor 在则连接在);`news` 句柄移交给
/// 新闻泵(同理保活——anysdk 流的最后一个句柄 drop 等价于关流)。
pub fn spawn(
    state: Arc<AppState>,
    stock: anysdk::alpaca_stream::StockStream,
    news: anysdk::alpaca_stream::NewsStream,
    mut upstream_rx: mpsc::UnboundedReceiver<UpstreamCommand>,
) {
    // 上游订阅 actor:单一任务顺序执行,保序;同时保活股票流句柄。
    // 广播接收端先于句柄移交创建(actor/pump 各拿各的)。
    let mut stock_rx = stock.messages();
    let mut news_rx = news.messages();
    let mut stock_status = stock.status();
    let mut news_status = news.status();

    tokio::spawn(async move {
        let _stream_alive = stock; // 保活:Drop 最后句柄 = 关流
        while let Some(command) = upstream_rx.recv().await {
            let result = match command {
                UpstreamCommand::Subscribe(subscription) => {
                    _stream_alive.subscribe(&subscription).await
                }
                UpstreamCommand::Unsubscribe(subscription) => {
                    _stream_alive.unsubscribe(&subscription).await
                }
            };
            if let Err(err) = result {
                tracing::warn!(error = %err, "upstream subscription command failed");
            }
        }
    });

    // 股票推送泵 + 摄取健康遥测:30s 汇总一条日志(窗口内消息数、交换所
    // 时间戳落后本地钟的最大值、fan_out 丢帧数)。max lag 大且无 drop =
    // 上游(Alpaca→server)段被限速;drop 计数增长 = 下发段慢客户端。
    // 这是区分两种延迟堆积的唯一手段,开盘时段尤其要看。
    let stock_pump_state = Arc::clone(&state);
    let max_ingest_lag_ms = Arc::new(AtomicI64::new(0));
    let ingested = Arc::new(AtomicU64::new(0));
    {
        let state = Arc::clone(&stock_pump_state);
        let max_ingest_lag_ms = Arc::clone(&max_ingest_lag_ms);
        let ingested = Arc::clone(&ingested);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(30));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                let lag_ms = max_ingest_lag_ms.swap(0, Ordering::Relaxed);
                let messages = ingested.swap(0, Ordering::Relaxed);
                let dropped = state.dropped_frames.swap(0, Ordering::Relaxed);
                if messages > 0 || dropped > 0 {
                    tracing::info!(
                        lag_ms_max = lag_ms,
                        messages,
                        fan_out_dropped = dropped,
                        "ingest health"
                    );
                }
            }
        });
    }
    tokio::spawn(async move {
        loop {
            match stock_rx.recv().await {
                Ok(message) => {
                    ingested.fetch_add(1, Ordering::Relaxed);
                    if let Some(ts) = message_ts_ms(&message) {
                        let lag = (Utc::now().timestamp_millis() - ts).max(0);
                        max_ingest_lag_ms.fetch_max(lag, Ordering::Relaxed);
                    }
                    if let Some(frame) = mapping::stock_message_to_frame(message) {
                        stock_pump_state.fan_out(&frame);
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "stock pump lagged; skipped upstream messages");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // 新闻推送泵(句柄随泵保活)。
    let news_pump_state = Arc::clone(&state);
    tokio::spawn(async move {
        let _stream_alive = news;
        loop {
            match news_rx.recv().await {
                Ok(message) => {
                    if let Some(frame) = mapping::news_message_to_frame(message) {
                        news_pump_state.fan_out(&frame);
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "news pump lagged; skipped upstream messages");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // 上游状态日志(断开/重连可见)。
    tokio::spawn(async move {
        while let Ok(event) = stock_status.recv().await {
            tracing::info!(?event, "upstream stock stream status");
        }
    });
    tokio::spawn(async move {
        while let Ok(event) = news_status.recv().await {
            tracing::info!(?event, "upstream news stream status");
        }
    });
}

/// 消息携带的交换所时间戳(毫秒);无时间戳的变体返回 `None`(不计延迟)。
fn message_ts_ms(message: &anysdk::alpaca_stream::StockMessage) -> Option<i64> {
    use anysdk::alpaca_stream::StockMessage;
    let time = match message {
        StockMessage::Quote(quote) => quote.time,
        StockMessage::Trade(trade) => trade.time,
        StockMessage::MinuteBar(bar)
        | StockMessage::UpdatedBar(bar)
        | StockMessage::DailyBar(bar) => bar.time,
        _ => return None,
    };
    Some(time.timestamp_millis())
}
