//! Settings → Small Clock: economic event alerting of the clock window.

use gpui_kit::{
    SharedString,
    component::setting::{
        NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage,
    },
};

use crate::tr;
use crate::utils::{AppIcon, settings};
use crate::windows::settings::SettingsWindow;

impl SettingsWindow {
    pub(super) fn small_clock_page(&self) -> SettingPage {
        super::settings_page("app.settings.small_clock", AppIcon::SmallClock).group(
            SettingGroup::new()
                .item(
                    SettingItem::new(
                        tr!("app.settings.alert_minutes"),
                        SettingField::number_input(
                            NumberFieldOptions {
                                min: 1.,
                                max: 120.,
                                step: 1.,
                            },
                            |cx| {
                                cx.global::<crate::utils::state::AppState>()
                                    .settings
                                    .small_clock
                                    .alert_minutes as f64
                            },
                            |value, cx| {
                                settings::update_settings(cx, |s| {
                                    s.small_clock.alert_minutes = value.round().max(1.) as u32;
                                });
                            },
                        ),
                    )
                    .description(tr!("app.settings.alert_minutes_desc")),
                )
                .item(SettingItem::new(
                    tr!("app.settings.alert_sound"),
                    SettingField::switch(
                        |cx| {
                            cx.global::<crate::utils::state::AppState>()
                                .settings
                                .small_clock
                                .sound
                        },
                        |value, cx| {
                            settings::update_settings(cx, |s| s.small_clock.sound = value);
                        },
                    ),
                ))
                .item(SettingItem::new(
                    tr!("app.settings.min_impact"),
                    SettingField::dropdown(
                        LEVELS
                            .iter()
                            .map(|(value, key)| (SharedString::from(*value), tr!(*key)))
                            .collect(),
                        |cx| {
                            cx.global::<crate::utils::state::AppState>()
                                .settings
                                .small_clock
                                .min_impact
                                .to_string()
                                .into()
                        },
                        |value, cx| {
                            settings::update_settings(cx, |s| {
                                s.small_clock.min_impact = value.parse().unwrap_or(2);
                            });
                        },
                    ),
                )),
        )
    }
}

/// The economic impact levels of the Finviz export, as (value, label key).
const LEVELS: [(&str, &str); 3] = [
    ("1", "app.settings.level_low"),
    ("2", "app.settings.level_medium"),
    ("3", "app.settings.level_high"),
];
