//! Economic calendar panel: the Finviz Elite economic calendar as a table,
//! for the current week or a week picked in the date picker (Mondays only).

use chrono::{Datelike, NaiveDate};
use protocol::{RpcRequestPayload, RpcResponsePayload};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, IntoElement,
    ParentElement as _, Render, Styled, WeakEntity, Window,
    base::dock::{PanelEvent, PanelInfo, PanelState},
    component::{
        button::Button,
        calendar::Matcher,
        date_picker::{DatePickerEvent, DatePickerState},
        table::TableState,
    },
    div,
};

use crate::panels::{Panel, refresh_button};
use crate::tr;
use crate::utils::AppIcon;
use crate::utils::state::AppState;
use crate::widgets::datatable::{self, EconomicDelegate};

pub struct EconomicCalendarPanel {
    focus_handle: FocusHandle,
    table: Entity<TableState<EconomicDelegate>>,
    date_picker: Entity<DatePickerState>,
}

impl Panel for EconomicCalendarPanel {
    type DataEntity = TableState<EconomicDelegate>;
    const NAME: &'static str = "EconomicCalendarPanel";
    const ICON: AppIcon = AppIcon::Calendar;
    const TITLE_KEY: &'static str = "app.panels.economic_calendar";

    fn new(window: &mut Window, cx: &mut Context<Self>, info: &serde_json::Value) -> Self {
        // The startup fetch targets the persisted week when there is one,
        // else the current week; seed both the picker and the delegate with
        // its Monday so they agree with the table.
        let monday = info
            .get("date")
            .and_then(|value| value.as_str())
            .and_then(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())
            .unwrap_or_else(|| snap_to_monday(chrono::Utc::now().date_naive()));
        // Disable every weekday except Monday: the export's window is a week.
        let date_picker = cx.new(|cx| {
            let mut state = DatePickerState::new(window, cx)
                .disabled_matcher(Matcher::DayOfWeek(vec![0, 2, 3, 4, 5, 6]));
            state.set_date(
                gpui_kit::component::calendar::Date::Single(Some(monday)),
                window,
                cx,
            );
            state
        });
        cx.subscribe(&date_picker, |this, _, event: &DatePickerEvent, cx| {
            // The picker emits exactly one event kind.
            let DatePickerEvent::Change(date_time) = event;
            let Some(date) = date_time.start() else {
                return;
            };
            let monday = snap_to_monday(date.date());
            this.table.update(cx, |state, cx| {
                state.delegate_mut().selected_week = Some(monday);
                cx.notify();
            });
            Self::fetch(this.table.downgrade(), cx);
            cx.notify();
        })
        .detach();

        let this = Self {
            focus_handle: cx.focus_handle(),
            table: datatable::new_table_state(cx, window, move |_| {
                let mut delegate = EconomicDelegate::new(vec![]);
                delegate.selected_week = Some(monday);
                delegate
            }),
            date_picker,
        };
        // Follow settings changes (e.g. a language switch) to retranslate
        // the column headers. The data itself is not settings-dependent, so
        // no refetch.
        cx.observe_global_in::<AppState>(window, |_, _, cx| cx.notify())
            .detach();
        Self::fetch(this.table.downgrade(), cx);
        // Offline at startup: the fetch above was skipped silently; load
        // once the connection lands.
        crate::panels::reload_on_connect::<Self>(this.table.downgrade(), cx);
        this
    }

    fn refresh(entity: WeakEntity<Self::DataEntity>, cx: &mut App) {
        Self::fetch(entity, cx)
    }
}

impl EconomicCalendarPanel {
    /// Fetch the delegate's week (the picked Monday, or the current one);
    /// a failure lands in the message center.
    fn fetch(table: WeakEntity<TableState<EconomicDelegate>>, cx: &mut App) {
        let Some(state) = table.upgrade() else {
            return;
        };
        // The server snaps the date to its week's Mon–Fri window; we only
        // say which week we want (the picked Monday, or today).
        let date = state
            .read(cx)
            .delegate()
            .selected_week
            .unwrap_or_else(|| chrono::Utc::now().date_naive());
        datatable::load_rows(
            table,
            tr!("app.calendar.economic.failed"),
            move |server| async move {
                match server
                    .rpc_checked(RpcRequestPayload::EconomicCalendar {
                        date: date.to_string(),
                    })
                    .await?
                {
                    RpcResponsePayload::EconomicCalendar(rows) => Ok(rows),
                    other => Err(anyhow::anyhow!("unexpected rpc response: {other:?}")),
                }
            },
            cx,
        );
    }
}

/// Snap any date to the Monday of its week.
fn snap_to_monday(date: NaiveDate) -> NaiveDate {
    date - chrono::Duration::days(date.weekday().num_days_from_monday() as i64)
}

impl gpui_kit::base::dock::Panel for EconomicCalendarPanel {
    fn panel_name(&self) -> &'static str {
        Self::NAME
    }

    /// Persist the picked week's Monday; `new` seeds the picker with it, so
    /// a restart keeps the week the user chose instead of resetting to now.
    fn dump(&self, cx: &App) -> PanelState {
        let week = self.table.read(cx).delegate().selected_week;
        let mut state = PanelState::new(Self::NAME);
        state.info = PanelInfo::Panel(match week {
            Some(monday) => serde_json::json!({ "date": monday.format("%Y-%m-%d").to_string() }),
            None => serde_json::Value::Null,
        });
        state
    }
}

impl EventEmitter<PanelEvent> for EconomicCalendarPanel {}

impl gpui_kit::Focusable for EconomicCalendarPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl gpui_kit::component::dock::Panel for EconomicCalendarPanel {
    fn title(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        Self::default_title()
    }

    fn toolbar_buttons(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Vec<Button>> {
        Some(vec![refresh_button::<Self>(
            "economic-calendar-refresh",
            self.table.downgrade(),
        )])
    }

    /// The week picker rides the title bar next to the toolbar buttons:
    /// `toolbar_buttons` only takes `Button`s, this hook takes any element.
    fn title_suffix(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        Some(super::common::pinned_date_picker(
            &self.date_picker,
            tr!("app.calendar.economic.pick_week"),
        ))
    }
}

impl Render for EconomicCalendarPanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(datatable::table_render(&self.table))
    }
}
