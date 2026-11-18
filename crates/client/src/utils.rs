//! Application plumbing shared by every window and panel: persisted
//! settings, the `AppState` global, the message center, theming, embedded
//! assets, icons, logging and the action types.

pub mod actions;
pub mod assets;
pub mod exchanges;

pub mod icons;
pub mod keyring_store;
pub mod logging;
pub mod message;
pub mod settings;
pub mod relay;
pub mod state;
pub mod theme;

pub use assets::Assets;
pub use icons::AppIcon;
pub use message::{error, info, success, warning};
pub use settings::Settings;
pub use state::AppState;
pub use theme::Theme;

/// `t!` as a [`gpui_kit::SharedString`], the type every gpui-kit text slot
/// takes. Accepts the same arguments as `rust_i18n::t!`.
#[macro_export]
macro_rules! tr {
    ($($arg:tt)*) => {
        ::gpui_kit::SharedString::from(::rust_i18n::t!($($arg)*).to_string())
    };
}

/// The per-user application data directory (`~/.zensidus`): the settings
/// file and the rotated logs live here.
pub fn app_dir() -> anyhow::Result<std::path::PathBuf> {
    let user_dir = directories::UserDirs::new()
        .ok_or_else(|| anyhow::anyhow!("failed to get user directories"))?;

    Ok(user_dir.home_dir().join(".zensidus"))
}
