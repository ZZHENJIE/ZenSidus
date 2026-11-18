use std::collections::HashSet;

use chrono::TimeZone;
use gpui_kit::{
    AsyncApp, Context, IntoElement, ParentElement, Render, Styled, WeakEntity, Window,
    base::{h_flex, v_flex},
    component::{ActiveTheme as _, Icon, WindowExt, notification::Notification, spinner::Spinner},
    div,
};
use relay_protocol::EconomicItem;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tracing::error;

use crate::{
    tr,
    utils::relay::RelayClient,
    utils::state::{AppState, audio::AppAudio},
    windows::{AppWindow, WindowType, hook_should_close},
};

/// A small floating clock window with an economic-event alert: the calendar
/// (the current week's export) is fetched once, and every tick compares the
/// calibrated time against it — events inside the configured lead time show
/// a badge in the content area and (once per event) play the hint sound.
/// Singleton like the other windows — reopening it activates the existing
/// one.
pub struct SmallClockWindow {
    text: String,
    loading: bool,
    /// The calibrated wall-clock time of the last tick (relay server time
    /// once, then incremented per second).
    now: i64,
    /// The whole fetched week of economic events.
    calendar: Vec<EconomicItem>,
    /// Events that already played their alert sound, keyed by
    /// (timestamp, title) — the export has no unique id.
    alerted: HashSet<(i64, String)>,
    /// Events inside the alert window right now: (count, max impact).
    upcoming: Option<(usize, u8)>,
    /// Kept alive on the struct on purpose: a dropped `Task` cancels its
    /// future, so dropping these with the window is exactly the shutdown.
    tokio_task: Option<tokio::task::JoinHandle<()>>,
    gui_task: Option<gpui_kit::Task<()>>,
}

fn format_timestamp(timestamp: i64) -> String {
    let local_time = match chrono::Local.timestamp_opt(timestamp, 0) {
        chrono::LocalResult::Single(time) => time,
        _ => {
            error!(
                "{}",
                tr!("app.log.invalid_timestamp", timestamp = timestamp.to_string())
            );
            return String::default();
        }
    };
    local_time.format("%H:%M:%S").to_string()
}

enum Event {
    /// One calibrated second.
    Tick(i64),
    /// The full calendar fetch finished.
    Loaded,
    Calendar(Vec<EconomicItem>),
}

impl SmallClockWindow {
    async fn tokio_task(tx: UnboundedSender<Event>, relay: Option<RelayClient>) {
        // Create the timer
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // Network time from the relay's Welcome/heartbeat clock estimate
        // (server clock, ms → s); without a sample, fall back to the local
        // clock.
        let mut timestamp: i64 = relay
            .as_ref()
            .and_then(|relay| relay.clock().get())
            .map(|(offset_ms, rtt_ms)| {
                (chrono::Utc::now().timestamp_millis() + offset_ms + rtt_ms / 2) / 1000
            })
            .unwrap_or_else(|| {
                error!("{}", tr!("app.log.network_time_failed", err = "no relay clock sample".to_string()));
                chrono::Local::now().timestamp()
            });
        // Send the loaded event
        if tx.send(Event::Loaded).is_err() {
            return;
        }

        loop {
            if tx.send(Event::Tick(timestamp)).is_err() {
                break;
            }
            interval.tick().await;
            timestamp += 1;
        }
    }

    /// One calendar fetch for this window's lifetime: the current week's
    /// export covers every event the alert window can ever fire on (the
    /// server snaps the date to its Mon–Fri window).
    async fn calendar_task(tx: UnboundedSender<Event>, relay: RelayClient) {
        let date = chrono::Utc::now().date_naive().to_string();
        match relay
            .rpc_checked(relay_protocol::RpcRequestPayload::EconomicCalendar { date })
            .await
        {
            Ok(relay_protocol::RpcResponsePayload::EconomicCalendar(rows)) => {
                let _ = tx.send(Event::Calendar(rows));
            }
            Ok(other) => {
                error!(
                    "{}",
                    tr!("app.log.calendar_fetch_failed", err = format!("unexpected rpc response: {other:?}"))
                );
            }
            Err(err) => {
                error!(
                    "{}",
                    tr!("app.log.calendar_fetch_failed", err = format!("{err:#}"))
                );
            }
        }
    }

    async fn gui_task(
        this: WeakEntity<SmallClockWindow>,
        cx: &mut AsyncApp,
        mut rx: UnboundedReceiver<Event>,
    ) {
        while let Some(event) = rx.recv().await {
            let _ = this.update(cx, |view, cx| {
                match event {
                    Event::Loaded => {
                        view.loading = false;
                    }
                    Event::Tick(now) => {
                        view.now = now;
                        view.text = format_timestamp(now);
                    }
                    Event::Calendar(items) => {
                        view.calendar = items;
                    }
                }
                view.evaluate_alerts(cx);
                cx.notify();
            });
        }
    }

    /// Recompute which events are inside the alert window
    /// (`now < event <= now + lead` and at least the configured impact) and
    /// play the hint sound once per newly arriving event. Runs on every
    /// tick, so setting changes apply within a second without a refetch.
    /// (Tick time is in seconds; the calendar rows are in epoch ms.)
    fn evaluate_alerts(&mut self, cx: &Context<Self>) {
        let config = &cx.global::<AppState>().settings.small_clock;
        let lead_ms = config.alert_minutes as i64 * 60 * 1000;
        let min_impact = config.min_impact;
        let now_ms = self.now * 1000;

        let upcoming: Vec<&EconomicItem> = self
            .calendar
            .iter()
            .filter(|item| {
                now_ms < item.time_ms && now_ms + lead_ms > item.time_ms && item.impact >= min_impact
            })
            .collect();

        let count = upcoming.len();
        self.upcoming = if count > 0 {
            Some((
                count,
                upcoming.iter().map(|item| item.impact).max().unwrap_or(0),
            ))
        } else {
            None
        };

        if config.sound {
            for item in upcoming {
                let key = (item.time_ms, item.event.clone());
                if self.alerted.insert(key) {
                    let _ = cx.global::<AppState>().play_audio(AppAudio::Hint);
                }
            }
        }
    }
}

impl AppWindow for SmallClockWindow {
    fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        _window_type: WindowType,
        _cache: serde_json::Value,
    ) -> Self {
        hook_should_close(window, cx);

        let mut this = Self {
            text: String::new(),
            loading: false,
            now: chrono::Local::now().timestamp(),
            calendar: Vec::new(),
            alerted: HashSet::new(),
            upcoming: None,
            tokio_task: None,
            gui_task: None,
        };

        if let Some(runtime) = AppState::tokio_runtime(cx) {
            // Runtime is available
            let (tx, rx) = unbounded_channel::<Event>();
            let relay = AppState::relay(cx).ok();
            // Request and timer Task
            this.loading = true;
            let tx_time = tx.clone();
            this.tokio_task = Some(runtime.spawn(async move {
                Self::tokio_task(tx_time, relay).await;
            }));
            // One calendar fetch alongside the timer (skipped silently while
            // the relay is down — the badge area just stays empty).
            if let Ok(relay) = AppState::relay(cx) {
                runtime.spawn(async move {
                    Self::calendar_task(tx, relay).await;
                });
            } else {
                this.loading = false;
                let _ = tx.send(Event::Loaded);
            }

            this.gui_task = Some(cx.spawn(async move |this, cx| {
                Self::gui_task(this, cx, rx).await;
            }))
        } else {
            window.push_notification(
                Notification::new().title(tr!("app.tokio_runtime_error")),
                cx,
            );
        }
        this
    }

    fn window_type(&self) -> WindowType {
        WindowType::SmallClock
    }

    fn default_window_type() -> WindowType {
        WindowType::SmallClock
    }

    fn render_titlebar_left(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> impl IntoElement {
        if self.loading {
            div().child(Spinner::new())
        } else {
            div().child(super::titlebar_title(self.text.clone()))
        }
    }

    fn render_content(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match self.upcoming {
            Some((count, max_impact)) => h_flex()
                .items_center()
                .justify_center()
                .gap_1()
                .px_2()
                .text_color(cx.theme().warning)
                .child(Icon::new(crate::utils::AppIcon::Bell))
                .child(tr!(
                    "app.small_clock.alert",
                    count = count,
                    level = max_impact
                )),
            None => v_flex(),
        }
    }
}

impl Render for SmallClockWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_window(window, cx)
    }
}
