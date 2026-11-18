//! The `AppState` global: the loaded settings, the shared relay hub and the
//! sound channel. Registered once at startup; every settings mutation goes
//! through `settings::update_settings`, whose `global_mut` notifies all
//! `observe_global::<AppState>` observers.

pub mod audio;
pub mod relay_hub;
pub mod stream_dispatch;

use gpui_kit::{App, AppContext as _, Entity, Global};

pub use relay_hub::{RelayHub, RelayHubEvent, RelayStatus, SymbolWish};
pub use stream_dispatch::{SharedSpec, StreamSpec, spawn_filter_stage};

use crate::tr;
use crate::utils::{keyring_store, message};

pub struct AppState {
    pub settings: crate::utils::settings::Settings,
    /// The debounced settings save ([`crate::utils::settings::update_settings`]
    /// sets this instead of hitting the disk per mutation).
    pub(crate) settings_dirty: bool,
    /// The pending debounce timer; `None` while no save is scheduled.
    pub(crate) settings_flush_task: Option<gpui_kit::Task<()>>,
    /// The secrets as last mirrored into the OS credential store, so the
    /// flush only touches entries whose value actually changed.
    pub(crate) synced_secrets: crate::utils::keyring_store::Secrets,
    audio_sender: std::sync::mpsc::Sender<audio::AppAudio>,
    /// The app-wide relay connection hub; created once and never replaced.
    pub relay_hub: Entity<RelayHub>,
    pub tokio_runtime: Option<tokio::runtime::Runtime>,
}

impl Global for AppState {}

impl AppState {
    pub fn new(settings: crate::utils::settings::Settings, cx: &mut App) -> Self {
        let synced_secrets = keyring_store::Secrets::from_settings(&settings);
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
            synced_secrets,
            audio_sender: audio::audio_thread(),
            relay_hub: cx.new(|_| RelayHub::new(runtime_handle)),
            tokio_runtime,
        }
    }

    /// Queue a sound on the audio thread; fails only if that thread is gone.
    pub fn play_audio(&self, audio: audio::AppAudio) -> anyhow::Result<()> {
        self.audio_sender
            .send(audio)
            .map_err(|_| anyhow::anyhow!("audio channel disconnected"))
    }

    /// The app-wide relay client (RPC + push outlet); clone freely.
    pub fn relay(cx: &App) -> anyhow::Result<crate::utils::relay::RelayClient> {
        cx.global::<AppState>()
            .relay_hub
            .read(cx)
            .client()
            .ok_or_else(|| anyhow::anyhow!("relay is not connected"))
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
