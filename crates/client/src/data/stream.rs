//! The subscription layer: one [`StreamHandle`] per stream-consuming panel,
//! owning the whole ~80-line lifecycle the panels used to hand-wire — the
//! wish plumbing (hub `SymbolWish` for the wire, `StreamSpec` for the
//! background filter), the pump (attach on `Connected`, fresh broadcast
//! receiver per connection; stop otherwise), and the unsubscribe paths
//! (`on_release` guard installed here, `on_removed` via
//! [`StreamHandle::unsubscribe`]).
//!
//! A consumer implements [`StreamConsumer`] (its stage name for the drain
//! diagnostics, optional status reactions, and the batch applier), keeps one
//! `StreamHandle` field and exposes it through
//! [`StreamConsumer::stream_handle`]. Subscription replay lives in the hub
//! (every `Connected` re-sends the unions); this layer only follows the
//! status to keep the pump in step.

use std::collections::BTreeSet;

use anysdk::alpaca_stream::StockMessage;
use gpui_kit::{App, Context, EntityId, Task};
use tokio::sync::broadcast;

use crate::data::api::NewsItem;
use crate::utils::state::stream_dispatch::{SharedSpec, StreamSpec, spawn_filter_stage};
use crate::utils::state::{AppState, HubEvent, SymbolWish};

// The connection status re-exported for the stream consumers (their
// `on_status` hooks match on it).
pub use crate::utils::state::ServerStatus;

/// What one consumer wants from the stream — the data layer's unified wish.
/// The hub's [`SymbolWish`] (which channel unions go on the wire) and the
/// filter stage's [`StreamSpec`] (what reaches the panel) are derived from
/// it, so the two can no longer drift apart per panel.
/// The hub's [`SymbolWish`] (which channel unions go on the wire) and the
/// filter stage's [`StreamSpec`] (what reaches the panel) are derived from
/// it, so the two can no longer drift apart per panel.
#[derive(Clone, Default)]
pub struct Wish {
    /// Followed symbols (uppercased); empty unsubscribes entirely.
    pub symbols: BTreeSet<String>,
    /// Trades + daily bars.
    pub trades: bool,
    /// Top-of-book quotes.
    pub quotes: bool,
    /// Minute/updated bars (intraday chart live updates; high rate).
    pub bars: bool,
    /// Quote rule (market screener): minimum size for a quote to matter
    /// (0 = any).
    pub min_size: i64,
    /// Quote rule (market screener): exchanges a quote must touch (bid or
    /// ask); empty = any.
    pub exchanges: Vec<String>,
}

impl Wish {
    fn to_hub_wish(&self) -> SymbolWish {
        SymbolWish {
            symbols: self.symbols.clone(),
            trades: self.trades,
            bars: self.bars,
            quotes: self.quotes,
        }
    }

    fn to_spec(&self) -> StreamSpec {
        StreamSpec {
            symbols: self.symbols.clone(),
            // Bar messages are filtered behind the trades gate (they ride
            // the same firehose), so a bars consumer must open it too; its
            // own `apply_batch` sorts the shapes out. The wire stays cheap
            // either way — the hub wish is what the server subscribes.
            trades: self.trades || self.bars,
            quotes: self.quotes,
            min_size: self.min_size,
            exchanges: self.exchanges.clone(),
        }
    }
}

/// Whether the server link is up right now — for consumers that seed their
/// state from the connection at construction time (live consumers get the
/// changes through [`StreamConsumer::on_status`] afterwards).
pub fn connected(cx: &App) -> bool {
    cx.global::<AppState>().hub.read(cx).status() == &ServerStatus::Connected
}

/// Subscribe to connection-status changes (every change fires the
/// callback). For consumers that are not live stream consumers but still
/// need to react to reconnects — e.g. invalidating states that could not
/// load while offline.
pub fn on_status_change<T: 'static>(
    cx: &mut Context<T>,
    on_change: impl Fn(&mut T, &ServerStatus, &mut Context<T>) + 'static,
) {
    let hub = cx.global::<AppState>().hub.clone();
    cx.subscribe(&hub, move |this, hub, _: &HubEvent, cx| {
        let status = hub.read(cx).status().clone();
        on_change(this, &status, cx);
    })
    .detach();
}

/// One side (bid or ask) of a top-of-book quote: price, size and the
/// exchange it rests on. [`BookSide::take`] applies the validity rule: a
/// price must be positive for the side to mean anything (a zero price is
/// the stream's "no quote on this side"). A one-sided quote is legal (e.g.
/// at the opening auction); the missing side renders as dashes.
#[derive(Clone, Default)]
pub struct BookSide {
    pub price: Option<f64>,
    pub size: Option<i64>,
    pub exchange: Option<String>,
}

impl BookSide {
    /// Take a side from a quote, `None` fields when that side has no quote.
    pub fn take(price: f64, size: i64, exchange: &str) -> Self {
        let (price, size) = if price > 0.0 {
            (Some(price), Some(size))
        } else {
            (None, None)
        };
        Self {
            price,
            size,
            exchange: (price.is_some()).then(|| exchange.to_string()),
        }
    }
}

/// A panel (or window) that consumes filtered stream batches. GPUI
/// entities are always sized; `Sized` keeps `Self` usable as the type
/// parameter of [`StreamHandle`] and [`Context`].
pub trait StreamConsumer: Sized + 'static {
    /// The drain stage's name in the `[lag]` diagnostics.
    const STAGE: &'static str;
    /// The consumer's stream handle (a field), for the lifecycle wiring
    /// installed by [`StreamHandle::new`].
    fn stream_handle(&mut self) -> &mut StreamHandle<Self>;
    /// Reaction to a hub status change, before the pump follows it
    /// (`Connected` reattaches the pump, anything else stops it — e.g. the
    /// quote panels clear their tables here: the last values are stale by
    /// then). Default: nothing.
    fn on_status(&mut self, _status: &ServerStatus, _cx: &mut Context<Self>) {}
    /// Apply one filtered batch (only messages the wish wants) — one
    /// entity update per batch is the whole point of the pump.
    fn apply_batch(&mut self, batch: Vec<StockMessage>, cx: &mut Context<Self>);
}

/// One consumer's stream subscription: wish plumbing, pump lifecycle and
/// unsubscribe paths, encapsulated.
pub struct StreamHandle<T: StreamConsumer> {
    subscriber: EntityId,
    /// The filter the background drain stage applies, kept in step with the
    /// wish ([`StreamHandle::set_wish`]).
    spec: SharedSpec,
    /// Live message pump for the current connection.
    pump: Option<Task<()>>,
    /// Keep T in the signature for the trait-bound plumbing.
    _consumer: std::marker::PhantomData<T>,
}

impl<T: StreamConsumer> StreamHandle<T> {
    /// Full lifecycle wiring for a fresh consumer: the hub-status observer
    /// (status reactions, pump attach/stop) and the release guard that
    /// unsubscribes no matter how the consumer dies.
    pub fn new(cx: &mut Context<T>) -> Self {
        let subscriber = cx.entity_id();
        let hub = cx.global::<AppState>().hub.clone();

        // Connection status drives the message pump: connected attaches a
        // fresh broadcast receiver, any other status cancels it (the old
        // receiver's channel died with the socket anyway).
        cx.subscribe(&hub, |this, hub, _: &HubEvent, cx| {
            let status = hub.read(cx).status().clone();
            this.on_status(&status, cx);
            match status {
                ServerStatus::Connected => this.stream_handle().attach_pump(cx),
                _ => this.stream_handle().stop_pump(),
            }
        })
        .detach();

        // Release the stream subscription no matter how this consumer dies.
        let release_hub = hub.clone();
        cx.on_release(move |_, cx| {
            release_hub.update(cx, |hub, _| {
                hub.set_symbols(subscriber, SymbolWish::default())
            });
        })
        .detach();

        Self {
            subscriber,
            spec: SharedSpec::default(),
            pump: None,
            _consumer: std::marker::PhantomData,
        }
    }

    /// Publish the consumer's wish: the background filter follows it
    /// immediately, the wire subscription follows through the hub (only the
    /// unions'增减 go on the wire; while disconnected the wish is just
    /// recorded for the next `Connected` replay). An empty symbol set
    /// unsubscribes the consumer's share entirely.
    pub fn set_wish(&mut self, wish: Wish, cx: &mut App) {
        *self.spec.write().expect("spec lock poisoned") = wish.to_spec();
        let hub = cx.global::<AppState>().hub.clone();
        hub.update(cx, |hub, _| {
            hub.set_symbols(self.subscriber, wish.to_hub_wish())
        });
    }

    /// (Re)attach the broadcast-receiver pump: one background drain stage
    /// over a fresh receiver of the hub's stream, batches forwarded to the
    /// consumer on the main thread. No-op while the server is down (the
    /// `Connected` event — first connect or reconnect — attaches it).
    pub fn attach_pump(&mut self, cx: &mut Context<T>) {
        let hub = cx.global::<AppState>().hub.clone();
        let clock = hub.read(cx).clock();
        let Some(rx) = hub.read(cx).stock_messages() else {
            return;
        };
        let spec = self.spec.clone();
        let (tx, mut rx_out) = tokio::sync::mpsc::channel(8);
        cx.background_executor()
            .spawn(spawn_filter_stage(T::STAGE, rx, spec, tx, clock))
            .detach();
        self.pump = Some(cx.spawn(async move |this, cx| {
            while let Some(batch) = rx_out.recv().await {
                if this
                    .update(cx, |this, cx| this.apply_batch(batch, cx))
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    /// Stop the pump: a consumer with no subscription (inactive tab,
    /// disconnected) must not wake for other consumers' messages.
    pub fn stop_pump(&mut self) {
        self.pump = None;
    }

    /// Drop the subscription (empty wish) and stop the pump — the dock
    /// panel's `on_removed` path (the release path is covered by the guard
    /// [`StreamHandle::new`] installs).
    pub fn unsubscribe(&mut self, cx: &mut App) {
        self.set_wish(Wish::default(), cx);
        self.stop_pump();
    }
}

/// A consumer of the market-wide news push. The server pushes news to every
/// connected client (no per-symbol gating, no subscription frame of its
/// own), so there is no wish here — only the pump and the follow-the-status
/// lifecycle.
pub trait NewsConsumer: Sized + 'static {
    /// The consumer's news pump (a field), for the lifecycle wiring
    /// installed by [`NewsPump::new`].
    fn news_pump(&mut self) -> &mut NewsPump;
    /// One pushed article, merged into the consumer's state.
    fn on_news(&mut self, item: &NewsItem, cx: &mut Context<Self>);
    /// Hub status changed; `newly_attached` is true when this event attached
    /// a pump that was not running before (a fresh connect — the moment to
    /// backfill what the offline startup skipped).
    fn on_news_status(
        &mut self,
        _status: &ServerStatus,
        _newly_attached: bool,
        _cx: &mut Context<Self>,
    ) {
    }
}

/// The market-wide news push pump: one broadcast receiver of the server's
/// NEWS channel, every article handed to [`NewsConsumer::on_news`] on the
/// main thread. It runs for the consumer's whole lifetime (news is
/// low-frequency, so consuming it costs nothing and the table stays current
/// without a catch-up fetch on re-activation) and ends with the consumer.
pub struct NewsPump {
    pump: Option<Task<()>>,
}

impl NewsPump {
    /// Full lifecycle wiring: the hub-status observer (attach on every
    /// change, status reactions to the consumer).
    pub fn new<T: NewsConsumer>(cx: &mut Context<T>) -> Self {
        let hub = cx.global::<AppState>().hub.clone();
        cx.observe(&hub, |this, hub, cx| {
            let status = hub.read(cx).status().clone();
            let was_attached = this.news_pump().is_attached();
            this.news_pump().attach(cx);
            this.on_news_status(&status, !was_attached, cx);
        })
        .detach();
        Self { pump: None }
    }

    /// Attach the pump (once — a live pump is never replaced). Silent no-op
    /// while the server is down; the hub-status observer calls again on
    /// `Connected`.
    pub fn attach<T: NewsConsumer>(&mut self, cx: &mut Context<T>) {
        if self.pump.is_some() {
            return;
        }
        let Ok(server) = AppState::server(cx) else {
            return;
        };
        let mut rx = server.news_messages();
        self.pump = Some(cx.spawn(async move |this, cx| {
            loop {
                match rx.recv().await {
                    Ok(news) => {
                        let item = NewsItem::from(news);
                        // An Err here means the consumer is gone: end the
                        // pump with it.
                        if this.update(cx, |this, cx| this.on_news(&item, cx)).is_err() {
                            break;
                        }
                    }
                    // A brief data gap only momentarily stales the feed:
                    // a news panel stays quiet in the message center.
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(
                            "{}",
                            crate::tr!("app.log.missed_stream_messages", count = skipped)
                        );
                    }
                    Err(_) => break,
                }
            }
        }));
    }

    /// Whether a pump is live.
    pub fn is_attached(&self) -> bool {
        self.pump.is_some()
    }
}
