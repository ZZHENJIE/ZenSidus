//! 服务端共享状态与常驻任务:上游订阅 actor(有序执行订阅命令)、
//! 推送泵(上游广播 → Frame → 扇出给全部客户端)、状态日志。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

use crate::config::Config;
use crate::executor::{AlpacaClient, FinvizClient};
use crate::hub::{HubDiff, SubscriptionHub};
use crate::mapping;
use protocol::Frame;

/// 上游订阅命令(actor 顺序执行,保序)。
#[derive(Debug)]
pub enum UpstreamCommand {
    Subscribe(anysdk::alpaca_stream::StockSubscription),
    Unsubscribe(anysdk::alpaca_stream::StockSubscription),
}

pub struct AppState {
    pub config: Config,
    pub http: reqwest::Client,
    pub alpaca: AlpacaClient,
    pub finviz: FinvizClient,
    /// 订阅命令入口(actor 顺序执行)。
    pub upstream_tx: mpsc::UnboundedSender<UpstreamCommand>,
    /// per-client 意愿聚合。
    pub hub: Mutex<SubscriptionHub>,
    /// 已认证客户端的下行出口(id → sender),推送扇出用。
    pub clients: Mutex<HashMap<u64, mpsc::UnboundedSender<Frame>>>,
    pub next_client_id: AtomicU64,
}

impl AppState {
    pub fn new_client_id(&self) -> u64 {
        self.next_client_id.fetch_add(1, Ordering::Relaxed)
    }

    /// 把差量发往上游(两个方向各自非空才发,避免空订阅帧打到上游)。
    pub fn apply_diff(&self, diff: HubDiff) {
        if !crate::hub::subscription_is_empty(&diff.subscribe)
            && let Err(err) = self.upstream_tx.send(UpstreamCommand::Subscribe(diff.subscribe))
        {
            tracing::warn!(error = %err, "upstream actor gone");
        }
        if !crate::hub::subscription_is_empty(&diff.unsubscribe)
            && let Err(err) = self.upstream_tx.send(UpstreamCommand::Unsubscribe(diff.unsubscribe))
        {
            tracing::warn!(error = %err, "upstream actor gone");
        }
    }

    /// 扇出一帧给全部已认证客户端。
    pub fn fan_out(&self, frame: &Frame) {
        let clients = self.clients.lock().unwrap();
        for sender in clients.values() {
            let _ = sender.send(frame.clone());
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
                UpstreamCommand::Subscribe(subscription) => _stream_alive.subscribe(&subscription).await,
                UpstreamCommand::Unsubscribe(subscription) => {
                    _stream_alive.unsubscribe(&subscription).await
                }
            };
            if let Err(err) = result {
                tracing::warn!(error = %err, "upstream subscription command failed");
            }
        }
    });

    // 股票推送泵。
    let stock_pump_state = Arc::clone(&state);
    tokio::spawn(async move {
        loop {
            match stock_rx.recv().await {
                Ok(message) => {
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
