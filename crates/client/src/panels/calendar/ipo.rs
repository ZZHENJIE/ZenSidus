//! IPO calendar panel: the iposcoop.com calendar as a table, with refresh
//! and "open the source site" buttons.

use gpui_kit::{
    App, Context, Entity, EventEmitter, FocusHandle, IntoElement, ParentElement as _, Render,
    Styled, WeakEntity, Window,
    base::dock::PanelEvent,
    component::{button::Button, table::TableState},
    div,
};

use protocol::{RpcRequestPayload, RpcResponsePayload};

use crate::panels::{Panel, refresh_button};
use crate::tr;
use crate::utils::AppIcon;
use crate::utils::state::AppState;
use crate::widgets::datatable::{self, IPODelegate};

pub struct IPOCalendarPanel {
    focus_handle: FocusHandle,
    table: Entity<TableState<IPODelegate>>,
}

impl Panel for IPOCalendarPanel {
    type DataEntity = TableState<IPODelegate>;
    const NAME: &'static str = "IPOCalendarPanel";
    const ICON: AppIcon = AppIcon::Calendar;
    const TITLE_KEY: &'static str = "app.panels.ipo_calendar";

    fn new(window: &mut Window, cx: &mut Context<Self>, _info: &serde_json::Value) -> Self {
        let this = Self {
            focus_handle: cx.focus_handle(),
            table: datatable::new_table_state(cx, window, |_| IPODelegate::new(vec![])),
        };
        // Follow settings changes (e.g. a language switch) to retranslate
        // the column headers. The data itself is not settings-dependent, so
        // no refetch.
        cx.observe_global_in::<AppState>(window, |_, _, cx| cx.notify())
            .detach();
        Self::refresh(this.table.downgrade(), cx);
        // Offline at startup: the fetch above was skipped silently; load
        // once the connection lands.
        crate::panels::reload_on_connect::<Self>(this.table.downgrade(), cx);
        this
    }

    /// Fetch the calendar; a failure lands in the message center.
    fn refresh(entity: WeakEntity<Self::DataEntity>, cx: &mut App) {
        datatable::load_rows(
            entity,
            tr!("app.calendar.ipo.failed"),
            |server| async move {
                match server.rpc_checked(RpcRequestPayload::IpoCalendar).await? {
                    RpcResponsePayload::IpoCalendar(items) => Ok(items),
                    other => Err(anyhow::anyhow!("unexpected rpc response: {other:?}")),
                }
            },
            cx,
        );
    }
}

impl gpui_kit::base::dock::Panel for IPOCalendarPanel {
    fn panel_name(&self) -> &'static str {
        Self::NAME
    }
}

impl EventEmitter<PanelEvent> for IPOCalendarPanel {}

impl gpui_kit::Focusable for IPOCalendarPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl gpui_kit::component::dock::Panel for IPOCalendarPanel {
    fn title(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        Self::default_title()
    }

    fn toolbar_buttons(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Vec<Button>> {
        Some(vec![refresh_button::<Self>(
            "ipo-refresh",
            self.table.downgrade(),
        )])
    }
}

impl Render for IPOCalendarPanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(datatable::table_render(&self.table))
    }
}
