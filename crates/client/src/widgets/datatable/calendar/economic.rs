//! Economic calendar rows from the Finviz Elite export, fetched through the
//! app's Finviz client.

use chrono::{NaiveDate, Utc};
use gpui_kit::{
    App, Context, Div, InteractiveElement as _, IntoElement, Stateful, Window,
    component::{
        Sizable as _,
        button::{Button, ButtonVariants as _},
        table::{Column, TableDelegate, TableState},
    },
    div,
};

use crate::data::format;
use crate::widgets::datatable::{self, DataPage, Loadable};
use crate::{tr, widgets::dialog::translate};

/// A calendar week's rows; the fetch window is the current Monday–Friday.
/// The row is the export item with its US-Eastern wall clock already
/// resolved to unix seconds (see `utils::finviz`).
pub(crate) type EconomicItem = crate::data::api::EconomicItem;

#[derive(Clone)]
pub struct EconomicDelegate {
    page: DataPage<EconomicItem>,
    /// The week the user picked in the calendar (its Monday); `None` = the
    /// current week.
    pub(crate) selected_week: Option<NaiveDate>,
}

impl EconomicDelegate {
    pub fn new(data: Vec<EconomicItem>) -> Self {
        Self {
            page: DataPage {
                rows: data,
                ..Default::default()
            },
            selected_week: None,
        }
    }

    /// Columns are built per call so a language switch retranslates the
    /// headers on the next render.
    fn column(col_ix: usize) -> Column {
        match col_ix {
            0 => Column::new("time", tr!("app.calendar.economic.time"))
                .width(110.)
                .resizable(false),
            1 => Column::new("event", tr!("app.calendar.economic.event")).width(260.),
            2 => Column::new("for", tr!("app.calendar.economic.for")).width(110.),
            3 => Column::new("actual", tr!("app.calendar.economic.actual")).width(110.),
            4 => Column::new("expected", tr!("app.calendar.economic.expected")).width(110.),
            5 => Column::new("prior", tr!("app.calendar.economic.prior")).width(110.),
            _ => Column::new("", ""),
        }
    }
}

impl Loadable for EconomicDelegate {
    type Row = EconomicItem;

    fn page_mut(&mut self) -> &mut DataPage<Self::Row> {
        &mut self.page
    }
}

impl TableDelegate for EconomicDelegate {
    fn columns_count(&self, _: &App) -> usize {
        6
    }

    fn rows_count(&self, _: &App) -> usize {
        self.page.rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        Self::column(col_ix)
    }

    fn loading(&self, _cx: &App) -> bool {
        self.page.is_loading
    }

    /// Events that have not happened yet (timestamp is in the future) get a
    /// subtle tinted row, so the upcoming side of the week stands out.
    fn render_tr(
        &mut self,
        row_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        let timestamp = self.page.rows.get(row_ix).map(|row| row.time_ms);
        match timestamp {
            Some(timestamp) if timestamp > Utc::now().timestamp_millis() => {
                datatable::tint_upcoming_row("economic-row", row_ix, timestamp, cx)
            }
            _ => div().id(("economic-row", row_ix)),
        }
    }

    fn render_loading(
        &mut self,
        _size: gpui_kit::component::Size,
        _window: &mut Window,
        _cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        datatable::loading_view()
    }

    /// A failed fetch clears the rows; say so instead of the bare icon.
    fn render_empty(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        datatable::empty_view(tr!("app.calendar.economic.empty").to_string().into(), cx)
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let row = &self.page.rows[row_ix];

        match col_ix {
            0 => format::format_ts_local(row.time_ms).into_any_element(),
            // The event title as a button tinted by the impact level: 1 =
            // success, 2 = warning, anything else = danger. Display only —
            // no action is wired yet.
            1 => {
                let event = row.event.clone();
                Button::new(("event-title", row_ix))
                    .small()
                    .label(event.clone())
                    .on_click(move |_, window, app| {
                        translate::open_translate_dialog(window, app, event.clone())
                    })
                    .with_variant(event_variant(row.impact))
                    .into_any_element()
            }
            2 => row.r#for.clone().unwrap_or_default().into_any_element(),
            3 => row.actual.clone().unwrap_or_default().into_any_element(),
            4 => row.expected.clone().unwrap_or_default().into_any_element(),
            5 => row.prior.clone().unwrap_or_default().into_any_element(),
            _ => String::new().into_any_element(),
        }
    }
}

/// The button variant for an impact level.
fn event_variant(impact: u8) -> gpui_kit::component::button::ButtonVariant {
    match impact {
        1 => gpui_kit::component::button::ButtonVariant::Success,
        2 => gpui_kit::component::button::ButtonVariant::Warning,
        _ => gpui_kit::component::button::ButtonVariant::Danger,
    }
}
