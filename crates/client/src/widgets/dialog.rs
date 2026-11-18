//! Modal dialogs. Each is an `open_*(window, cx, …)` entry point callable
//! from any click handler; dialog-local state entities are created before
//! `open_dialog` because the builder closure re-runs on every update.

pub mod confirm;
pub mod market_preset;
pub mod screener_preset;
pub mod search_symbol;
pub mod translate;

pub use confirm::{dialog_footer, open_confirm_dialog};
pub use market_preset::{MarketPresetDraft, open_market_preset_dialog};
pub use screener_preset::{PresetDraft, open_screener_preset_dialog};
pub use search_symbol::open_search_symbol_dialog;
pub use translate::open_translate_dialog;
