//! The `AppState` global: the loaded settings, the shared server hub and the
//! sound channel. Registered once at startup; every settings mutation goes
//! through `settings::update_settings`, whose `global_mut` notifies all
//! `observe_global::<AppState>` observers.

pub mod audio;
pub mod hub;
pub mod stream_dispatch;

use gpui_kit::{App, AppContext as _, Entity, Global};

pub use hub::{Hub, HubEvent, ServerStatus, SymbolWish};
pub use stream_dispatch::{SharedSpec, StreamSpec, spawn_filter_stage};

use crate::tr;
use crate::utils::message;

pub struct AppState {
    pub settings: crate::utils::settings::Settings,
    /// The debounced settings save ([`crate::utils::settings::update_settings`]
    /// sets this instead of hitting the disk per mutation).
    pub(crate) settings_dirty: bool,
    /// The pending debounce timer; `None` while no save is scheduled.
    pub(crate) settings_flush_task: Option<gpui_kit::Task<()>>,
    audio_sender: std::sync::mpsc::Sender<audio::AppAudio>,
    /// The app-wide server connection hub; created once and never replaced.
    pub hub: Entity<Hub>,
    pub tokio_runtime: Option<tokio::runtime::Runtime>,
}

impl Global for AppState {}

impl AppState {
    pub fn new(settings: crate::utils::settings::Settings, cx: &mut App) -> Self {
        // The runtime is built first so the hub can share its handle (there
        // is exactly one tokio runtime in the app).
        let tokio_runtime = match tokio::runtime::Runtime::new() {
            Ok(rt) => Some(rt),
            Err(err) => {
                message::error(tr!("app.tokio_runtime_error"), err, cx);
                None
            }
        };
        let runtime_handle = tokio_runtime.as_ref().map(|rt| rt.handle().clone());
        Self {
            settings,
            settings_dirty: false,
            settings_flush_task: None,
            audio_sender: audio::audio_thread(),
            hub: cx.new(|_| Hub::new(runtime_handle)),
            tokio_runtime,
        }
    }

    /// Queue a sound on the audio thread; fails only if that thread is gone.
    pub fn play_audio(&self, audio: audio::AppAudio) -> anyhow::Result<()> {
        self.audio_sender
            .send(audio)
            .map_err(|_| anyhow::anyhow!("audio channel disconnected"))
    }

    /// The app-wide server client (RPC + push outlet); clone freely.
    pub fn server(cx: &App) -> anyhow::Result<crate::utils::server::ServerClient> {
        cx.global::<AppState>()
            .hub
            .read(cx)
            .client()
            .ok_or_else(|| anyhow::anyhow!("server is not connected"))
    }

    pub fn tokio_runtime(cx: &App) -> Option<&tokio::runtime::Runtime> {
        cx.global::<AppState>().tokio_runtime.as_ref()
    }
}

impl Drop for AppState {
    fn drop(&mut self) {
        // Shut the runtime down without blocking whatever thread is dropping
        // the global state (a plain `Runtime` drop waits on every task).
        if let Some(runtime) = self.tokio_runtime.take() {
            runtime.shutdown_background();
        }
    }
}
