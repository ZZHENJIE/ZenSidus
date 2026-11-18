//! Settings → General: UI language and the theme colors.

use gpui_kit::{
    Entity, SharedString,
    component::{
        color_picker::{ColorPicker, ColorPickerState},
        label::Label,
        setting::{SettingField, SettingGroup, SettingItem, SettingPage},
    },
};

use crate::tr;
use crate::utils::state::AppState;
use crate::utils::{AppIcon, settings};
use crate::windows::settings::SettingsWindow;

fn language_field() -> SettingField<SharedString> {
    SettingField::dropdown(
        settings::LANGUAGES
            .iter()
            .map(|(code, name)| (SharedString::from(*code), SharedString::from(*name)))
            .collect(),
        |cx| cx.global::<AppState>().settings.language.clone().into(),
        |value, cx| {
            rust_i18n::set_locale(&value);
            settings::update_settings(cx, |s| s.language = value.to_string());
        },
    )
    .default_value(settings::LANGUAGES[0].0)
}

impl SettingsWindow {
    fn color_field(&self, picker: &Entity<ColorPickerState>) -> SettingField<SharedString> {
        let picker = picker.clone();
        SettingField::render(move |_, _, _| ColorPicker::new(&picker))
    }

    pub(super) fn general_page(&self) -> SettingPage {
        let mut colors_group = SettingGroup::new().title(tr!("app.settings.colors"));
        for (field, picker) in self.tone_pickers() {
            colors_group = colors_group.item(
                SettingItem::new(tr!(field.label_key()), self.color_field(picker))
                    .description(tr!(field.desc_key())),
            );
        }

        super::settings_page("app.settings.general", AppIcon::Settings)
            .group(SettingGroup::new().item(SettingItem::new(
                tr!("app.settings.language"),
                language_field(),
            )))
            .group(
                SettingGroup::new()
                    .title(tr!("app.settings.theme"))
                    .description(tr!("app.settings.theme_desc"))
                    .item(
                        SettingItem::new(
                            tr!("app.settings.color_primary"),
                            self.color_field(&self.primary_picker),
                        )
                        .description(tr!("app.settings.color_primary_desc")),
                    )
                    .item(
                        SettingItem::new(
                            tr!("app.settings.color_bullish"),
                            self.color_field(&self.bullish_picker),
                        )
                        .description(tr!("app.settings.color_bullish_desc")),
                    )
                    .item(
                        SettingItem::new(
                            tr!("app.settings.color_bearish"),
                            self.color_field(&self.bearish_picker),
                        )
                        .description(tr!("app.settings.color_bearish_desc")),
                    )
                    .footer(|_, _| Label::new(tr!("app.settings.theme_hint"))),
            )
            .group(colors_group)
    }
}
