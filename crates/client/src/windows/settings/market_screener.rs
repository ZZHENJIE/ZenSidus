//! Settings → Market Screener: the subscription sizing of the panel's
//! live quote pool.

use gpui_kit::component::setting::{
    NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage,
};

use crate::tr;
use crate::utils::{AppIcon, settings, state::AppState};
use crate::windows::settings::SettingsWindow;

impl SettingsWindow {
    pub(super) fn market_screener_page(&self) -> SettingPage {
        super::settings_page("app.settings.market_screener", AppIcon::Filter).group(
            SettingGroup::new().item(
                SettingItem::new(
                    tr!("app.settings.pool_limit"),
                    SettingField::number_input(
                        NumberFieldOptions {
                            min: 0.,
                            max: 5000.,
                            step: 50.,
                        },
                        |cx| cx.global::<AppState>().settings.market_screener.pool_limit as f64,
                        |value, cx| {
                            settings::update_settings(cx, |s| {
                                s.market_screener.pool_limit = value.round().max(0.) as usize;
                            });
                        },
                    ),
                )
                .description(tr!("app.settings.pool_limit_desc")),
            ),
        )
    }
}
