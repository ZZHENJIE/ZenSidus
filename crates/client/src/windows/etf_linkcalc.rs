//! ETF link calculator. An ETF and its index are linked by the day's
//! opening prices (the anchor pair the move is measured from), the ETF's
//! leverage multiple, its direction and two offsets. The window shows both
//! symbols' live top-of-book quotes from the shared Alpaca stream, and for
//! each live ETF quote level the index price at which that level would
//! trade on the bid and on the ask side (the *break* prices). The link
//! parameters are edited in a dialog ([`params_dialog`]) opened from the
//! title bar; they are persisted per window instance.

mod params_dialog;

use std::collections::BTreeSet;

use anysdk::alpaca_stream::StockMessage;
use anysdk::alpaca_stream::stocks::BarMessage;
use chrono::NaiveDate;
use relay_protocol::{RpcRequestPayload, RpcResponsePayload};
use gpui_kit::{
    App, Context, Div, EntityId, Hsla, IntoElement, ParentElement as _, Render, SharedString,
    Styled as _, Task, Window,
    base::{h_flex, v_flex},
    component::{ActiveTheme as _, input::InputState},
    div,
};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::{
    tr,
    utils::AppIcon,
    utils::state::{RelayHubEvent, RelayStatus, AppState, SymbolWish},
    widgets::icon_button,
    windows::{AppWindow, WindowType, hook_should_close},
};

/// The ETF ↔ index link, edited in the parameters dialog: the two symbols,
/// the day's opening prices (the anchor pair), the ETF's leverage multiple,
/// its direction and two offsets. Both offsets are fixed corrections (any
/// sign) added to the implied index level — they shift both break sides
/// together, mirroring the single-price + single-offset original.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LinkParams {
    /// Index symbol (an index proxy ETF such as `SPY` works): the top row.
    pub index_symbol: String,
    /// The leveraged/inverse ETF: the second row and the break prices.
    pub etf_symbol: String,
    /// The ETF's opening price today. Old caches stored this as `etf_base`.
    #[serde(alias = "etf_base")]
    pub etf_open: f64,
    /// The index's opening price today (old alias `index_base`).
    #[serde(alias = "index_base")]
    pub index_open: f64,
    /// The ETF's leverage: it moves `multiple` times the index, in the
    /// opposite direction when `is_short`.
    pub multiple: u8,
    /// `true` for an inverse ETF (做空): it rises as the index falls.
    pub is_short: bool,
    /// Fixed correction on top of the computed index level (old alias
    /// `offset`).
    #[serde(alias = "offset")]
    pub index_offset: f64,
    /// Extra correction on top of the implied index level; like
    /// `index_offset`, it shifts both break sides together.
    pub long_short_offset: f64,
}

impl Default for LinkParams {
    fn default() -> Self {
        Self {
            index_symbol: String::new(),
            etf_symbol: String::new(),
            etf_open: 0.0,
            index_open: 0.0,
            multiple: 1,
            is_short: false,
            index_offset: 0.0,
            long_short_offset: 0.0,
        }
    }
}

impl LinkParams {
    /// Opening prices known and a usable multiple; nothing can be derived
    /// until then.
    pub fn is_valid(&self) -> bool {
        self.etf_open > 0.0 && self.index_open > 0.0 && self.multiple >= 1
    }

    /// Signed leverage: index return = ETF return ÷ factor.
    fn factor(&self) -> f64 {
        let sign = if self.is_short { -1.0 } else { 1.0 };
        f64::from(self.multiple) * sign
    }

    /// ETF return relative to its opening price. `None` without a valid
    /// link or for a non-positive price (no quote yet counts as none).
    fn etf_change(&self, etf_price: f64) -> Option<f64> {
        (self.is_valid() && etf_price > 0.0).then(|| (etf_price - self.etf_open) / self.etf_open)
    }

    /// The index level implied by the day's move alone: the ETF's return
    /// since the open, divided by the leverage, applied to the index open.
    /// `None` when the implied level is not positive — no real index gets
    /// there, so the parameters are off.
    fn index_level(&self, etf_price: f64) -> Option<f64> {
        let change = self.etf_change(etf_price)?;
        positive(self.index_open * (1.0 + change / self.factor()))
    }

    /// Index price at which an ETF quote at `etf_price` would trade on the
    /// *bid* side: implied level + index offset + long/short offset — both
    /// offsets shift both sides together, matching the pre-bid/ask single
    /// price version.
    pub fn break_bid(&self, etf_price: f64) -> Option<f64> {
        self.break_price(etf_price)
    }

    /// Same for the *ask* side (identical arithmetic; the two offsets move
    /// both sides in lockstep).
    pub fn break_ask(&self, etf_price: f64) -> Option<f64> {
        self.break_price(etf_price)
    }

    fn break_price(&self, etf_price: f64) -> Option<f64> {
        positive(self.index_level(etf_price)? + self.index_offset + self.long_short_offset)
    }
}

/// A price level must be positive to mean anything.
fn positive(value: f64) -> Option<f64> {
    (value > 0.0).then_some(value)
}

/// Per-instance persisted state (the old `etf_price` input is gone; unknown
/// fields in existing caches are ignored). The open dates bookkeep the
/// auto-filled opening prices: a daily bar only refreshes its side when it
/// is from a day the side has not been anchored for yet, so a manual edit
/// survives the day but yesterday's open never leaks into today.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Cache {
    params: LinkParams,
    index_open_date: Option<NaiveDate>,
    etf_open_date: Option<NaiveDate>,
}

pub struct ETFLinkCalcWindow {
    id: u32,
    params: LinkParams,
    /// The trading days the opening prices were last anchored for.
    index_open_date: Option<NaiveDate>,
    etf_open_date: Option<NaiveDate>,
    /// This window's id in the hub's subscriber table.
    subscriber: EntityId,
    /// Live top-of-book quotes; `None` per side while no quote has arrived
    /// (a one-sided quote is legal, e.g. at the opening auction).
    index_bid: Option<f64>,
    index_ask: Option<f64>,
    etf_bid: Option<f64>,
    etf_ask: Option<f64>,
    /// Live message pump for the current connection.
    pump: Option<Task<()>>,
}

impl AppWindow for ETFLinkCalcWindow {
    fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        window_type: WindowType,
        cache: serde_json::Value,
    ) -> Self {
        hook_should_close(window, cx);

        let cache: Cache = serde_json::from_value(cache).unwrap_or_default();

        let hub = cx.global::<AppState>().relay_hub.clone();

        // Connection status drives the message pump: connected attaches a
        // fresh broadcast receiver, any other status cancels it and drops
        // the quotes (the last values are stale by then).
        cx.subscribe(&hub, |this, hub, _: &RelayHubEvent, cx| {
            match hub.read(cx).status() {
                RelayStatus::Connected => this.refresh_pump(cx),
                _ => this.drop_quotes(cx),
            }
        })
        .detach();

        // Release the stream subscription no matter how this window dies.
        let subscriber = cx.entity_id();
        let release_hub = hub.clone();
        cx.on_release(move |_, cx| {
            release_hub.update(cx, |hub, _| {
                hub.set_symbols(subscriber, SymbolWish::default())
            });
        })
        .detach();

        let mut this = Self {
            id: window_type.instance_id().unwrap_or(0),
            params: cache.params,
            index_open_date: cache.index_open_date,
            etf_open_date: cache.etf_open_date,
            subscriber,
            index_bid: None,
            index_ask: None,
            etf_bid: None,
            etf_ask: None,
            pump: None,
        };
        this.sync_subscription(cx);
        // The stream may already be up: the window can open after connect.
        this.refresh_pump(cx);
        // The daily bars anchoring the opens may sit anywhere between the
        // last session and mid-formation: pull a snapshot once up front so
        // the breaks work from the first frame instead of waiting for the
        // stream's bar (which only arrives once the day's bar exists).
        this.fetch_opens(cx);
        this
    }

    fn open(cx: &mut App) {
        let id = super::allocate_instance_id(cx, WindowType::ETFLinkCalc(0));
        Self::open_instance(WindowType::ETFLinkCalc(id), cx);
    }

    fn window_type(&self) -> WindowType {
        WindowType::ETFLinkCalc(self.id)
    }

    fn default_window_type() -> WindowType {
        WindowType::ETFLinkCalc(0)
    }

    fn dump_cache(&mut self, _cx: &mut App) -> serde_json::Value {
        self.cache_value()
    }

    fn render_titlebar_left(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> impl IntoElement {
        super::titlebar_title(self.window_type().title())
    }

    /// The parameters entry lives on the title bar now, so the body stays
    /// the live display.
    fn render_titlebar_right(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let weak = cx.weak_entity();
        div().pr_2().child(icon_button(
            "etf-linkcalc-params",
            AppIcon::Settings,
            tr!("app.etf_linkcalc.params"),
            false,
            move |_, window, cx| {
                let Some(entity) = weak.upgrade() else {
                    return;
                };
                let params = entity.read(cx).params.clone();
                let weak = weak.clone();
                params_dialog::open_params_dialog(window, cx, params, move |params, cx| {
                    let _ = weak.update(cx, |this, cx| this.apply_params(params, cx));
                });
            },
        ))
    }

    fn render_content(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Fill the body under the title bar and center the row stack in it,
        // so the layout stays centered as the window is resized. Blank
        // break-price cells mean today's anchor pair is not in yet — the
        // semantics live in the user docs, not on screen.
        v_flex()
            .w_full()
            .flex_1()
            .justify_center()
            .p_4()
            .gap_3()
            .child(self.render_quote_row(true, cx))
            .child(self.render_quote_row(false, cx))
            .child(self.render_breaks(cx))
    }
}

impl ETFLinkCalcWindow {
    /// Store the edited parameters and follow any symbol change on the wire.
    /// The parameters are persisted right away (through the debounced
    /// settings save): quitting the app tears windows down without going
    /// through [`AppWindow::close`], so cache-until-close would lose them —
    /// the same reason dock layouts persist on every layout change.
    fn apply_params(&mut self, params: LinkParams, cx: &mut Context<Self>) {
        self.params = params;
        let cache = self.cache_value();
        super::update_window_state(cx, self.window_type(), |state| state.cache = cache);
        self.sync_subscription(cx);
        // A symbol switch invalidates which bars the opens came from: pull
        // the fresh pair up front, the stream's bars follow in time.
        self.fetch_opens(cx);
        cx.notify();
    }

    /// The parameters as a persisted cache payload.
    fn cache_value(&self) -> serde_json::Value {
        serde_json::to_value(Cache {
            params: self.params.clone(),
            index_open_date: self.index_open_date,
            etf_open_date: self.etf_open_date,
        })
        .unwrap_or(serde_json::Value::Null)
    }

    /// Publish this window's two symbols (with quotes) to the hub. Symbols
    /// are normalized so the pump matches stream messages exactly; with
    /// either unset there is nothing to subscribe.
    fn sync_subscription(&mut self, cx: &mut Context<Self>) {
        let symbols: BTreeSet<String> = [&self.params.index_symbol, &self.params.etf_symbol]
            .into_iter()
            .filter(|symbol| !symbol.is_empty())
            .map(|symbol| symbol.to_ascii_uppercase())
            .collect();
        let subscriber = self.subscriber;
        let hub = cx.global::<AppState>().relay_hub.clone();
        hub.update(cx, |hub, _| {
            hub.set_symbols(
                subscriber,
                SymbolWish {
                    symbols,
                    trades: false,
                    quotes: true,
                },
            )
        });
    }

    /// (Re)attach the broadcast-receiver pump: one receiver of the hub's
    /// stream, filtered for this window's two symbols.
    fn refresh_pump(&mut self, cx: &mut Context<Self>) {
        let hub = cx.global::<AppState>().relay_hub.clone();
        let Some(rx) = hub.read(cx).stock_messages() else {
            return;
        };
        self.pump = Some(cx.spawn(async move |this, cx| {
            let mut rx = rx;
            loop {
                match rx.recv().await {
                    Ok(first) => {
                        // Drain the queued rest: one entity update per batch
                        // keeps the main thread cheap while other panels'
                        // high-rate subscriptions fill the channel.
                        let mut batch = vec![first];
                        while let Ok(message) = rx.try_recv() {
                            batch.push(message);
                        }
                        if this
                            .update(cx, |this, cx| {
                                for message in batch {
                                    this.apply_message(message, cx);
                                }
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    // A brief data gap only momentarily stales the chips:
                    // a utility window stays quiet in the message center.
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(
                            "{}",
                            tr!("app.log.missed_stream_messages", count = skipped)
                        );
                    }
                    Err(_) => break,
                }
            }
        }));
    }

    fn apply_message(&mut self, message: StockMessage, cx: &mut Context<Self>) {
        match message {
            StockMessage::Quote(quote) => {
                let (bid, ask) = (positive(quote.bid_price), positive(quote.ask_price));
                let applied = if quote.symbol.eq_ignore_ascii_case(&self.params.index_symbol) {
                    self.index_bid = bid;
                    self.index_ask = ask;
                    true
                } else if quote.symbol.eq_ignore_ascii_case(&self.params.etf_symbol) {
                    self.etf_bid = bid;
                    self.etf_ask = ask;
                    true
                } else {
                    false
                };
                if applied {
                    cx.notify();
                }
            }
            StockMessage::DailyBar(bar) => self.apply_daily_bar(&bar, cx),
            _ => {}
        }
    }

    /// Anchor the day's opening price from a daily bar. The stream delivers
    /// the two symbols' daily bars alongside their quotes (the hub always
    /// subscribes them), so the break prices work without the opens being
    /// typed by hand. A side is only re-anchored on a day it has not been
    /// anchored for yet: within the day a manual entry from the parameter
    /// dialog wins over the bar, and a stale cached open from a previous
    /// day cannot leak forward. The updated parameters persist right away,
    /// same as [`Self::apply_params`].
    fn apply_daily_bar(&mut self, bar: &BarMessage, cx: &mut Context<Self>) {
        if self.anchor_open(&bar.symbol, bar.time.date_naive(), bar.open) {
            let cache = self.cache_value();
            super::update_window_state(cx, self.window_type(), |state| state.cache = cache);
            cx.notify();
        }
    }

    /// Date-gated open anchoring shared by the stream's daily bars and the
    /// snapshot fetch: `date` must be *today* — the current trading day, not
    /// a day the side has already been anchored for — and `open` must be
    /// positive. The today requirement is what keeps a pre-open snapshot's
    /// previous-session bar from being mistaken for today's anchor (before
    /// the open it is the only bar the snapshot carries). Within the day a
    /// manual entry from the parameter dialog wins over the bar (the gate
    /// rejects the second anchor for the same date). Returns whether
    /// anything changed. US daily bars carry their trading day in their UTC
    /// timestamp, so the UTC date is the trading date for both sources.
    fn anchor_open(&mut self, symbol: &str, date: NaiveDate, open: f64) -> bool {
        if date != chrono::Utc::now().date_naive() {
            return false;
        }
        let Some(open) = positive(open) else {
            return false;
        };
        let (is_index, is_etf) = (
            symbol.eq_ignore_ascii_case(&self.params.index_symbol),
            symbol.eq_ignore_ascii_case(&self.params.etf_symbol),
        );
        let (already_anchored, side_open) = match (is_index, is_etf) {
            (true, _) => (&mut self.index_open_date, &mut self.params.index_open),
            (_, true) => (&mut self.etf_open_date, &mut self.params.etf_open),
            _ => return false,
        };
        if *already_anchored == Some(date) {
            return false;
        }
        *side_open = open;
        *already_anchored = Some(date);
        true
    }

    /// Pull both symbols' snapshots once (window opened or symbols changed)
    /// and anchor the opens from the *current* daily bars. Before the open
    /// the snapshot carries the previous session's bar, which the date gate
    /// rejects — the break prices then stay blank until either the day's
    /// first bar shows up or the opens are typed in the parameter dialog.
    /// A failure stays in the log: the stream's bars still anchor later.
    fn fetch_opens(&mut self, cx: &mut Context<Self>) {
        let symbols: Vec<String> = [&self.params.index_symbol, &self.params.etf_symbol]
            .into_iter()
            .filter(|symbol| !symbol.is_empty())
            .map(|symbol| symbol.to_ascii_uppercase())
            .collect();
        if symbols.is_empty() {
            return;
        }
        let Ok(relay) = AppState::relay(cx) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let Ok(response) = relay
                .rpc_checked(RpcRequestPayload::Snapshot {
                    symbols: symbols.clone(),
                })
                .await
            else {
                return;
            };
            let RpcResponsePayload::Snapshot(snapshots) = response else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                let mut changed = false;
                for snapshot in &snapshots {
                    if let Some(bar) = &snapshot.daily_bar {
                        let date = chrono::DateTime::from_timestamp_millis(bar.time_ms)
                            .map(|time| time.date_naive());
                        if let Some(date) = date
                            && this.anchor_open(&snapshot.symbol, date, bar.open)
                        {
                            changed = true;
                        }
                    }
                }
                if changed {
                    let cache = this.cache_value();
                    super::update_window_state(cx, this.window_type(), |state| state.cache = cache);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Detach the pump and clear the quotes: with the stream down the last
    /// values are stale, and a trading tool must not show stale prices.
    fn drop_quotes(&mut self, cx: &mut Context<Self>) {
        self.pump = None;
        self.index_bid = None;
        self.index_ask = None;
        self.etf_bid = None;
        self.etf_ask = None;
        cx.notify();
    }

    /// Whether the anchor pair is usable *today*: every side is either a
    /// manual entry (no date — the parameter dialog asks the user to refresh
    /// those each trading day) or was anchored for today. Auto anchors from
    /// a previous session (a stale cache from Friday read on Monday) fail
    /// this, so the breaks go blank instead of measuring against the wrong
    /// day's open.
    fn opens_current(&self) -> bool {
        let today = chrono::Utc::now().date_naive();
        let fresh = |date: Option<NaiveDate>| date.is_none() || date == Some(today);
        fresh(self.index_open_date) && fresh(self.etf_open_date)
    }

    /// The two break prices an ETF quote level implies: (bid side, ask side).
    fn break_pair(&self, etf_price: Option<f64>) -> (Option<f64>, Option<f64>) {
        if !self.opens_current() {
            return (None, None);
        }
        match etf_price {
            Some(price) => (self.params.break_bid(price), self.params.break_ask(price)),
            None => (None, None),
        }
    }

    /// One symbol row: the symbol pill followed by its live bid and ask.
    fn render_quote_row(&self, index: bool, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (symbol, bid, ask) = if index {
            (
                self.params.index_symbol.as_str(),
                self.index_bid,
                self.index_ask,
            )
        } else {
            (self.params.etf_symbol.as_str(), self.etf_bid, self.etf_ask)
        };
        h_flex()
            .items_center()
            .gap_2()
            .child(pill(
                if symbol.is_empty() { "—" } else { symbol },
                theme.secondary,
                theme.secondary_foreground,
            ))
            // Bid/ask take the user's quote colors (red/green by default).
            .child(price_pill(bid, theme.chart_bullish, cx))
            .child(price_pill(ask, theme.chart_bearish, cx))
    }

    /// The break-price row: the leverage chip and the direction chip on the
    /// left — together exactly as wide as the symbol chips above — then for
    /// each live ETF quote level its bid-side and ask-side index price, so
    /// the two pairs align under the ETF row's bid and ask chips. Each pair
    /// sits under the quote level it was derived from (bid pair under the
    /// bid chip, ask pair under the ask chip): on a short ETF the higher
    /// quote maps to the lower index, so the ladder reads descending —
    /// column alignment wins over ladder aesthetics.
    fn render_breaks(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (bid_level_bid, bid_level_ask) = self.break_pair(self.etf_bid);
        let (ask_level_bid, ask_level_ask) = self.break_pair(self.etf_ask);
        let direction = if self.params.is_short {
            tr!("app.etf_linkcalc.short")
        } else {
            tr!("app.etf_linkcalc.long")
        };

        h_flex()
            .items_center()
            .gap_2()
            .child(pill(
                format!("×{}", self.params.multiple),
                theme.secondary,
                theme.secondary_foreground,
            ))
            .child(pill(direction, theme.secondary, theme.secondary_foreground))
            .child(price_pill(bid_level_bid, theme.chart_bullish, cx))
            .child(price_pill(bid_level_ask, theme.chart_bearish, cx))
            .child(price_pill(ask_level_bid, theme.chart_bullish, cx))
            .child(price_pill(ask_level_ask, theme.chart_bearish, cx))
    }
}

impl Render for ETFLinkCalcWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_window(window, cx)
    }
}

/// A flat chip (native rectangle, no rounding) that scales with the window:
/// every chip in a row takes an equal share of the available width, text
/// centered.
fn pill(text: impl Into<SharedString>, bg: Hsla, fg: Hsla) -> Div {
    div()
        .flex()
        .flex_1()
        .justify_center()
        .py_1()
        .bg(bg)
        .text_color(fg)
        .text_sm()
        .child(text.into())
}

/// A price chip: filled with `color` when a value is present — the theme
/// background as its text keeps it readable on both quote colors — and a
/// muted placeholder while no quote has arrived.
fn price_pill(value: Option<f64>, color: Hsla, cx: &App) -> Div {
    let theme = cx.theme();
    match value {
        Some(value) => pill(format_price(value), color, theme.background),
        None => pill("—", theme.muted, theme.muted_foreground),
    }
}

/// Two decimals cover index levels and penny-increment ETFs alike.
fn format_price(value: f64) -> String {
    format!("{value:.2}")
}

/// A number field seeded with `value` (empty when zero, so the placeholder
/// shows; non-zero seeds as-is, which is how negative offsets come back).
/// `step` makes the spin buttons work; `min` clamps when the field has a
/// floor (prices are non-negative, an offset is not).
fn number_input(
    window: &mut Window,
    cx: &mut Context<InputState>,
    value: f64,
    placeholder: String,
    min: Option<f64>,
    step: f64,
) -> InputState {
    let state = InputState::new(window, cx)
        .placeholder(placeholder)
        .step(step);
    let state = match min {
        Some(min) => state.min(min),
        None => state,
    };
    if value != 0.0 {
        // Shortest round-trip form, not a padded decimal: `12.34`, not
        // `12.3400`.
        state.default_value(value.to_string())
    } else {
        state
    }
}

fn parse_f64(s: &str) -> f64 {
    // `0.0` is the "no input" sentinel; reject `NaN`/`inf` spellings too —
    // a non-finite value in any field breaks the cache serialization and
    // would wipe the whole saved state on close.
    s.trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(multiple: u8, is_short: bool) -> LinkParams {
        LinkParams {
            etf_open: 12.0,
            index_open: 4500.0,
            multiple,
            is_short,
            ..Default::default()
        }
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn leverage_divides_the_etf_move() {
        // A 3× long ETF up 3% means the index is up 1%.
        let p = params(3, false);
        assert!(close(p.index_level(12.36).unwrap(), 4545.0));
    }

    #[test]
    fn short_inverts_the_direction() {
        // A 2× short ETF up 2% means the index is down 1%.
        let p = params(2, true);
        assert!(close(p.index_level(12.24).unwrap(), 4455.0));
    }

    #[test]
    fn offsets_shift_both_sides_in_lockstep() {
        // 做多:两侧同式 —— level + index offset + long/short offset。
        let p = LinkParams {
            index_offset: 5.0,
            long_short_offset: 10.0,
            ..params(3, false)
        };
        assert!(close(p.break_bid(12.36).unwrap(), 4560.0));
        assert!(close(p.break_ask(12.36).unwrap(), 4560.0));
    }

    #[test]
    fn short_offsets_shift_both_sides_in_lockstep() {
        // 做空:两侧同式 —— level + index offset + long/short offset。
        let p = LinkParams {
            index_offset: 5.0,
            long_short_offset: 10.0,
            ..params(2, true)
        };
        assert!(close(p.break_bid(12.24).unwrap(), 4470.0));
        assert!(close(p.break_ask(12.24).unwrap(), 4470.0));
    }

    #[test]
    fn negative_offsets_derive_nothing_past_zero() {
        let p = LinkParams {
            index_offset: -5000.0,
            ..params(3, false)
        };
        assert!(p.break_bid(12.36).is_none());
        assert!(p.break_ask(12.36).is_none());
        // Negative offsets may push the whole level under zero: both sides
        // derive nothing then.
        let p = LinkParams {
            index_offset: 5.0 - 4545.0, // level lands at 5
            long_short_offset: -10.0,
            ..params(3, false)
        };
        assert!(p.break_bid(12.36).is_none());
        assert!(p.break_ask(12.36).is_none());
    }

    #[test]
    fn invalid_link_derives_nothing() {
        let mut p = params(1, false);
        p.etf_open = 0.0;
        assert!(!p.is_valid());
        assert!(p.break_bid(12.0).is_none());
        assert!(p.break_ask(12.0).is_none());
        // Zero is "no quote yet", not a price.
        assert!(p.break_bid(0.0).is_none());
    }

    #[test]
    fn cache_tolerates_missing_fields_and_old_names() {
        // An old cache (base prices, one offset, an ETF price input that no
        // longer exists) loads through the aliases.
        let cache: Cache = serde_json::from_value(serde_json::json!({
            "params": {
                "etf_base": 8.55,
                "index_base": 755.5,
                "multiple": 1,
                "is_short": true,
                "offset": 1.2
            },
            "etf_price": 8.56
        }))
        .unwrap();
        assert_eq!(cache.params.etf_open, 8.55);
        assert_eq!(cache.params.index_open, 755.5);
        assert_eq!(cache.params.index_offset, 1.2);
        assert_eq!(cache.params.multiple, 1);
        assert!(cache.params.is_short);
        assert_eq!(cache.params.long_short_offset, 0.0);
        assert!(cache.params.index_symbol.is_empty());

        let blank: Cache = serde_json::from_value(serde_json::Value::Null).unwrap_or_default();
        assert_eq!(blank.params, LinkParams::default());
    }
}
