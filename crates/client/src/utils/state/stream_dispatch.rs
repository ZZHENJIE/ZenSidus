//! The background stage between the shared stream and the UI pumps.
//!
//! The broadcast channel carries the whole firehose the hub subscribed to
//! (a 600-symbol pool prints thousands of messages per second during market
//! hours), while the UI can only commit a fraction of that on the main
//! thread. This stage drains the channel **on worker threads** — keeping it
//! empty so the socket never ages — filters and coalesces per the panel's
//! [`StreamSpec`], and forwards only relevant batches to the foreground
//! pump through a bounded channel. Under overload the excess is dropped
//! here instead of accumulating: the UI stays near real time by design.

use std::{
    collections::BTreeSet,
    future::Future,
    sync::{Arc, RwLock},
};

use anysdk::alpaca_stream::{StockMessage, stocks::QuoteMessage};
use tokio::sync::broadcast;

/// What a panel consumes from the stream, as the background stage sees it.
/// Mutate through the shared handle ([`SharedSpec`]) whenever the panel's
/// selection or rules change; the stage picks it up on the next batch.
#[derive(Clone, Default)]
pub struct StreamSpec {
    /// Followed symbols (uppercased); empty forwards nothing.
    pub symbols: BTreeSet<String>,
    /// Consume the trades + daily-bars channels.
    pub trades: bool,
    /// Consume the quotes channel.
    pub quotes: bool,
    /// Market-screener rule: minimum size for a quote to matter (0 = any).
    pub min_size: i64,
    /// Market-screener rule: exchange codes a quote must touch (ax or bid);
    /// empty = any exchange.
    pub exchanges: Vec<String>,
}

/// The spec handle shared between the panel (writer, main thread) and the
/// background stage (reader).
pub type SharedSpec = Arc<RwLock<StreamSpec>>;

/// Spawn the background drain stage for one panel on the executor's worker
/// threads. Returns the foreground receiver of filtered batches (each batch
/// = only messages the spec wants). Ends when the stream closes. `clock`
/// (relay 对时估计)存在时,诊断行把 `oldest_max` 换算成本真延迟——原始
/// 读数 = 本机时钟偏移 + 真延迟,不看补偿值没法验收线路。
pub fn spawn_filter_stage(
    panel: &'static str,
    mut source: broadcast::Receiver<StockMessage>,
    spec: SharedSpec,
    tx: tokio::sync::mpsc::Sender<Vec<StockMessage>>,
    clock: Option<Arc<crate::utils::relay::ClockEstimate>>,
) -> impl Future<Output = ()> {
    let mut probe = Probe::new(clock);
    async move {
        loop {
            let batch = match source.recv().await {
                Ok(first) => {
                    let mut batch = vec![first];
                    while let Ok(message) = source.try_recv() {
                        batch.push(message);
                    }
                    batch
                }
                // Lagged: the stage jumped forward — the dropped messages
                // are gone; keep going from the oldest retained one.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    probe.lagged(n);
                    continue;
                }
                Err(_) => break,
            };

            let (forwarded, oldest) = {
                let spec = spec.read().expect("spec lock poisoned");
                let relevant: Vec<StockMessage> = batch
                    .iter()
                    .filter(|message| spec.wants(message))
                    .cloned()
                    .collect();
                let oldest = batch.iter().filter_map(oldest_time).min();
                (relevant, oldest)
            };
            probe.step(batch.len(), forwarded.len(), oldest);
            if probe.due() {
                probe.print(panel);
            }

            if forwarded.is_empty() {
                continue;
            }
            // Bounded and non-blocking: when the UI cannot keep up, the
            // stage drops whole batches instead of growing a backlog —
            // the panels degrade to skipping rows, never to stale data.
            if tx.try_send(forwarded).is_err() {
                probe.dropped_batch();
            }
        }
    }
}

fn oldest_time(message: &StockMessage) -> Option<chrono::DateTime<chrono::Utc>> {
    match message {
        StockMessage::Quote(quote) => Some(quote.time),
        StockMessage::Trade(trade) => Some(trade.time),
        StockMessage::MinuteBar(bar)
        | StockMessage::UpdatedBar(bar)
        | StockMessage::DailyBar(bar) => Some(bar.time),
        _ => None,
    }
}

impl StreamSpec {
    fn wants(&self, message: &StockMessage) -> bool {
        match message {
            StockMessage::Quote(quote) => self.quotes && self.quote_qualifies(quote),
            StockMessage::Trade(trade) => self.trades && self.wants_symbol(&trade.symbol),
            StockMessage::MinuteBar(bar)
            | StockMessage::UpdatedBar(bar)
            | StockMessage::DailyBar(bar) => self.trades && self.wants_symbol(&bar.symbol),
            _ => false,
        }
    }

    fn wants_symbol(&self, symbol: &str) -> bool {
        self.symbols
            .iter()
            .any(|known| known.eq_ignore_ascii_case(symbol))
    }

    /// The market-screener rule as one predicate: a quote matters when its
    /// bid or ask side rests on one of the preset's exchanges with a size
    /// meeting the preset's minimum. Empty exchanges = any exchange.
    fn quote_qualifies(&self, quote: &QuoteMessage) -> bool {
        let side = |exchange: &str, size: i64| -> bool {
            (self.exchanges.is_empty()
                || self
                    .exchanges
                    .iter()
                    .any(|known| known.eq_ignore_ascii_case(exchange)))
                && (self.min_size <= 0 || size >= self.min_size)
        };
        side(&quote.bid_exchange, quote.bid_size) || side(&quote.ask_exchange, quote.ask_size)
    }
}

/// `[lag]` 诊断行默认关闭(生产 stderr 无噪声);设 `ZENSIDUS_LAG=1` 重新
/// 打开——验收线路延迟(对时补偿后的 oldest_max)、排查掉批时用。
fn lag_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("ZENSIDUS_LAG").is_ok_and(|v| v != "0"))
}

/// Console diagnostics for one stage, one line per 2 seconds: total in,
/// forwarded out, the oldest message age in the window, and dropped batches.
/// With the relay clock estimate available the age is compensated to the
/// true end-to-end latency (raw reading = clock offset + real age).
struct Probe {
    clock: Option<Arc<crate::utils::relay::ClockEstimate>>,
    last_print: Option<std::time::Instant>,
    total_in: usize,
    forwarded: usize,
    /// Raw age by the local clock (offset included).
    max_age_ms: i64,
    /// Offset-compensated age — the number to judge the route by.
    max_true_age_ms: i64,
    dropped: usize,
}

impl Probe {
    fn new(clock: Option<Arc<crate::utils::relay::ClockEstimate>>) -> Self {
        Self {
            clock,
            last_print: None,
            total_in: 0,
            forwarded: 0,
            max_age_ms: 0,
            max_true_age_ms: 0,
            dropped: 0,
        }
    }

    fn lagged(&mut self, n: u64) {
        if lag_enabled() {
            eprintln!("[lag] stage skipped {n} messages (receiver behind)");
        }
    }

    fn step(
        &mut self,
        total_in: usize,
        forwarded: usize,
        oldest: Option<chrono::DateTime<chrono::Utc>>,
    ) {
        self.total_in += total_in;
        self.forwarded += forwarded;
        if let Some(oldest) = oldest {
            let raw = (chrono::Utc::now() - oldest).num_milliseconds().max(0);
            // offset = 服务端钟 − 本地钟;raw 里含 +offset,减掉即真延迟
            // (精度 ±RTT/2;无样本时不补偿)。
            let true_age = match self.clock.as_ref().and_then(|clock| clock.get()) {
                Some((offset_ms, _rtt)) => (raw - offset_ms).max(0),
                None => raw,
            };
            self.max_age_ms = self.max_age_ms.max(raw);
            self.max_true_age_ms = self.max_true_age_ms.max(true_age);
        }
    }

    fn dropped_batch(&mut self) {
        self.dropped += 1;
    }

    fn due(&self) -> bool {
        self.last_print
            .is_none_or(|last| last.elapsed().as_secs_f64() >= 2.0)
    }

    fn print(&mut self, panel: &'static str) {
        let seconds = self
            .last_print
            .map(|last| last.elapsed().as_secs_f64().max(0.5))
            .unwrap_or(2.0);
        self.last_print = Some(std::time::Instant::now());
        let total_rate = (self.total_in as f64 / seconds).round() as usize;
        let forwarded_rate = (self.forwarded as f64 / seconds).round() as usize;
        if !lag_enabled() {
            return;
        }
        if self.clock.is_some() {
            eprintln!(
                "[lag] {panel}: in~{total_rate}/s fwd~{forwarded_rate}/s oldest_max={}ms raw={}ms dropped={}",
                self.max_true_age_ms, self.max_age_ms, self.dropped
            );
        } else {
            eprintln!(
                "[lag] {panel}: in~{total_rate}/s fwd~{forwarded_rate}/s oldest_max={}ms dropped={}",
                self.max_age_ms, self.dropped
            );
        }
        self.total_in = 0;
        self.forwarded = 0;
        self.max_age_ms = 0;
        self.max_true_age_ms = 0;
        self.dropped = 0;
    }
}
