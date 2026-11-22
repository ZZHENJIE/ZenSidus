//! Earnings calendar panel: the Finviz Elite earnings calendar as a table,
//! for the picked trading day (weekends are disabled in the date picker).

use chrono::{Datelike, NaiveDate, Weekday};
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
use crate::widgets::datatable::{self, EarningsDelegate};

pub struct EarningsCalendarPanel {
    focus_handle: FocusHandle,
    table: Entity<TableState<EarningsDelegate>>,
    date_picker: Entity<DatePickerState>,
}

impl Panel for EarningsCalendarPanel {
    type DataEntity = TableState<EarningsDelegate>;
    const NAME: &'static str = "EarningsCalendarPanel";
    const ICON: AppIcon = AppIcon::Calendar;
    const TITLE_KEY: &'static str = "app.panels.earnings_calendar";

    fn new(window: &mut Window, cx: &mut Context<Self>, info: &serde_json::Value) -> Self {
        // The startup fetch targets the persisted day when there is one,
        // else the latest trading day. Seed both the picker and the
        // delegate with it so they agree with the table.
        let initial = info
            .get("date")
            .and_then(|value| value.as_str())
            .and_then(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())
            .unwrap_or_else(|| latest_trading_day(chrono::Utc::now().date_naive()));
        // Disable the weekend only: the export reports on a per-day basis.
        let date_picker = cx.new(|cx| {
            let mut state =
                DatePickerState::new(window, cx).disabled_matcher(Matcher::DayOfWeek(vec![0, 6]));
            state.set_date(
                gpui_kit::component::calendar::Date::Single(Some(initial)),
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
            this.table.update(cx, |state, cx| {
                state.delegate_mut().selected_date = Some(date.date());
                cx.notify();
            });
            Self::fetch(this.table.downgrade(), cx);
            cx.notify();
        })
        .detach();

        let this = Self {
            focus_handle: cx.focus_handle(),
            table: datatable::new_table_state(cx, window, move |_| {
                let mut delegate = EarningsDelegate::new(vec![]);
                delegate.selected_date = Some(initial);
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

impl EarningsCalendarPanel {
    /// Fetch the delegate's day (the picked date, or the latest trading day);
    /// a failure lands in the message center.
    fn fetch(table: WeakEntity<TableState<EarningsDelegate>>, cx: &mut App) {
        let Some(state) = table.upgrade() else {
            return;
        };
        let date = state
            .read(cx)
            .delegate()
            .selected_date
            .unwrap_or_else(|| latest_trading_day(chrono::Utc::now().date_naive()));
        datatable::load_rows(
            table,
            tr!("app.calendar.earnings.failed"),
            move |server| async move { crate::data::api::calendar::earnings(&server, date).await },
            cx,
        );
    }
}

/// The latest trading day on or before `date`: the Friday before a weekend.
fn latest_trading_day(date: NaiveDate) -> NaiveDate {
    match date.weekday() {
        Weekday::Sat => date - chrono::Duration::days(1),
        Weekday::Sun => date - chrono::Duration::days(2),
        _ => date,
    }
}

impl gpui_kit::base::dock::Panel for EarningsCalendarPanel {
    fn panel_name(&self) -> &'static str {
        Self::NAME
    }

    /// Persist the picked day; `new` seeds the picker with it, so a restart
    /// keeps the day the user chose instead of resetting to now.
    fn dump(&self, cx: &App) -> PanelState {
        let date = self.table.read(cx).delegate().selected_date;
        let mut state = PanelState::new(Self::NAME);
        state.info = PanelInfo::Panel(match date {
            Some(date) => serde_json::json!({ "date": date.format("%Y-%m-%d").to_string() }),
            None => serde_json::Value::Null,
        });
        state
    }
}

impl EventEmitter<PanelEvent> for EarningsCalendarPanel {}

impl gpui_kit::Focusable for EarningsCalendarPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl gpui_kit::component::dock::Panel for EarningsCalendarPanel {
    fn title(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        Self::default_title()
    }

    fn toolbar_buttons(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Vec<Button>> {
        Some(vec![refresh_button::<Self>(
            "earnings-calendar-refresh",
            self.table.downgrade(),
        )])
    }

    /// The day picker rides the title bar next to the toolbar buttons:
    /// `toolbar_buttons` only takes `Button`s, this hook takes any element.
    fn title_suffix(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        Some(super::common::pinned_date_picker(
            &self.date_picker,
            tr!("app.calendar.earnings.pick_day"),
        ))
    }
}

impl Render for EarningsCalendarPanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(datatable::table_render(&self.table))
    }
}
