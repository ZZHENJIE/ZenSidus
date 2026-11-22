//! Persisted application settings (`~/.zensidus/settings.json`).
//!
//! Every struct here carries `#[serde(default)]`: a settings file written by
//! an older build that lacks a field still loads, with that field at its
//! default. Without it one missing field fails the whole parse and silently
//! resets every setting (theme, language, watchlists) to defaults.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::utils::server::ServerConfig;
use gpui_kit::{App, SharedString};

/// How long `update_settings` waits before writing the file, so a burst of
/// mutations (typing in a text field, dragging a color picker, a dock
/// re-layout) lands as one disk write instead of one per event.
const SAVE_DEBOUNCE: Duration = Duration::from_millis(500);

/// Mutate the persisted settings through the `AppState` global. `global_mut`
/// also notifies every `observe_global::<AppState>` window, so open windows
/// refresh on each change.
///
/// The file write is debounced ([`SAVE_DEBOUNCE`]): this only marks the state
/// dirty and schedules the flush. [`flush_settings`] runs it synchronously —
/// call it before quitting; a crash may lose the last debounce window of
/// changes, which is the trade for not hitting the disk per keystroke.
pub fn update_settings(cx: &mut App, apply: impl FnOnce(&mut Settings)) {
    let needs_flush = {
        let state = cx.global_mut::<crate::utils::AppState>();
        apply(&mut state.settings);
        state.settings_dirty = true;
        state.settings_flush_task.is_none()
    };
    if needs_flush {
        let task = cx.spawn(async move |cx| {
            cx.background_executor().timer(SAVE_DEBOUNCE).await;
            cx.update(flush_settings);
        });
        cx.global_mut::<crate::utils::AppState>()
            .settings_flush_task = Some(task);
    }
}

/// Write the settings file now if a debounced save is pending. Runs on the
/// UI thread; call it on the quit paths so deferred saves are not lost.
pub fn flush_settings(cx: &mut App) {
    let save_result = {
        let state = cx.global_mut::<crate::utils::AppState>();
        // Detach instead of dropping: dropping a `Task` cancels it, and this
        // may run from inside that very task.
        if let Some(task) = state.settings_flush_task.take() {
            task.detach();
        }
        if !state.settings_dirty {
            return;
        }
        state.settings_dirty = false;
        state.settings.save()
    };
    if let Err(err) = save_result {
        tracing::error!(
            "{}",
            crate::tr!("app.log.settings_save_failed", err = format!("{err:#}"))
        );
        crate::utils::message::error(crate::tr!("app.notify.save_failed"), err, cx);
    }
}

/// UI languages the app ships, as `(rust-i18n locale, display name)`. The
/// locale must match a file stem under `locales/` exactly: rust-i18n does no
/// case folding, and `set_locale` with an unknown code makes every `t!`
/// return its bare key. The first entry is the default.
pub const LANGUAGES: [(&str, &str); 2] = [("en-us", "English"), ("zh-cn", "中文")];

/// Display name of a UI language code; unknown codes show as themselves.
pub fn language_label(code: &str) -> SharedString {
    LANGUAGES
        .iter()
        .find(|(value, _)| *value == code)
        .map(|(_, label)| SharedString::from(*label))
        .unwrap_or_else(|| SharedString::from(code.to_string()))
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct Settings {
    /// rust-i18n locale; one of [`LANGUAGES`].
    pub language: String,
    pub theme: super::Theme,
    pub server: Server,
    /// Saved screener presets. The section key stays `finviz` so older
    /// settings files keep loading; the export credential itself moved to
    /// the server.
    pub finviz: Finviz,
    /// Market Screener panel presets, keyed by title.
    pub market_screener: MarketScreener,
    pub translate: Translate,
    /// Small clock window: economic event alerting.
    pub small_clock: SmallClock,
    /// Symbol groups of the Watchlist panels, keyed by group name.
    pub watchlists: BTreeMap<String, Vec<String>>,
    /// Geometry and layout cache of every window ever opened.
    pub window_state: Vec<crate::windows::WindowState>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: LANGUAGES[0].0.into(),
            theme: super::Theme::default(),
            server: Server::default(),
            finviz: Finviz::default(),
            market_screener: MarketScreener::default(),
            translate: Translate::default(),
            small_clock: SmallClock::default(),
            watchlists: BTreeMap::new(),
            window_state: crate::windows::window_state_default(),
        }
    }
}

/// Economic-event alerting of the small clock window. The calendar is
/// fetched once per window (the current week's export); these knobs decide
/// which of its events count as "about to happen".
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct SmallClock {
    /// Alert for events starting within this many minutes.
    pub alert_minutes: u32,
    /// Play the hint sound when a new upcoming event is detected.
    pub sound: bool,
    /// Minimum economic impact level (1-3) to alert on.
    pub min_impact: u8,
}

impl Default for SmallClock {
    fn default() -> Self {
        Self {
            alert_minutes: 5,
            sound: true,
            min_impact: 2,
        }
    }
}

/// Preferred source/target languages of the translate dialog, stored as
/// rust-i18n locale codes (`"en-us"`/`"zh-cn"`). The derived default is
/// empty on purpose: the rust-i18n locale is still its built-in `"en"`
/// while `Settings::default`/serde run (bootstrap calls `set_locale` only
/// after loading), so the direction cannot be derived there — `bootstrap`
/// fills the empty fields from the UI language via
/// [`Translate::for_language`].
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Translate {
    pub from_lang: String,
    pub to_lang: String,
}

impl Translate {
    /// The pair for `locale`: translate from the UI language into the other
    /// supported one.
    pub fn for_language(locale: &str) -> Self {
        if locale.to_ascii_lowercase().starts_with("zh") {
            Self {
                from_lang: "zh-cn".into(),
                to_lang: "en-us".into(),
            }
        } else {
            Self {
                from_lang: "en-us".into(),
                to_lang: "zh-cn".into(),
            }
        }
    }
}

/// server 连接配置:应用唯一的对外数据出口。**token 不落盘**——它在每次
/// 点击连接时的对话框里输入,只活在本会话的连接里;这里只持久化地址与
/// TLS 选项。
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Server {
    /// 服务器地址(`host:port`;TLS 由服务器 Nginx 终结)。
    pub addr: String,
    /// true = TLS 连接。
    pub tls: bool,
    /// 固定校验的证书 PEM 路径(CF Origin 证书不在公共信任链);空 = 标准
    /// 公共根校验。
    pub tls_cert: String,
}

impl Server {
    /// 完整可用的连接配置,或 `None`(地址为空)。`code` 是连接对话框里
    /// 输入的当前 TOTP 动态码,不属于持久化状态。
    pub fn config(&self, code: &str) -> Option<ServerConfig> {
        let addr = self.addr.trim();
        let code = code.trim();
        (!addr.is_empty() && !code.is_empty()).then(|| ServerConfig {
            addr: addr.to_string(),
            code: code.to_string(),
            tls: self.tls,
            tls_cert: (!self.tls_cert.trim().is_empty())
                .then(|| PathBuf::from(self.tls_cert.trim())),
        })
    }
}

/// Saved screener presets, keyed by preset name. The parameter type is the
/// SDK's; its serde accepts the shapes persisted by the previous data layer
/// (`"Stocks"`, `parameter`).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Finviz {
    pub screener_parameter: BTreeMap<String, anysdk::finviz_sdk::screener::Parameter>,
}

/// One Market Screener preset: a Finviz preset builds the symbol pool and a
/// tape filter decides which pool symbols' live quotes make it onto which
/// per-exchange table — a symbol enters the table of exchange `E` when its
/// ask or bid side sits on `E` with a size meeting `min_size`.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct MarketScreenerPreset {
    /// Ask/bid exchange codes that qualify (e.g. `K`, `P`); every code gets
    /// its own table. Empty is rejected by the preset form.
    pub exchanges: Vec<String>,
    /// Minimum quote size (shares) the qualifying side must show; `0` = any.
    pub min_size: i64,
    /// The Finviz preset whose export builds the symbol pool.
    pub finviz_preset: String,
}

/// The Market Screener panel's saved presets, keyed by title, plus the
/// panel's subscription sizing knob.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct MarketScreener {
    pub presets: BTreeMap<String, MarketScreenerPreset>,
    /// Symbol-pool cap of the panel's quote subscription; `0` = unlimited.
    pub pool_limit: usize,
}

impl Default for MarketScreener {
    fn default() -> Self {
        Self {
            presets: BTreeMap::new(),
            pool_limit: 200,
        }
    }
}

impl MarketScreener {
    /// The configured pool cap; `None` = unlimited (`pool_limit` 0).
    pub fn pool_limit(&self) -> Option<usize> {
        (self.pool_limit > 0).then_some(self.pool_limit)
    }
}

impl Settings {
    /// Load the settings file, writing a default one when it does not exist
    /// yet. A file that exists but fails to parse is an error (the caller
    /// falls back to defaults in memory without overwriting the file).
    pub fn load() -> anyhow::Result<Self> {
        use anyhow::Context;
        let path = super::app_dir()?.join("settings.json");

        if !path.exists() {
            let default = Self::default();

            if let Some(parent) = path.parent()
                && !parent.as_os_str().is_empty()
            {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create dir: {}", parent.display()))?;
            }

            if let Err(e) = default.save() {
                tracing::warn!(
                    "{}",
                    crate::tr!(
                        "app.log.default_config_write_failed",
                        path = path.display().to_string(),
                        err = format!("{e}")
                    )
                );
            }

            return Ok(default);
        }

        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read config: {}", path.display()))?;
        let config = serde_json::from_str(&contents)
            .with_context(|| format!("failed to parse config: {}", path.display()))?;
        Ok(config)
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = super::app_dir()?.join("settings.json");
        self.save_to(&path)
    }

    /// Atomic write: serialize to a sibling temp file, then rename over the
    /// target — a crash mid-write can never leave a half-written
    /// `settings.json` behind (which would reset every setting to defaults).
    /// `fs::rename` replaces the existing file on Windows; if even that
    /// fails (transient AV/indexer locks on the target), fall back to a
    /// direct write rather than losing the save.
    fn save_to(&self, path: &Path) -> anyhow::Result<()> {
        let contents = serde_json::to_string_pretty(self)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, contents.as_str())?;
        match std::fs::rename(&tmp, path) {
            Ok(()) => Ok(()),
            Err(rename_err) => {
                std::fs::write(path, contents.as_str()).map_err(|write_err| {
                    anyhow::anyhow!(
                        "rename failed ({rename_err:#}) and direct write also failed: {write_err:#}"
                    )
                })?;
                tracing::warn!(
                    "{}",
                    crate::tr!(
                        "app.log.settings_rename_fallback",
                        err = format!("{rename_err:#}")
                    )
                );
                let _ = std::fs::remove_file(&tmp);
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LANGUAGES, Settings};

    /// Every language the UI offers must be a locale rust-i18n actually
    /// loaded (the `locales/` file stems, case-sensitive), and nothing else
    /// should ship. A mismatch here means every `t!` in the app shows its
    /// bare key.
    #[test]
    fn languages_match_locale_files() {
        let mut shipped: Vec<String> = LANGUAGES.iter().map(|(code, _)| code.to_string()).collect();
        shipped.sort_unstable();
        let mut loaded: Vec<String> = crate::_rust_i18n_available_locales()
            .into_iter()
            .map(|locale| locale.into_owned())
            .collect();
        loaded.sort_unstable();
        assert_eq!(loaded, shipped);
    }

    /// A settings file from an older build (fields missing, a nested struct
    /// partially filled — including retired sections like the old `alpaca`
    /// credentials, or fields this build no longer has, like the persisted
    /// server `token`) must still load instead of resetting everything to
    /// defaults.
    #[test]
    fn partial_settings_file_loads() {
        let settings: Settings = serde_json::from_str(
            r#"{"language":"zh-cn","alpaca":{"api_key":"k"},"server":{"addr":"127.0.0.1:9720","token":"legacy"}}"#,
        )
        .expect("partial settings parse");
        assert_eq!(settings.language, "zh-cn");
        assert_eq!(settings.server.addr, "127.0.0.1:9720");
        assert!(!settings.server.tls, "tls defaults to off");
        assert!(
            !settings.window_state.is_empty(),
            "main window record is seeded"
        );
    }

    /// Spot-check that critical translation keys resolve in both locales.
    /// rust-i18n never errors on a missing key — it renders the bare key —
    /// and a YAML edit that accidentally re-parents a block (an indentation
    /// slip) once silenced a whole panel's table headers without any build
    /// or test failing. Extend the list when adding structural locale keys.
    #[test]
    fn critical_keys_translate() {
        let keys = [
            "app.calendar.economic.time",
            "app.calendar.economic.empty",
            "app.calendar.earnings.date",
            "app.calendar.earnings.empty",
            "app.calendar.ipo.company",
            "app.small_clock.alert",
            "app.settings.small_clock",
            "app.auth_code.title",
            "app.tokio_runtime_error",
        ];
        for locale in ["en-us", "zh-cn"] {
            rust_i18n::set_locale(locale);
            for key in keys {
                assert_ne!(
                    crate::tr!(key).to_string(),
                    key,
                    "missing translation: {key} ({locale})"
                );
            }
        }
        rust_i18n::set_locale("en-us");
    }

    /// The atomic save must replace an existing file (the rename path) and
    /// leave no temp file behind; the written file must parse back equal.
    #[test]
    fn atomic_save_replaces_and_round_trips() {
        let dir = std::env::temp_dir().join(format!("zensidus-save-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("settings.json");

        let first = Settings::default();
        first.save_to(&path).expect("first save");
        let second = Settings {
            language: "zh-cn".into(),
            ..Settings::default()
        };
        // Saving over an existing file exercises the rename-replace path.
        second.save_to(&path).expect("second save");

        let reread: Settings =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("saved file exists"))
                .expect("saved file parses");
        assert_eq!(reread.language, "zh-cn");
        assert!(
            !path.with_extension("json.tmp").exists(),
            "temp file is renamed away"
        );
        std::fs::remove_dir_all(&dir).expect("cleanup temp dir");
    }
}
