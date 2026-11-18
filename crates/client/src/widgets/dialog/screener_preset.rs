//! The screener-preset form, shared by the settings Finviz page and the
//! Screener panel footer. Opens blank ("New Screener Preset") or seeded
//! ("Edit Screener Preset") when an `initial` preset is passed. The export
//! columns are picked from a searchable multi-select combobox; the committed
//! `Parameter.columns` is a comma-separated list of column indexes in
//! selection order.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::{
    App, AppContext as _, Entity, IntoElement as _, ParentElement as _, SharedString, Styled as _,
    Window,
    base::{h_flex, v_flex},
    component::{
        ActiveTheme as _, Sizable as _, WindowExt,
        combobox::{Combobox, ComboboxEvent, ComboboxState},
        input::{Input, InputState},
        label::Label,
        radio::Radio,
        searchable_list::{SearchableListItem, SearchableVec},
    },
    div, px,
};

use anysdk::finviz_sdk::screener::{EXPORT_COLUMNS, Parameter, ScreenerType};

use crate::tr;
use crate::widgets::dialog::dialog_footer;

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

/// The values the form commits: the preset name and its parameters.
pub type PresetDraft = (String, Parameter);

/// One entry of the Finviz export-column table, as a combobox item: the
/// value is the column's `c=` index (a string, the combobox's value type),
/// the label reads "Name (index)".
#[derive(Clone)]
struct FinvizColumn {
    index: u16,
    name: SharedString,
    value: String,
}

impl SearchableListItem for FinvizColumn {
    type Value = String;

    fn title(&self) -> SharedString {
        SharedString::from(format!("{} ({})", self.name, self.index))
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }

    fn matches(&self, query: &str) -> bool {
        let query = query.to_lowercase();
        self.name.to_lowercase().contains(&query) || self.index.to_string().contains(&query)
    }
}

/// The columns field: a searchable multi-select over [`EXPORT_COLUMNS`].
/// `selected` is kept up to date on every toggle (selection order); `unknown`
/// holds stored tokens the table has no item for, so an edited preset never
/// silently loses them.
#[derive(Clone)]
struct ColumnsField {
    state: Entity<ComboboxState<SearchableVec<FinvizColumn>>>,
    selected: Rc<std::cell::RefCell<Vec<String>>>,
    unknown: Rc<std::cell::RefCell<Vec<String>>>,
}

impl ColumnsField {
    fn new(initial: Option<&String>, window: &mut Window, cx: &mut App) -> Self {
        let items: Vec<FinvizColumn> = EXPORT_COLUMNS
            .iter()
            .map(|(index, name)| FinvizColumn {
                index: *index,
                name: SharedString::from(*name),
                value: index.to_string(),
            })
            .collect();
        let state = cx.new(|cx| {
            ComboboxState::new(SearchableVec::new(items), vec![], window, cx)
                .searchable(true)
                .multiple(true)
        });

        // Seed from the stored list: known indexes become selections, the
        // rest ride along so editing + confirming never loses them.
        let selected = Rc::new(std::cell::RefCell::new(Vec::new()));
        let unknown = Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut known: Vec<String> = Vec::new();
        if let Some(stored) = initial {
            for token in stored.split(',') {
                let token = token.trim();
                if token.is_empty() {
                    continue;
                }
                match token.parse::<u16>() {
                    Ok(index) if EXPORT_COLUMNS.iter().any(|(i, _)| *i == index) => {
                        known.push(token.to_string());
                    }
                    _ => unknown.borrow_mut().push(token.to_string()),
                }
            }
        }
        if !known.is_empty() {
            state.update(cx, |state, cx| {
                state.set_selected_values(&known, window, cx);
            });
        }
        *selected.borrow_mut() = known;

        Self {
            state,
            selected,
            unknown,
        }
    }

    /// The committed value: the selected indexes in selection order, with
    /// unknown stored tokens appended.
    fn commit(&self) -> Option<String> {
        let mut tokens = self.selected.borrow().clone();
        tokens.extend(self.unknown.borrow().iter().cloned());
        let tokens: Vec<String> = tokens
            .iter()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect();
        non_empty(&tokens.join(","))
    }

    /// The styled element: chips of the first few selections with a "+N"
    /// overflow badge (the full list edits in the popover).
    fn element(&self, placeholder: SharedString) -> impl gpui_kit::IntoElement {
        let placeholder_for_trigger = placeholder.clone();
        Combobox::new(&self.state)
            .small()
            .w_full()
            .menu_width(px(340.))
            .cleanable(false)
            .placeholder(placeholder)
            .render_trigger(move |trigger, _window, cx| {
                let selection = trigger.selection();
                if selection.is_empty() {
                    return div()
                        .text_color(cx.theme().muted_foreground)
                        .child(placeholder_for_trigger.clone())
                        .into_any_element();
                }
                let muted = cx.theme().muted_foreground;
                let secondary = cx.theme().secondary;
                const MAX_CHIPS: usize = 3;
                let overflow = selection.len().saturating_sub(MAX_CHIPS);
                h_flex()
                    .w_full()
                    .overflow_hidden()
                    .gap_1()
                    .children(selection.iter().take(MAX_CHIPS).map(|(_, item)| {
                        div()
                            .px_1()
                            .rounded_sm()
                            .text_xs()
                            .bg(secondary)
                            .child(item.title().to_string())
                    }))
                    .children((overflow > 0).then(|| {
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(format!("+{overflow}"))
                    }))
                    .into_any_element()
            })
    }
}

/// Open the form. Input entities are created before opening (the builder
/// may re-run); `on_confirm` receives the named preset and persists it
/// however the caller wants. Pass `Some((name, parameter))` to edit an
/// existing preset: every field comes prefilled (an empty optional field
/// stays empty) and the title reads "Edit".
pub fn open_screener_preset_dialog(
    window: &mut Window,
    cx: &mut App,
    initial: Option<PresetDraft>,
    on_confirm: impl Fn(PresetDraft, &mut App) + 'static,
) {
    let initial = initial.unwrap_or_else(|| {
        (
            String::new(),
            Parameter {
                order_by: "-volume".to_string(),
                ..Parameter::default()
            },
        )
    });
    let (initial_name, initial_param) = initial;

    let seed = |value: Option<&String>| value.cloned().unwrap_or_default();
    let name_input = cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(tr!("app.settings.finviz_preset_name"))
            .default_value(initial_name.clone())
    });
    let signal_input =
        cx.new(|cx| InputState::new(window, cx).default_value(seed(initial_param.signal.as_ref())));
    let order_input =
        cx.new(|cx| InputState::new(window, cx).default_value(initial_param.order_by.clone()));
    let filter_input = cx
        .new(|cx| InputState::new(window, cx).default_value(seed(initial_param.filters.as_ref())));
    let columns_field = ColumnsField::new(initial_param.columns.as_ref(), window, cx);
    let screener_type = Rc::new(Cell::new(initial_param.screener_type));
    let on_confirm = Rc::new(on_confirm);

    // Every toggle reports the full selection; park it for the commit.
    let columns_state = columns_field.state.clone();
    let columns_selected = columns_field.selected.clone();
    cx.subscribe(
        &columns_state,
        move |_, event: &ComboboxEvent<SearchableVec<FinvizColumn>>, _| {
            if let ComboboxEvent::Change(values) = event {
                *columns_selected.borrow_mut() = values.clone();
            }
        },
    )
    .detach();

    window.open_dialog(cx, move |dialog, _window, cx| {
        let stocks_checked = screener_type.get() == ScreenerType::Stocks;

        let pick_type = {
            let cell = screener_type.clone();
            move |kind: ScreenerType| {
                let cell = cell.clone();
                move |_: &bool, window: &mut Window, _: &mut App| {
                    cell.set(kind);
                    window.refresh();
                }
            }
        };

        let confirm = {
            let on_confirm = on_confirm.clone();
            let name_input = name_input.clone();
            let signal_input = signal_input.clone();
            let order_input = order_input.clone();
            let filter_input = filter_input.clone();
            let columns_field = columns_field.clone();
            let screener_type = screener_type.clone();
            move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut App| {
                let name = name_input.read(cx).value().trim().to_string();
                if name.is_empty() {
                    return;
                }
                let order = order_input.read(cx).value().trim().to_string();
                let order = if order.is_empty() {
                    "-volume".to_string()
                } else {
                    order
                };
                let signal = non_empty(&signal_input.read(cx).value());
                let parameter = non_empty(&filter_input.read(cx).value());
                let columns = columns_field.commit();
                let kind = screener_type.get();
                on_confirm(
                    (
                        name,
                        Parameter {
                            screener_type: kind,
                            order_by: order,
                            signal,
                            filters: parameter,
                            columns,
                        },
                    ),
                    cx,
                );
                window.close_dialog(cx);
            }
        };

        // One labelled input with a muted, single-line description under it
        // (text_xs like the Field component) — five stacked fields must fit
        // the dialog's fixed height.
        let field = |label: &str, input: &gpui_kit::Entity<InputState>, desc: Option<&str>| {
            let mut column = v_flex()
                .gap_1()
                .child(Label::new(tr!(label)))
                .child(Input::new(input));
            if let Some(desc) = desc {
                column = column.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(tr!(desc)),
                );
            }
            column
        };

        let title = if initial_name.is_empty() {
            tr!("app.settings.finviz_add_title")
        } else {
            tr!("app.settings.finviz_edit_title")
        };
        dialog
            .title(title)
            .child(
                v_flex()
                    .gap_3()
                    .child(field("app.settings.finviz_preset_name", &name_input, None))
                    .child(
                        v_flex()
                            .gap_1()
                            .child(Label::new(tr!("app.settings.finviz_preset_type")))
                            .child(
                                h_flex()
                                    .gap_4()
                                    .child(
                                        Radio::new("finviz-type-stocks")
                                            .label(tr!("app.settings.finviz_stock"))
                                            .checked(stocks_checked)
                                            .on_click(pick_type(ScreenerType::Stocks)),
                                    )
                                    .child(
                                        Radio::new("finviz-type-etf")
                                            .label(tr!("app.settings.finviz_etf"))
                                            .checked(!stocks_checked)
                                            .on_click(pick_type(ScreenerType::ETF)),
                                    ),
                            )
                            .child(
                                div()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(tr!("app.settings.finviz_type_desc")),
                            ),
                    )
                    .child(field(
                        "app.settings.finviz_signal",
                        &signal_input,
                        Some("app.settings.finviz_signal_desc"),
                    ))
                    .child(field(
                        "app.settings.finviz_order",
                        &order_input,
                        Some("app.settings.finviz_order_desc"),
                    ))
                    .child(field(
                        "app.settings.finviz_filter",
                        &filter_input,
                        Some("app.settings.finviz_filter_desc"),
                    ))
                    .child(
                        v_flex()
                            .gap_1()
                            .child(Label::new(tr!("app.settings.finviz_columns")))
                            .child(
                                columns_field
                                    .element(tr!("app.settings.finviz_columns_placeholder")),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(tr!("app.settings.finviz_columns_desc")),
                            ),
                    ),
            )
            .footer(dialog_footer("finviz", confirm))
    });
}
