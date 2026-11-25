//! The data layer: the panels' only data outlet. Everything that talks to
//! the server — typed RPC queries ([`api`]), live subscriptions
//! ([`stream`]) — lives here, plus the shared display formatters
//! ([`format`]).
//!
//! Rules (project conventions, mirrored in the zensidus-client skill):
//! - panels and UI components never touch `protocol::`, `ServerClient` or
//!   `AppState::hub` directly; they go through this layer's typed API;
//! - this layer holds no UI types (no panel/delegate state); the one
//!   exception is [`DataRequest`], which is generic over the caller's state
//!   entity and only orchestrates — it names no concrete panel type.
//!
//! Errors: queries return `anyhow::Result` (there is no `DataError` — the
//! offline case never surfaces as an error, it is a silent skip, see
//! [`DataRequest::run`]); failures are reported to the message center by
//! the layer itself, under the caller's `error_title`.

pub mod api;
pub mod format;
pub mod stream;

/// The app's live server client, for the rare fetch that orchestrates
/// itself by hand (a panel whose sync half runs inside its own update —
/// an entity cannot `update` itself there — and only needs to hand the
/// client to a typed [`api`] query). Prefer [`DataRequest`].
pub fn server_client(cx: &App) -> anyhow::Result<crate::utils::server::ServerClient> {
    crate::utils::state::AppState::server(cx)
}

use std::future::Future;

use gpui_kit::{App, Context, SharedString, WeakEntity};

use crate::utils::message;
use crate::utils::server::ServerClient;
use crate::utils::state::AppState;

/// The panels' one fetch orchestration: the offline guard, the
/// fetch-generation handshake against out-of-order responses and the error
/// report to the message center — the three things every pulling panel used
/// to hand-write (table `load_rows`, the kline history load and the market
/// news backfill each had their own copy).
///
/// A request is pinned to a generation so a slow earlier response can never
/// overwrite a newer one:
///
/// - `begin` marks the state as loading and returns the generation this
///   request is pinned to; `None` aborts silently (nothing to load).
/// - `is_current` tells whether a generation is still the state's newest
///   fetch; a superseded outcome is discarded without touching the state
///   (the newer fetch owns the loading flag now).
/// - `on_ok` / `on_err` write the outcome back — including clearing the
///   loading flag; an `Err` is additionally reported to the message center
///   under the request's `error_title` (after `on_err`, so state first,
///   message last, as everywhere else).
///
/// Offline (no server connection) the request never starts: silent skip
/// with a debug log — startup auto-load paths wait for the hub's
/// `Connected` event (`panels::reload_on_connect`) instead of stacking one
/// error per panel into the message center.
///
/// # 重入约束
///
/// `run` 的 begin 阶段会同步 `update` 状态实体。若 `run` 在**该实体自身**
/// 的 update 周期内被调(观察者回调、`refresh` 钩子里的
/// `entity.update(|this| this.fetch())` 等),"update 自己时再 update
/// 自己"直接 panic(实测)。约束:目标实体 ≠ 当前 update 中的实体——
/// 表面板的目标是 `TableState`(面板 ≠ 表,安全),`DataEntity = Self`
/// 的面板(如 MarketScreener)必须走手写编排:同步半段直接改字段
/// (`&mut self` 在手),异步回写经 `cx.spawn`(见
/// `panels/screener/market.rs::fetch`)。
pub struct DataRequest<T> {
    state: WeakEntity<T>,
    error_title: SharedString,
}

impl<T: 'static> DataRequest<T> {
    /// A request against `state`; failures report under `error_title`.
    pub fn new(state: WeakEntity<T>, error_title: SharedString) -> Self {
        Self { state, error_title }
    }

    /// Run the `query` (a typed [`crate::data::api`] function against the
    /// app's server client) and hand the outcome to the state entity. The
    /// callbacks receive the request's generation so states that keep their
    /// own handshake (the kline history) can re-check it.
    pub fn run<V, Fut, B, C, Q, OkFn, ErrFn>(
        self,
        cx: &mut App,
        begin: B,
        is_current: C,
        query: Q,
        on_ok: OkFn,
        on_err: ErrFn,
    ) where
        B: FnOnce(&mut T, &mut Context<T>) -> Option<u64>,
        C: Fn(&mut T, u64) -> bool + 'static,
        Q: FnOnce(ServerClient) -> Fut + 'static,
        Fut: Future<Output = anyhow::Result<V>> + 'static,
        OkFn: FnOnce(&mut T, &mut Context<T>, u64, V) + 'static,
        ErrFn: FnOnce(&mut T, &mut Context<T>, u64, &anyhow::Error) + 'static,
    {
        let Some(state) = self.state.upgrade() else {
            return;
        };
        let Ok(server) = AppState::server(cx) else {
            tracing::debug!("{}: server is not connected", self.error_title);
            return;
        };
        let Some(generation) = state.update(cx, |state, cx| begin(state, cx)) else {
            return;
        };
        let error_title = self.error_title;
        cx.spawn(async move |cx| {
            let result = query(server).await;
            state.update(cx, |state, cx| {
                if !is_current(state, generation) {
                    // A newer fetch (or a state reset) started while this
                    // request was in flight; its outcome is obsolete.
                    return;
                }
                match result {
                    Ok(value) => on_ok(state, cx, generation, value),
                    Err(err) => {
                        on_err(state, cx, generation, &err);
                        message::error(error_title, err, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}
