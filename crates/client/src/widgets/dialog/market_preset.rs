//! The Market Screener preset form: a title, the Finviz preset whose export
//! builds the symbol pool, the ask/bid exchange codes (one table each) and
//! the minimum size both sides share. Opens blank ("new") or seeded
//! ("edit") when an `initial` draft is passed. The exchange codes are picked
//! from a searchable multi-select combobox, the same shape as the Finviz
//! preset dialog's columns field.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::{
    App, AppContext as _, IntoElement as _, ParentElement as _, SharedString, Styled as _, Window,
    base::{h_flex, v_flex},
    component::{
        ActiveTheme as _, Sizable as _, WindowExt,
        combobox::{Combobox, ComboboxEvent, ComboboxState},
        input::{Input, InputState},
        label::Label,
        searchable_list::{SearchableListItem, SearchableVec},
    },
    div, px,
};

use crate::tr;
use crate::utils::exchanges::EXCHANGE_CODES;
use crate::utils::state::AppState;
use crate::widgets::dialog::dialog_footer;

/// The values the form commits.
#[derive(Clone)]
pub struct MarketPresetDraft {
    pub name: String,
    pub exchanges: Vec<String>,
    pub min_size: i64,
    pub finviz_preset: String,
}

/// One entry of the exchange table, as a combobox item: the value is the
/// single-letter code (what Alpaca's `ax`/`bx` carry), the label reads
/// "Venue (code)".
#[derive(Clone)]
struct ExchangeOption {
    code: String,
    name: SharedString,
}

impl SearchableListItem for ExchangeOption {
    type Value = String;

    fn title(&self) -> SharedString {
        SharedString::from(format!("{} ({})", self.name, self.code))
    }

    fn value(&self) -> &Self::Value {
        &self.code
    }

    fn matches(&self, query: &str) -> bool {
        let query = query.to_lowercase();
        self.name.to_lowercase().contains(&query) || self.code.to_lowercase().contains(&query)
    }
}

/// The exchanges field: a searchable multi-select over [`EXCHANGE_CODES`].
/// `selected` is kept up to date on every toggle (selection order); `unknown`
/// holds stored codes the table has no item for, so an edited preset never
/// silently loses them.
#[derive(Clone)]
struct ExchangesField {
    state: gpui_kit::Entity<ComboboxState<SearchableVec<ExchangeOption>>>,
    selected: Rc<RefCell<Vec<String>>>,
    unknown: Rc<RefCell<Vec<String>>>,
}

impl ExchangesField {
    fn new(initial: &[String], window: &mut Window, cx: &mut App) -> Self {
        let items: Vec<ExchangeOption> = EXCHANGE_CODES
            .iter()
            .map(|(code, name)| ExchangeOption {
                code: (*code).to_string(),
                name: SharedString::from(*name),
            })
            .collect();
        let state = cx.new(|cx| {
            ComboboxState::new(SearchableVec::new(items), vec![], window, cx)
                .searchable(true)
                .multiple(true)
        });

        // Seed from the stored list: known codes become selections, the rest
        // ride along so editing + confirming never loses them.
        let selected = Rc::new(RefCell::new(Vec::new()));
        let unknown = Rc::new(RefCell::new(Vec::new()));
        let mut known: Vec<String> = Vec::new();
        for code in initial {
            if EXCHANGE_CODES
                .iter()
                .any(|(known_code, _)| known_code == code)
            {
                known.push(code.clone());
            } else {
                unknown.borrow_mut().push(code.clone());
            }
        }
        if !known.is_empty() {
            state.update(cx, |state, cx| {
                state.set_selected_values(&known, window, cx);
            });
        }
        *selected.borrow_mut() = known;

        // Every toggle reports the full selection; park it for the commit.
        // Without this the chips render but the parking cell stays empty and
        // Confirm silently bails on an "empty" selection.
        let parked = selected.clone();
        cx.subscribe(
            &state,
            move |_, event: &ComboboxEvent<SearchableVec<ExchangeOption>>, _| {
                if let ComboboxEvent::Change(values) = event {
                    *parked.borrow_mut() = values.clone();
                }
            },
        )
        .detach();

        Self {
            state,
            selected,
            unknown,
        }
    }

    /// The committed value: the selected codes in selection order, with
    /// unknown stored codes appended.
    fn commit(&self) -> Vec<String> {
        let mut codes = self.selected.borrow().clone();
        codes.extend(self.unknown.borrow().iter().cloned());
        codes
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
                const MAX_CHIPS: usize = 4;
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

/// A single-select combobox over plain names, parking its selection into a
/// shared cell on every change (the same park-and-commit shape the multi
/// select uses).
struct PickOne {
    state: gpui_kit::Entity<ComboboxState<SearchableVec<String>>>,
    selected: Rc<RefCell<Option<String>>>,
}

impl PickOne {
    fn new(names: Vec<String>, initial: Option<String>, window: &mut Window, cx: &mut App) -> Self {
        let state = cx.new(|cx| {
            ComboboxState::new(SearchableVec::new(names.clone()), vec![], window, cx)
                .searchable(false)
        });
        let selected = Rc::new(RefCell::new(initial.filter(|name| names.contains(name))));
        if let Some(name) = selected.borrow().clone() {
            state.update(cx, |state, cx| {
                state.set_selected_values(&[name], window, cx)
            });
        }
        let parked = selected.clone();
        cx.subscribe(
            &state,
            move |_, event: &ComboboxEvent<SearchableVec<String>>, _| {
                if let ComboboxEvent::Change(values) = event {
                    *parked.borrow_mut() = values.last().cloned();
                }
            },
        )
        .detach();
        Self { state, selected }
    }

    fn element(&self, placeholder: SharedString) -> impl gpui_kit::IntoElement {
        Combobox::new(&self.state)
            .small()
            .w_full()
            .cleanable(false)
            .placeholder(placeholder)
    }
}

/// Open the form. Pass `Some(draft)` to edit an existing preset: every field
/// comes prefilled and the title reads "edit". `on_confirm` receives the
/// committed draft and persists it however the caller wants.
pub fn open_market_preset_dialog(
    window: &mut Window,
    cx: &mut App,
    initial: Option<MarketPresetDraft>,
    on_confirm: impl Fn(MarketPresetDraft, &mut App) + 'static,
) {
    let initial = initial.unwrap_or(MarketPresetDraft {
        name: String::new(),
        exchanges: Vec::new(),
        min_size: 0,
        finviz_preset: String::new(),
    });
    let MarketPresetDraft {
        name: initial_name,
        exchanges: initial_exchanges,
        min_size: initial_min_size,
        finviz_preset: initial_finviz,
    } = initial;

    let name_input = cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(tr!("app.market_screener.preset_title"))
            .default_value(initial_name.clone())
    });
    let size_input = cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder("0")
            .default_value(if initial_min_size > 0 {
                initial_min_size.to_string()
            } else {
                String::new()
            })
    });
    let exchanges_field = ExchangesField::new(&initial_exchanges, window, cx);

    let finviz_names: Vec<String> = cx
        .global::<AppState>()
        .settings
        .finviz
        .screener_parameter
        .keys()
        .cloned()
        .collect();
    let finviz_picker = PickOne::new(finviz_names, Some(initial_finviz), window, cx);

    let on_confirm = Rc::new(on_confirm);

    window.open_dialog(cx, move |dialog, _window, cx| {
        let confirm = {
            let on_confirm = on_confirm.clone();
            let name_input = name_input.clone();
            let size_input = size_input.clone();
            let exchanges_field = exchanges_field.clone();
            let finviz_selected = finviz_picker.selected.clone();
            move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut App| {
                let name = name_input.read(cx).value().trim().to_string();
                let exchanges = exchanges_field.commit();
                let Some(finviz_preset) = finviz_selected.borrow().clone() else {
                    return;
                };
                if name.is_empty() || exchanges.is_empty() {
                    return;
                }
                let min_size = size_input
                    .read(cx)
                    .value()
                    .trim()
                    .parse::<i64>()
                    .unwrap_or(0)
                    .max(0);
                on_confirm(
                    MarketPresetDraft {
                        name,
                        exchanges,
                        min_size,
                        finviz_preset,
                    },
                    cx,
                );
                window.close_dialog(cx);
            }
        };

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
            tr!("app.market_screener.add_preset")
        } else {
            tr!("app.market_screener.edit_preset")
        };

        dialog
            .title(title)
            .width(px(480.))
            .child(
                v_flex()
                    .gap_3()
                    .child(field(
                        "app.market_screener.preset_title",
                        &name_input,
                        Some("app.market_screener.preset_title_hint"),
                    ))
                    .child(
                        v_flex()
                            .gap_1()
                            .child(Label::new(tr!("app.market_screener.finviz_preset")))
                            .child(finviz_picker.element(tr!("app.market_screener.pick_finviz"))),
                    )
                    .child(
                        v_flex()
                            .gap_1()
                            .child(Label::new(tr!("app.market_screener.exchanges")))
                            .child(
                                exchanges_field
                                    .element(tr!("app.market_screener.exchanges_placeholder")),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(tr!("app.market_screener.exchanges_hint")),
                            ),
                    )
                    .child(field(
                        "app.market_screener.min_size",
                        &size_input,
                        Some("app.market_screener.min_size_hint"),
                    )),
            )
            .footer(dialog_footer("market-preset", confirm))
    });
}
