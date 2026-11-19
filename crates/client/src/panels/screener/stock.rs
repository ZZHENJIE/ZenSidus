//! Screener panel: one saved Finviz preset's export as a table. The preset
//! picker (with add/delete in its dropdown footer) rides the tab title bar;
//! the selection is persisted per panel instance.

use gpui_kit::{
    App, Context, Entity, EventEmitter, FocusHandle, IntoElement, ParentElement as _, Render,
    SharedString, Styled, WeakEntity, Window,
    assets::IconName,
    base::dock::{PanelEvent, PanelInfo, PanelState},
    component::{
        button::Button, combobox::ComboboxEvent, searchable_list::SearchableVec, table::TableState,
    },
    div, px,
};

use protocol::{RpcRequestPayload, RpcResponsePayload};

use crate::panels::{Panel, panel_title, refresh_button};
use crate::tr;
use crate::utils::state::AppState;
use crate::utils::{AppIcon, message};
use crate::widgets::datatable::{self, StockScreenerDelegate};
use crate::widgets::dialog::{open_confirm_dialog, open_screener_preset_dialog};
use crate::widgets::picker::{PickerAction, StringPicker};

pub struct StockScreenerPanel {
    focus_handle: FocusHandle,
    picker: StringPicker,
    table: Entity<TableState<StockScreenerDelegate>>,
}

impl Panel for StockScreenerPanel {
    type DataEntity = TableState<StockScreenerDelegate>;
    const NAME: &'static str = "StockScreenerPanel";
    const ICON: AppIcon = AppIcon::Filter;
    const TITLE_KEY: &'static str = "app.panels.stock_screener";

    fn new(window: &mut Window, cx: &mut Context<Self>, info: &serde_json::Value) -> Self {
        let focus_handle = cx.focus_handle();
        let persisted_preset = info
            .get("preset")
            .and_then(|value| value.as_str())
            .map(String::from);

        let delegate = StockScreenerDelegate::new(persisted_preset, cx);
        let picker = StringPicker::new(
            StockScreenerDelegate::preset_names(cx),
            delegate.preset.clone(),
            window,
            cx,
        );
        let table = datatable::new_table_state(cx, window, move |_| delegate);

        // Preset lists are shared settings; follow edits made elsewhere
        // (settings Finviz page or other Screener instances).
        cx.observe_global_in::<AppState>(window, |this, window, cx| {
            let changed = this.table.update(cx, |state, cx| {
                let changed = state.delegate_mut().sync_with_settings(cx);
                if changed {
                    cx.notify();
                }
                changed
            });
            if changed {
                Self::fetch(this.table.downgrade(), cx);
            }
            // The picker mirrors the delegate once the sync has settled.
            let current = this.preset(cx);
            this.picker
                .set_items(StockScreenerDelegate::preset_names(cx), window, cx);
            this.picker.set_selected(current, window, cx);
            cx.notify();
        })
        .detach();

        // Picking a preset in the picker loads its data.
        cx.subscribe(
            picker.state(),
            |this, _, event: &ComboboxEvent<SearchableVec<String>>, cx| {
                if let ComboboxEvent::Change(values) = event
                    && let Some(name) = values.last()
                {
                    this.select_preset(name.clone(), cx);
                }
            },
        )
        .detach();

        let this = Self {
            focus_handle,
            picker,
            table,
        };
        // Auto-load once when a preset is selected.
        if this.preset(cx).is_some() {
            Self::fetch(this.table.downgrade(), cx);
        }
        this
    }

    fn refresh(entity: WeakEntity<Self::DataEntity>, cx: &mut App) {
        Self::fetch(entity, cx)
    }
}

impl StockScreenerPanel {
    fn preset(&self, cx: &App) -> Option<String> {
        self.table.read(cx).delegate().preset.clone()
    }

    /// Switch to `name` and load it. Re-selecting the current preset is a
    /// no-op; the toolbar refresh button is the explicit way to refetch.
    fn select_preset(&mut self, name: String, cx: &mut Context<Self>) {
        let changed = self.table.update(cx, |state, cx| {
            let changed = state.delegate_mut().select(name);
            if changed {
                cx.notify();
            }
            changed
        });
        if changed {
            Self::fetch(self.table.downgrade(), cx);
            cx.notify();
        }
    }

    fn add_preset(
        &mut self,
        name: String,
        parameter: anysdk::finviz_sdk::screener::Parameter,
        cx: &mut Context<Self>,
    ) {
        StockScreenerDelegate::add_preset(name.clone(), parameter, cx);
        self.select_preset(name, cx);
    }

    /// Fetch the selected preset's export into the table through the app's
    /// Finviz client. Without an API key nothing is sent: the user gets a
    /// message and the table stays idle.
    fn fetch(table: WeakEntity<TableState<StockScreenerDelegate>>, cx: &mut App) {
        let Some(preset) = table
            .upgrade()
            .and_then(|table| table.read(cx).delegate().preset.clone())
        else {
            return;
        };
        let Some(parameter) = cx
            .global::<AppState>()
            .settings
            .finviz
            .screener_parameter
            .get(&preset)
            .cloned()
        else {
            return;
        };
        // Finviz token 已下沉服务端;唯一的前置条件是 relay 在线。
        let Ok(relay) = AppState::relay(cx) else {
            message::error(
                tr!("app.stock_screener.no_relay"),
                anyhow::anyhow!("relay is not connected"),
                cx,
            );
            return;
        };
        let parameter = super::screener_parameter_dto(&parameter);

        // Stamp the fetch generation: a preset switch or settings sync while
        // this request is in flight invalidates its outcome.
        let Ok(generation) = table.update(cx, |state, cx| {
            let delegate = state.delegate_mut();
            delegate.is_loading = true;
            delegate.fetch_generation += 1;
            cx.notify();
            delegate.fetch_generation
        }) else {
            return;
        };
        cx.spawn(async move |cx| {
            let result = async {
                match relay
                    .rpc_checked(RpcRequestPayload::ScreenerExport { parameter })
                    .await?
                {
                    RpcResponsePayload::ScreenerExport(export) => Ok(export),
                    other => Err(anyhow::anyhow!("unexpected rpc response: {other:?}")),
                }
            }
            .await;
            match result {
                Ok(result) => {
                    let _ = table.update(cx, |state, cx| {
                        let delegate = state.delegate_mut();
                        if delegate.fetch_generation != generation {
                            // Superseded while in flight: discard silently,
                            // the newer fetch owns the loading flag now.
                            return;
                        }
                        delegate.set_data(result.headers, result.rows);
                        // The columns follow the export's headers: rebuild
                        // the table's column-group snapshot.
                        state.refresh(cx);
                        cx.notify();
                    });
                }
                Err(err) => {
                    let _ = table.update(cx, |state, cx| {
                        let delegate = state.delegate_mut();
                        if delegate.fetch_generation != generation {
                            return;
                        }
                        delegate.is_loading = false;
                        cx.notify();
                    });
                    cx.update(|cx| message::error(tr!("app.stock_screener.failed"), err, cx));
                }
            }
        })
        .detach();
    }
}

impl gpui_kit::base::dock::Panel for StockScreenerPanel {
    fn panel_name(&self) -> &'static str {
        Self::NAME
    }

    /// Persist the selected preset per instance; `new` reads it back.
    fn dump(&self, cx: &App) -> PanelState {
        let mut state = PanelState::new(Self::NAME);
        state.info = PanelInfo::Panel(serde_json::json!({ "preset": self.preset(cx) }));
        state
    }
}

impl EventEmitter<PanelEvent> for StockScreenerPanel {}

impl gpui_kit::Focusable for StockScreenerPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl gpui_kit::component::dock::Panel for StockScreenerPanel {
    /// The selected preset as the tab label; the panel name until one exists.
    fn title(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let label = self
            .preset(cx)
            .map(SharedString::from)
            .unwrap_or_else(|| tr!(Self::TITLE_KEY));
        panel_title(Self::ICON, label)
    }

    fn toolbar_buttons(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Vec<Button>> {
        Some(vec![refresh_button::<Self>(
            "screener-refresh",
            self.table.downgrade(),
        )])
    }

    /// The preset picker rides the tab title bar, next to the toolbar
    /// buttons. It is pinned to a fixed width: the combobox inside is
    /// `w_full`, and unbounded it would eat the whole tab row and push the
    /// toolbar out of view (the DatePicker trap). Preset management
    /// (new/edit/delete) lives in the shared picker footer: edit reopens the
    /// preset form seeded with the selected preset.
    fn title_suffix(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        let weak = cx.weak_entity();

        Some(
            div()
                .flex_shrink_0()
                .w(px(200.))
                .child(self.picker.managed_element(
                    tr!("app.stock_screener.pick_preset"),
                    {
                        let weak = weak.clone();
                        move |cx| weak.upgrade().and_then(|p| p.read(cx).preset(cx))
                    },
                    Some(PickerAction::new(
                        "screener-add-preset",
                        IconName::Plus,
                        tr!("app.stock_screener.add_preset"),
                        {
                            let weak = weak.clone();
                            move |window, cx, _| {
                                open_screener_preset_dialog(window, cx, None, {
                                    let weak = weak.clone();
                                    move |draft, cx| {
                                        let (name, parameter) = draft;
                                        let _ = weak.update(cx, |this, cx| {
                                            this.add_preset(name, parameter, cx)
                                        });
                                    }
                                });
                            }
                        },
                    )),
                    Some(PickerAction::new(
                        "screener-edit-preset",
                        AppIcon::Pencil,
                        tr!("app.stock_screener.edit_preset"),
                        {
                            let weak = weak.clone();
                            move |window, cx, selected| {
                                let Some(name) = selected else {
                                    return;
                                };
                                let parameter = cx
                                    .global::<AppState>()
                                    .settings
                                    .finviz
                                    .screener_parameter
                                    .get(&name)
                                    .cloned()
                                    .unwrap_or_default();
                                let previous = name.clone();
                                open_screener_preset_dialog(window, cx, Some((name, parameter)), {
                                    let weak = weak.clone();
                                    move |draft, cx| {
                                        let (name, parameter) = draft;
                                        let _ = weak.update(cx, |_, cx| {
                                            StockScreenerDelegate::upsert_preset(
                                                Some(&previous),
                                                name,
                                                parameter,
                                                cx,
                                            )
                                        });
                                    }
                                });
                            }
                        },
                    )),
                    Some(PickerAction::new(
                        "screener-delete-preset",
                        IconName::Trash,
                        tr!("app.stock_screener.delete_preset"),
                        {
                            move |window, cx, selected| {
                                let Some(name) = selected else {
                                    return;
                                };
                                open_confirm_dialog(
                                    window,
                                    cx,
                                    tr!("app.stock_screener.delete_preset"),
                                    tr!("app.stock_screener.delete_confirm", name = name.clone()),
                                    move |cx| {
                                        StockScreenerDelegate::remove_preset(&name, cx);
                                    },
                                );
                            }
                        },
                    )),
                )),
        )
    }
}

impl Render for StockScreenerPanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(datatable::table_render(&self.table))
    }
}
