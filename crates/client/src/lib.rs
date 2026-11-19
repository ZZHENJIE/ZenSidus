//! ZenSidus desktop GUI. `main.rs` and the examples are thin shells over
//! [`bootstrap`] and [`init_app`]; everything else lives in the modules
//! below.

rust_i18n::i18n!("locales", fallback = "en-us");

pub mod panels;
pub mod utils;
pub mod widgets;
pub mod windows;

use gpui_kit::App;

use utils::{AppState, Settings};

/// Process-level setup that must happen before the GPUI application starts:
/// the data directory, file logging, the persisted settings (defaults when
/// the file is unreadable) and the UI locale. The returned guard flushes the
/// log file; keep it alive for the whole run.
pub fn bootstrap() -> anyhow::Result<(Settings, tracing_appender::non_blocking::WorkerGuard)> {
    let app_dir = utils::app_dir()?;
    let guard = utils::logging::init(&app_dir.join("logs"), true)?;
    let mut settings = Settings::load().unwrap_or_else(|err| {
        tracing::error!(
            "{}",
            crate::tr!("app.log.load_settings_failed", err = format!("{err:#}"))
        );
        Settings::default()
    });
    // An unknown locale code (hand-edited file, older build) would make
    // every `t!` render its bare key: fall back to the default language.
    if !utils::settings::LANGUAGES
        .iter()
        .any(|(code, _)| *code == settings.language)
    {
        tracing::warn!(
            "{}",
            crate::tr!(
                "app.log.unknown_language",
                language = format!("{:?}", settings.language),
                fallback = format!("{:?}", utils::settings::LANGUAGES[0].0)
            )
        );
        settings.language = utils::settings::LANGUAGES[0].0.into();
    }
    rust_i18n::set_locale(&settings.language);
    // `Translate::default` cannot know the UI language (see its doc), so
    // bootstrap fills the pair when it is still empty.
    if settings.translate.from_lang.is_empty() || settings.translate.to_lang.is_empty() {
        settings.translate = utils::settings::Translate::for_language(&settings.language);
    }
    Ok((settings, guard))
}

/// In-app setup shared by every entry point: gpui-kit, the theme, the message center
/// open only after this returns, since opening one persists into `AppState`.
pub fn init_app(settings: Settings, cx: &mut App) {
    gpui_kit::init(cx);
    settings.theme.load(cx);
    utils::message::MessageLog::init(cx);

    let state = AppState::new(settings, cx);
    cx.set_global(state);
}
