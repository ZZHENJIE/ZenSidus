//! The link-parameters form for [`super::ETFLinkCalcWindow`]: both symbols,
//! the leverage multiple, the direction, the day's opening prices and the
//! two offsets. Input entities are created before `open_dialog` (the
//! builder re-runs on every update); Enter in any field commits, same as
//! the footer button.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::{
    App, AppContext as _, ClickEvent, Context, Entity, Focusable as _, ParentElement as _,
    Styled as _, Window,
    base::{h_flex, v_flex},
    component::{
        ActiveTheme as _, WindowExt,
        input::{Input, InputEvent, InputState, NumberInput},
        label::Label,
        radio::Radio,
    },
    div, px,
};

use super::{LinkParams, number_input, parse_f64};
use crate::tr;
use crate::widgets::dialog::dialog_footer;

/// The dialog's commit, shared by the footer button and Enter in any field.
type Commit = Rc<dyn Fn(&mut Window, &mut App)>;

/// Open the form seeded with `current`. `on_confirm` receives the edited
/// link; the dialog closes right after.
pub(super) fn open_params_dialog(
    window: &mut Window,
    cx: &mut App,
    current: LinkParams,
    on_confirm: impl Fn(LinkParams, &mut App) + 'static,
) {
    let zero = || "0".to_string();
    // Symbol fields seed the current value; the example symbols show in
    // their placeholders while empty.
    let index_symbol = cx.new(|cx| symbol_input(window, cx, &current.index_symbol, "SPY"));
    let etf_symbol = cx.new(|cx| symbol_input(window, cx, &current.etf_symbol, "SPDN"));
    let multiple = cx.new(|cx| {
        number_input(
            window,
            cx,
            f64::from(current.multiple),
            zero(),
            Some(1.0),
            1.0,
        )
        .max(255.0)
    });
    let etf_open = cx.new(|cx| number_input(window, cx, current.etf_open, zero(), Some(0.0), 0.01));
    let index_open =
        cx.new(|cx| number_input(window, cx, current.index_open, zero(), Some(0.0), 1.0));
    // The offsets may be negative, so no floor.
    let index_offset =
        cx.new(|cx| number_input(window, cx, current.index_offset, zero(), None, 1.0));
    let long_short_offset =
        cx.new(|cx| number_input(window, cx, current.long_short_offset, zero(), None, 1.0));
    let is_short = Rc::new(Cell::new(current.is_short));
    let window_handle = window.window_handle();

    // One commit path for the footer button and Enter in any field.
    let commit: Commit = Rc::new({
        let index_symbol = index_symbol.clone();
        let etf_symbol = etf_symbol.clone();
        let multiple = multiple.clone();
        let etf_open = etf_open.clone();
        let index_open = index_open.clone();
        let index_offset = index_offset.clone();
        let long_short_offset = long_short_offset.clone();
        let is_short = is_short.clone();
        move |window: &mut Window, cx: &mut App| {
            let params = LinkParams {
                index_symbol: symbol_value(&index_symbol.read(cx).value()),
                etf_symbol: symbol_value(&etf_symbol.read(cx).value()),
                multiple: parse_f64(&multiple.read(cx).value())
                    .round()
                    .clamp(1.0, 255.0) as u8,
                is_short: is_short.get(),
                etf_open: parse_f64(&etf_open.read(cx).value()),
                index_open: parse_f64(&index_open.read(cx).value()),
                index_offset: parse_f64(&index_offset.read(cx).value()),
                long_short_offset: parse_f64(&long_short_offset.read(cx).value()),
            };
            on_confirm(params, cx);
            window.close_dialog(cx);
        }
    });

    for input in [
        &index_symbol,
        &etf_symbol,
        &multiple,
        &etf_open,
        &index_open,
        &index_offset,
        &long_short_offset,
    ] {
        let commit = commit.clone();
        cx.subscribe(input, move |_, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                let commit = commit.clone();
                let _ = window_handle.update(cx, move |_, window, cx| commit(window, cx));
            }
        })
        .detach();
    }

    // Focus the first field once, when the dialog layer is first built;
    // later builder re-runs must not steal focus back.
    let initial_focus = Cell::new(true);

    window.open_dialog(cx, move |dialog, window, cx| {
        if initial_focus.replace(false) {
            index_symbol.read(cx).focus_handle(cx).focus(window, cx);
        }

        let muted = cx.theme().muted_foreground;
        // One half of a two-column row: label over input, equal widths.
        let text_field = |label: &'static str, input: &Entity<InputState>| {
            v_flex()
                .flex_1()
                .gap_1()
                .child(Label::new(tr!(label)))
                .child(Input::new(input))
        };
        let number_field = |label: &'static str, input: &Entity<InputState>| {
            v_flex()
                .flex_1()
                .gap_1()
                .child(Label::new(tr!(label)))
                .child(NumberInput::new(input))
        };
        let pick = |short: bool| {
            let cell = is_short.clone();
            move |_: &bool, window: &mut Window, _: &mut App| {
                cell.set(short);
                window.refresh();
            }
        };
        let short = is_short.get();
        let confirm = {
            let commit = commit.clone();
            move |_: &ClickEvent, window: &mut Window, cx: &mut App| commit(window, cx)
        };

        dialog
            .title(tr!("app.etf_linkcalc.params_title"))
            .width(px(420.))
            .child(
                v_flex()
                    .gap_3()
                    // Row 1: the two symbols.
                    .child(
                        h_flex()
                            .gap_3()
                            .child(text_field("app.etf_linkcalc.index_symbol", &index_symbol))
                            .child(text_field("app.etf_linkcalc.etf_symbol", &etf_symbol)),
                    )
                    // Row 2: leverage and direction.
                    .child(
                        h_flex()
                            .gap_3()
                            .child(number_field("app.etf_linkcalc.multiple", &multiple))
                            .child(
                                v_flex()
                                    .flex_1()
                                    .gap_1()
                                    .child(Label::new(tr!("app.etf_linkcalc.direction")))
                                    .child(
                                        h_flex()
                                            .h_7()
                                            .items_center()
                                            .gap_4()
                                            .child(
                                                Radio::new("etf-linkcalc-long")
                                                    .label(tr!("app.etf_linkcalc.long"))
                                                    .checked(!short)
                                                    .on_click(pick(false)),
                                            )
                                            .child(
                                                Radio::new("etf-linkcalc-short")
                                                    .label(tr!("app.etf_linkcalc.short"))
                                                    .checked(short)
                                                    .on_click(pick(true)),
                                            ),
                                    ),
                            ),
                    )
                    // Row 3: the day's opening prices (they go stale daily).
                    .child(
                        h_flex()
                            .gap_3()
                            .child(number_field("app.etf_linkcalc.index_open", &index_open))
                            .child(number_field("app.etf_linkcalc.etf_open", &etf_open)),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child(tr!("app.etf_linkcalc.opens_desc")),
                    )
                    // Row 4: the two offsets (both signed).
                    .child(
                        h_flex()
                            .gap_3()
                            .child(number_field("app.etf_linkcalc.index_offset", &index_offset))
                            .child(number_field(
                                "app.etf_linkcalc.long_short_offset",
                                &long_short_offset,
                            )),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child(tr!("app.etf_linkcalc.offsets_desc")),
                    ),
            )
            .footer(dialog_footer("etf-linkcalc-params", confirm))
    });
}

/// A symbol as the params store it: trimmed and uppercased (stream symbols
/// and Alpaca's wire format are uppercase).
fn symbol_value(value: &str) -> String {
    value.trim().to_ascii_uppercase()
}

/// A symbol text field seeded with `value`; the example symbol shows in
/// the placeholder while the field is empty.
fn symbol_input(
    window: &mut Window,
    cx: &mut Context<InputState>,
    value: &str,
    example: &'static str,
) -> InputState {
    let state = InputState::new(window, cx).placeholder(example);
    if value.is_empty() {
        state
    } else {
        state.default_value(value.to_string())
    }
}
