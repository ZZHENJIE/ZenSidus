//! The app-wide server connection hub: one socket to the server, one
//! union subscription, fanned out to every panel that asks.
//!
//! The underlying [`ServerClient`] supervises itself: it reconnects with
//! exponential backoff and (re)connects without ever failing after start.
//! The hub therefore only owns the per-consumer union subscription, the
//! *subscription replay* (it resends the unions on every `Connected` event)
//! and the surface of that status to the UI.

use std::collections::{BTreeMap, BTreeSet};

use gpui_kit::{
    App, Context, EntityId, EventEmitter, Hsla, SharedString, Task, component::ActiveTheme as _,
};

use anysdk::alpaca_stream::StockMessage;
use protocol::Channels;
use tokio::sync::broadcast;

use crate::tr;
use crate::utils::server::{ServerClient, ServerStatus as WireStatus};

pub use crate::utils::server::ServerStatus;

/// Emitted whenever [`ServerStatus`] changes.
pub enum HubEvent {
    StatusChanged,
}

/// UI helpers of the connection status (badges, tooltips, status dot).
impl ServerStatus {
    /// Localized one-word status, for badges and tooltips.
    pub fn label(&self) -> SharedString {
        match self {
            Self::Connected => tr!("app.settings.status.connected"),
            Self::Connecting => tr!("app.settings.status.connecting"),
            Self::Disconnected => tr!("app.settings.status.disconnected"),
            Self::Failed(_) => tr!("app.settings.status.failed"),
        }
    }

    /// The `Failed` reason, when present.
    pub fn reason(&self) -> Option<SharedString> {
        match self {
            Self::Failed(reason) => Some(reason.clone().into()),
            _ => None,
        }
    }

    /// Theme color of the status dot.
    pub fn color(&self, cx: &App) -> Hsla {
        let theme = cx.theme();
        match self {
            Self::Connected => theme.success,
            Self::Connecting => theme.warning,
            Self::Disconnected => theme.muted_foreground,
            Self::Failed(_) => theme.danger,
        }
    }
}

/// What one consumer wants from the shared stream: the symbols it follows,
/// whether it needs the trades/daily-bars channels, and whether it needs
/// top-of-book quotes. All channels cost bandwidth — a consumer that only
/// reads quotes (a whole-pool screener, for instance) must set
/// `trades: false`, otherwise every followed symbol drags in a full trade
/// firehose.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SymbolWish {
    pub symbols: BTreeSet<String>,
    pub trades: bool,
    pub quotes: bool,
}

/// Owns the single app-wide server client and the shared symbol subscription.
///
/// Consumers (panels) register the symbols they want under their own
/// [`EntityId`] as a [`SymbolWish`]; the hub subscribes the *union* on the
/// wire (quotes only for consumers that asked), so two panels watching NVDA
/// cost one server subscription. Messages fan out through
/// [`Hub::stock_messages`] (a fresh broadcast receiver per consumer,
/// which filters for its own symbols); the socket lives and dies with this
/// hub. Subscription replay is here: on every `Connected` event (first
/// connect or reconnect) the current unions are re-sent to the server.
pub struct Hub {
    status: ServerStatus,
    server: Option<ServerClient>,
    /// Handle on the app-wide tokio runtime. `None` only when that runtime
    /// failed to start; connect then reports `Failed` instead of panicking.
    runtime: Option<tokio::runtime::Handle>,
    /// Wanted symbols per consumer. The wire subscription is the union
    /// (quotes: the union over consumers that opted in).
    subscribers: BTreeMap<EntityId, SymbolWish>,
    /// Bumped on every teardown. Watcher tasks capture it and bail when it
    /// has moved on, so a manual `disconnect` reliably discards stale events.
    epoch: u64,
    /// Status + staleness watchers; dropped (cancelled) on teardown.
    watchers: Vec<Task<()>>,
}

impl EventEmitter<HubEvent> for Hub {}

impl Default for Hub {
    fn default() -> Self {
        Self::new(None)
    }
}

impl Hub {
    pub fn new(runtime: Option<tokio::runtime::Handle>) -> Self {
        Self {
            status: ServerStatus::Disconnected,
            server: None,
            runtime,
            subscribers: BTreeMap::new(),
            epoch: 0,
            watchers: Vec::new(),
        }
    }

    pub fn status(&self) -> &ServerStatus {
        &self.status
    }

    /// A fresh broadcast receiver on the stock messages, when the server is
    /// up. Consumers filter for their own symbols downstream.
    pub fn stock_messages(&self) -> Option<broadcast::Receiver<StockMessage>> {
        self.server.as_ref().map(|server| server.stock_messages())
    }

    /// The live server client (RPC + push outlet), if started.
    pub fn client(&self) -> Option<ServerClient> {
        self.server.clone()
    }

    /// The server clock estimate, when the server is up: lag 探针用它把
    /// `oldest_max` 补偿成本真延迟。
    pub fn clock(&self) -> Option<std::sync::Arc<crate::utils::server::ClockEstimate>> {
        self.server.as_ref().map(|server| server.clock())
    }

    /// Whether the server client is up (status may still be Connecting).
    pub fn connected(&self) -> bool {
        self.server.is_some()
    }

    /// Start the server client. The auth code comes from the connect dialog
    /// (a fresh TOTP code per connect — it is never persisted); address/TLS
    /// options are re-read from the settings on every attempt, so edits apply
    /// on the next connect. Re-entrant calls while up are ignored.
    pub fn connect(&mut self, code: String, cx: &mut Context<Self>) {
        if self.server.is_some() {
            return;
        }

        let Some(runtime) = self.runtime.clone() else {
            self.fail(anyhow::anyhow!("no tokio runtime available"), cx);
            return;
        };
        let Some(config) = cx.global::<super::AppState>().settings.server.config(&code) else {
            self.fail(anyhow::anyhow!("server address is empty"), cx);
            return;
        };

        match ServerClient::start(runtime, config) {
            Ok(server) => {
                self.server = Some(server);
                self.set_status(ServerStatus::Connecting, cx);
                self.watch_status(cx);
                self.watch_staleness(cx);
            }
            Err(err) => self.fail(err, cx),
        }
    }

    /// Report a failed attempt: `Failed` with the (truncated) reason and a
    /// message entry.
    fn fail(&mut self, err: anyhow::Error, cx: &mut Context<Self>) {
        tracing::warn!(
            "{}",
            tr!("app.log.stream_connect_failed", err = format!("{err:#}"))
        );
        let reason = short_reason(&err);
        self.set_status(ServerStatus::Failed(reason.to_string()), cx);
        crate::utils::message::error(
            format!("{} — {reason}", tr!("app.settings.status.failed")),
            err,
            cx,
        );
    }

    /// Mirror the client's status events into the hub's status and the
    /// message center. Every `Connected` (first connect *or* reconnect)
    /// replays the current unions — the server client keeps no desired state
    /// of its own.
    fn watch_status(&mut self, cx: &mut Context<Self>) {
        let Some(server) = self.server.as_ref() else {
            return;
        };
        let mut rx = server.status();
        let epoch = self.epoch;
        self.watchers.push(cx.spawn(async move |this, cx| {
            loop {
                match rx.recv().await {
                    Ok(WireStatus::Connected) => {
                        let _ = this.update(cx, |hub, cx| {
                            if hub.epoch != epoch {
                                return;
                            }
                            if hub.status == ServerStatus::Connecting {
                                crate::utils::message::success(
                                    tr!("app.settings.status.connected"),
                                    cx,
                                );
                            }
                            hub.set_status(ServerStatus::Connected, cx);
                            // (Re)subscribe everything consumers asked for,
                            // in one ordered burst per channel group.
                            hub.replay_subscriptions();
                        });
                    }
                    Ok(WireStatus::Connecting) => {
                        let _ = this.update(cx, |hub, cx| {
                            if hub.epoch != epoch {
                                return;
                            }
                            if hub.status == ServerStatus::Connected {
                                crate::utils::message::warning(
                                    tr!("app.settings.status.reconnecting"),
                                    tr!("app.log.stream_connection_lost"),
                                    cx,
                                );
                            }
                            hub.set_status(ServerStatus::Connecting, cx);
                        });
                    }
                    // A permanent failure (bad token): retrying is pointless,
                    // the client actor has already stopped.
                    Ok(WireStatus::Failed(reason)) => {
                        let _ = this.update(cx, |hub, cx| {
                            if hub.epoch != epoch {
                                return;
                            }
                            let err = anyhow::anyhow!(reason);
                            let short = short_reason(&err);
                            crate::utils::message::error(
                                format!("{} — {short}", tr!("app.settings.status.failed")),
                                err,
                                cx,
                            );
                            hub.set_status(ServerStatus::Failed(short.to_string()), cx);
                            hub.teardown();
                        });
                        break;
                    }
                    Ok(WireStatus::Disconnected) | Err(_) => break,
                }
            }
        }));
    }

    /// Watch the data feed for staleness: a long-lived connection through a
    /// flaky route can keep *flowing* while falling minutes behind (TCP
    /// collapse buffers the data instead of breaking the stream — the
    /// silence watchdog never fires because pongs still arrive). Owns one
    /// extra receiver; when the newest exchange timestamp falls >15s behind
    /// the local clock while quotes/trades are still being delivered, the
    /// link is force-reconnected for a fresh route (subscriptions replay via
    /// the `Connected` handler).
    fn watch_staleness(&mut self, cx: &mut Context<Self>) {
        let Some(server) = self.server.as_ref() else {
            return;
        };
        let mut rx = server.stock_messages();
        let epoch = self.epoch;
        self.watchers.push(cx.spawn(async move |this, cx| {
            const STALE_AFTER_MS: i64 = 15_000;
            const ARRIVAL_WINDOW_MS: i64 = 300_000;
            let mut last_arrival: Option<std::time::Instant> = None;
            let mut newest: Option<chrono::DateTime<chrono::Utc>> = None;
            loop {
                let timer = cx
                    .background_executor()
                    .timer(std::time::Duration::from_secs(5));
                tokio::select! {
                    message = rx.recv() => match message {
                        Ok(m) => {
                            let time = match &m {
                                StockMessage::Quote(q) => Some(q.time),
                                StockMessage::Trade(t) => Some(t.time),
                                StockMessage::MinuteBar(b)
                                | StockMessage::UpdatedBar(b)
                                | StockMessage::DailyBar(b) => Some(b.time),
                                _ => None,
                            };
                            if time.is_some() {
                                last_arrival = Some(std::time::Instant::now());
                            }
                            newest = time.or(newest);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                        Err(_) => break,
                    },
                    _ = timer => {
                        // Only judge while the feed is actively delivering:
                        // a silent feed (closed market) says nothing about
                        // the route.
                        let flowing = last_arrival
                            .map(|t| (t.elapsed().as_millis() as i64) < ARRIVAL_WINDOW_MS)
                            .unwrap_or(false);
                        let stale = newest
                            .map(|t| (chrono::Utc::now() - t).num_milliseconds() > STALE_AFTER_MS)
                            .unwrap_or(false);
                        if !(flowing && stale) {
                            continue;
                        }
                        let age = newest
                            .map(|t| (chrono::Utc::now() - t).num_milliseconds().max(0))
                            .unwrap_or(0);
                        let _ = this.update(cx, |hub, cx| {
                            if hub.epoch != epoch {
                                return;
                            }
                            crate::utils::message::warning(
                                tr!("app.log.stream_stale", age = age),
                                tr!("app.log.stream_stale", age = age),
                                cx,
                            );
                            if let Some(server) = hub.server.as_ref() {
                                server.force_reconnect();
                            }
                        });
                        break;
                    }
                }
            }
        }));
    }

    /// Stop the server client and cancel the watchers. The consumers' wanted
    /// symbols survive for the next connect.
    pub fn disconnect(&mut self, cx: &mut Context<Self>) {
        let live = self.server.is_some();
        self.teardown();
        if live && self.status != ServerStatus::Disconnected {
            self.set_status(ServerStatus::Disconnected, cx);
        }
    }

    /// Replace `subscriber`'s wanted symbols (and which channels it wants).
    /// Only the symbols that enter or leave the unions go on the wire, so
    /// shared symbols stay subscribed until the last consumer lets go. An
    /// empty symbol set drops the subscriber. While disconnected this only
    /// records the wish; the next `Connected` event replays the unions.
    pub fn set_symbols(&mut self, subscriber: EntityId, wish: SymbolWish) {
        let before_trades = self.trades_union();
        let before_quotes = self.quotes_union();
        if wish.symbols.is_empty() {
            self.subscribers.remove(&subscriber);
        } else {
            self.subscribers.insert(subscriber, wish);
        }
        let after_trades = self.trades_union();
        let after_quotes = self.quotes_union();

        let trades_added: Vec<String> = after_trades.difference(&before_trades).cloned().collect();
        let trades_removed: Vec<String> =
            before_trades.difference(&after_trades).cloned().collect();
        let quotes_added: Vec<String> = after_quotes.difference(&before_quotes).cloned().collect();
        let quotes_removed: Vec<String> =
            before_quotes.difference(&after_quotes).cloned().collect();

        let Some(server) = self.server.as_ref() else {
            return;
        };
        if !trades_added.is_empty() {
            server.subscribe(trades_added, Channels::TRADES | Channels::DAILY);
        }
        if !trades_removed.is_empty() {
            server.unsubscribe(trades_removed, Channels::TRADES | Channels::DAILY);
        }
        if !quotes_added.is_empty() {
            server.subscribe(quotes_added, Channels::QUOTES);
        }
        if !quotes_removed.is_empty() {
            server.unsubscribe(quotes_removed, Channels::QUOTES);
        }
    }

    /// Re-send the full unions (first connect or reconnect replay).
    fn replay_subscriptions(&self) {
        let Some(server) = self.server.as_ref() else {
            return;
        };
        let trades: Vec<String> = self.trades_union().into_iter().collect();
        let quotes: Vec<String> = self.quotes_union().into_iter().collect();
        if !trades.is_empty() {
            server.subscribe(trades, Channels::TRADES | Channels::DAILY);
        }
        if !quotes.is_empty() {
            server.subscribe(quotes, Channels::QUOTES);
        }
    }

    /// Stop the client and invalidate every captured callback via the epoch
    /// bump. Dropping the watcher tasks cancels them.
    fn teardown(&mut self) {
        self.epoch += 1;
        if let Some(server) = self.server.take() {
            server.close();
        }
        self.watchers.clear();
    }

    /// Symbols of consumers that asked for trades/daily bars: the
    /// trades-channel union.
    fn trades_union(&self) -> BTreeSet<String> {
        self.subscribers
            .values()
            .filter(|wish| wish.trades)
            .flat_map(|wish| wish.symbols.iter().cloned())
            .collect()
    }

    /// Symbols of consumers that asked for quotes: the quotes-channel union.
    fn quotes_union(&self) -> BTreeSet<String> {
        self.subscribers
            .values()
            .filter(|wish| wish.quotes)
            .flat_map(|wish| wish.symbols.iter().cloned())
            .collect()
    }

    fn set_status(&mut self, status: ServerStatus, cx: &mut Context<Self>) {
        if self.status != status {
            self.status = status;
            cx.emit(HubEvent::StatusChanged);
        }
        cx.notify();
    }
}

/// The error as one short line for the `Failed` payload and tooltips
/// (upstream error text can be arbitrarily long).
fn short_reason(err: &anyhow::Error) -> SharedString {
    let text = format!("{err:#}");
    match text.char_indices().nth(120) {
        Some((idx, _)) => format!("{}…", &text[..idx]).into(),
        None => text.into(),
    }
}
