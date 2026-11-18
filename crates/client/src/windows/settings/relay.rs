//! Settings → Relay: the relay-server connection (the app's only outbound
//! data link) and the live connection status with connect/disconnect
//! actions. Edits apply on the next connect — the buttons are right below.

use gpui_kit::{
    IntoElement as _, ParentElement as _, Styled as _,
    base::h_flex,
    component::{
        ActiveTheme as _, Disableable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        setting::{SettingField, SettingGroup, SettingItem, SettingPage},
    },
    div, px,
};

use crate::tr;
use crate::utils::state::{RelayStatus, AppState};
use crate::utils::{AppIcon, settings};
use crate::windows::settings::{SettingsWindow, text_setting};

/// Status dot + label, reflecting the hub live (the window subscribes to
/// `RelayHubEvent::StatusChanged`). A `Failed` status appends the reason so
/// the failure is explainable without opening the logs.
fn relay_status_item() -> SettingItem {
    SettingItem::render(move |_, _, cx| {
        let status: RelayStatus = cx.global::<AppState>().relay_hub.read(cx).status().clone();
        h_flex()
            .gap_2()
            .child(div().size(px(8.)).rounded_full().bg(status.color(cx)))
            .child(div().child(status.label()))
            .children(
                status
                    .reason()
                    .filter(|reason| !reason.is_empty())
                    .map(|reason| {
                        div()
                            .text_color(cx.theme().muted_foreground)
                            .text_sm()
                            .child(reason.clone())
                    }),
            )
            .into_any_element()
    })
    .keywords(["relay status"])
}

fn relay_actions_item() -> SettingItem {
    SettingItem::render(move |_, _, cx| {
        let hub = cx.global::<AppState>().relay_hub.clone();
        let connecting = matches!(hub.read(cx).status(), RelayStatus::Connecting);
        let up = hub.read(cx).connected();

        let connect = {
            let hub = hub.clone();
            Button::new("relay-connect")
                .primary()
                .small()
                .label(tr!("app.settings.connect"))
                .disabled(connecting || up)
                .on_click(move |_, _, cx| {
                    // Address/token/TLS are re-read by the hub itself.
                    hub.update(cx, |hub, cx| hub.connect(cx));
                })
        };
        let disconnect = Button::new("relay-disconnect")
            .outline()
            .small()
            .label(tr!("app.settings.disconnect"))
            .disabled(!up && !connecting)
            .on_click({
                let hub = hub.clone();
                move |_, _, cx| hub.update(cx, |hub, cx| hub.disconnect(cx))
            });

        h_flex().gap_2().child(connect).child(disconnect)
    })
    .keywords(["relay connect disconnect"])
}

impl SettingsWindow {
    pub(super) fn relay_page(&self) -> SettingPage {
        super::settings_page("app.settings.relay", AppIcon::Market)
            .description(tr!("app.settings.relay_desc"))
            .group(
                SettingGroup::new()
                    .title(tr!("app.settings.connection"))
                    .item(
                        SettingItem::new(
                            tr!("app.settings.relay_addr"),
                            text_setting(|s| &s.relay.addr, |s, v| s.relay.addr = v),
                        )
                        .description(tr!("app.settings.relay_addr_desc")),
                    )
                    .item(
                        SettingItem::new(
                            tr!("app.settings.relay_token"),
                            text_setting(|s| &s.relay.token, |s, v| s.relay.token = v),
                        )
                        .description(tr!("app.settings.relay_token_desc")),
                    )
                    .item(
                        SettingItem::new(
                            tr!("app.settings.relay_tls"),
                            SettingField::switch(
                                |cx| cx.global::<AppState>().settings.relay.tls,
                                |value, cx| {
                                    settings::update_settings(cx, |s| s.relay.tls = value);
                                },
                            ),
                        )
                        .description(tr!("app.settings.relay_tls_desc")),
                    )
                    .item(
                        SettingItem::new(
                            tr!("app.settings.relay_tls_cert"),
                            text_setting(|s| &s.relay.tls_cert, |s, v| s.relay.tls_cert = v),
                        )
                        .description(tr!("app.settings.relay_tls_cert_desc")),
                    )
                    .item(relay_status_item())
                    .item(relay_actions_item()),
            )
    }
}
