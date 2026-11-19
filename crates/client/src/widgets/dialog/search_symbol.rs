//! Symbol search dialog: debounced search-as-you-type over the data gateway,
//! keyboard navigation, Enter or click to pick.

use std::cell::Cell;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, ClickEvent, Context, Entity, Focusable as _,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled, Window,
    assets::IconName,
    base::{h_flex, v_flex},
    component::{
        ActiveTheme as _, Icon, Sizable as _, WindowExt,
        input::{
            Input, InputEvent, InputGroup, InputGroupAddon, InputGroupAddonAlignment,
            InputGroupButton, InputState,
        },
        spinner::Spinner,
    },
    div,
    prelude::FluentBuilder as _,
    px,
};

use protocol::{RpcRequestPayload, RpcResponsePayload, SearchItem};

use crate::tr;
use crate::utils::message;
use crate::utils::state::AppState;

/// How long to wait after the last keystroke before sending the request.
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(300);

/// Callback invoked with the confirmed search result.
pub type OnPick = Arc<dyn Fn(&SearchItem, &mut App) + 'static>;

/// Dialog-local state: the input, the latest results and the keyboard
/// selection. Created per dialog opening; closing the dialog drops it.
pub struct SearchSymbolState {
    input: Entity<InputState>,
    /// The dialog's window, for forced redraws: the content builder re-runs
    /// per window frame, and this state entity has no observers, so a bare
    /// `cx.notify()` after an async response never becomes visible on its
    /// own.
    window_handle: AnyWindowHandle,
    /// Search callback invoked with the confirmed item.
    on_pick: OnPick,
    results: Vec<SearchItem>,
    /// Keyboard selection into `results`.
    selected: Option<usize>,
    searching: bool,
    /// Debounce generation; a finished request only applies when the
    /// generation still matches (newer keystrokes win).
    generation: usize,
}

impl SearchSymbolState {
    fn schedule_search(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let request_gen = self.generation;

        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_DEBOUNCE).await;

            let Ok((keyword, current_gen, lang)) = this.update(cx, |state, cx| {
                let keyword = state.input.read(cx).value().to_string();
                // futunn 的语言参数就是 locale 线值("zh-cn"/"en-us")。
                let lang = rust_i18n::locale().to_string();
                (keyword, state.generation, lang)
            }) else {
                return;
            };

            // A newer keystroke superseded this request.
            if request_gen != current_gen {
                return;
            }

            if keyword.trim().is_empty() {
                let _ = this.update(cx, |state, cx| {
                    state.results.clear();
                    state.selected = None;
                    state.searching = false;
                    cx.notify();
                });
                return;
            }

            let _ = this.update(cx, |state, cx| {
                state.searching = true;
                cx.notify();
            });
            let Ok(relay) = cx.update(|cx| AppState::relay(cx)) else {
                let _ = this.update(cx, |state, cx| {
                    state.searching = false;
                    cx.notify();
                });
                return;
            };
            let result = relay
                .rpc_checked(RpcRequestPayload::Search {
                    keyword: keyword.clone(),
                    lang,
                })
                .await;

            let Ok(Some(window_handle)) = this.update(cx, |state, cx| {
                if state.generation != request_gen {
                    return None;
                }
                state.searching = false;
                match result {
                    Ok(RpcResponsePayload::Search(items)) => {
                        state.selected = (!items.is_empty()).then_some(0);
                        state.results = items;
                    }
                    Ok(other) => {
                        state.results.clear();
                        state.selected = None;
                        message::error(
                            tr!("app.notify.search_failed"),
                            anyhow::anyhow!("unexpected rpc response: {other:?}"),
                            cx,
                        );
                    }
                    Err(err) => {
                        state.results.clear();
                        state.selected = None;
                        message::error(tr!("app.notify.search_failed"), err, cx);
                    }
                }
                cx.notify();
                Some(state.window_handle)
            }) else {
                return;
            };
            let _ = window_handle.update(cx, |_, window, _| window.refresh());
        })
        .detach();
    }

    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.results.is_empty() {
            return;
        }
        let len = self.results.len();
        let next = match self.selected {
            Some(ix) => (ix as isize + delta).clamp(0, len as isize - 1) as usize,
            None => 0,
        };
        self.selected = Some(next);
        cx.notify();
    }

    fn picked(&self) -> Option<SearchItem> {
        let index = self.selected?;
        self.results.get(index).cloned()
    }
}

/// Open the symbol search dialog. `on_pick` runs with the confirmed item;
/// the dialog closes right after. The search language is the current UI
/// language.
pub fn open_search_symbol_dialog(
    window: &mut Window,
    cx: &mut App,
    on_pick: impl Fn(&SearchItem, &mut App) + 'static,
) {
    let input = cx.new(|cx| InputState::new(window, cx).placeholder(tr!("app.search.placeholder")));
    let window_handle = window.window_handle();
    let state = cx.new(|_| SearchSymbolState {
        input: input.clone(),
        window_handle,
        on_pick: Arc::new(on_pick),
        results: vec![],
        selected: None,
        searching: false,
        generation: 0,
    });

    let window_handle = window.window_handle();

    // Re-search on every edit; Enter confirms the selected result via the
    // PressEnter subscription (the only pick path — a key-down handler would
    // fire in addition, because the input's Enter action propagates before
    // the queued PressEnter effect runs, picking twice).
    cx.subscribe(&input, {
        let state = state.clone();
        move |_, event: &InputEvent, cx| match event {
            InputEvent::Change => state.update(cx, |state, cx| state.schedule_search(cx)),
            InputEvent::PressEnter { .. } => {
                let picked = state.read(cx).picked();
                if let Some(item) = picked {
                    let on_pick = state.read(cx).on_pick.clone();
                    on_pick(&item, cx);
                    let _ = window_handle.update(cx, |_, window, cx| window.close_dialog(cx));
                }
            }
            _ => {}
        }
    })
    .detach();

    // Focus the input once, when the dialog layer is first built; later
    // builder re-runs (e.g. result updates) must not steal focus back.
    let initial_focus = Cell::new(true);

    window.open_dialog(cx, move |dialog, window, cx| {
        if initial_focus.replace(false) {
            input.read(cx).focus_handle(cx).focus(window, cx);
        }

        let snapshot = state.read(cx);
        let muted = cx.theme().muted_foreground;

        let list_area: gpui_kit::AnyElement = if snapshot.searching {
            div()
                .flex()
                .justify_center()
                .py_3()
                .child(Spinner::new().small())
                .into_any_element()
        } else if snapshot.results.is_empty() {
            div()
                .flex()
                .justify_center()
                .py_3()
                .text_color(muted)
                .child(tr!("app.search.no_results"))
                .into_any_element()
        } else {
            let results = snapshot.results.clone();
            let selected = snapshot.selected;
            div()
                .id("search-results")
                .max_h(px(280.))
                .overflow_y_scroll()
                .flex_col()
                .gap_0p5()
                .children(results.into_iter().enumerate().map(|(ix, item)| {
                    let state = state.clone();
                    let selected = selected == Some(ix);
                    h_flex()
                        .id(("search-result", ix))
                        .justify_between()
                        .gap_3()
                        .px_2()
                        .py_1()
                        .rounded_sm()
                        .when(selected, |row| row.bg(cx.theme().secondary_active))
                        .on_click({
                            let item = item.clone();
                            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                let on_pick = state.read(cx).on_pick.clone();
                                on_pick(&item, cx);
                                window.close_dialog(cx);
                            }
                        })
                        .child(div().child(SharedString::from(item.symbol.clone())))
                        .child(
                            div()
                                .text_color(muted)
                                .child(SharedString::from(item.name.clone())),
                        )
                }))
                .into_any_element()
        };

        let body = v_flex()
            .w(px(440.))
            .gap_2()
            .key_context("SearchSymbol")
            .on_key_down({
                let state = state.clone();
                move |event: &KeyDownEvent, _: &mut Window, cx: &mut App| match event
                    .keystroke
                    .key
                    .as_ref()
                {
                    "up" => {
                        state.update(cx, |state, cx| state.move_selection(-1, cx));
                        cx.stop_propagation();
                    }
                    "down" => {
                        state.update(cx, |state, cx| state.move_selection(1, cx));
                        cx.stop_propagation();
                    }
                    _ => {}
                }
            })
            .child({
                let input = input.clone();
                let state = state.clone();
                let searching = snapshot.searching;
                InputGroup::new("search-input")
                    .input(Input::new(&input))
                    .addon(
                        InputGroupAddon::new("search-leading")
                            .align(InputGroupAddonAlignment::InlineStart)
                            .child(Icon::new(IconName::Search).small()),
                    )
                    .addon(
                        InputGroupAddon::new("search-trailing")
                            .align(InputGroupAddonAlignment::InlineEnd)
                            .child(
                                InputGroupButton::new("search-refresh")
                                    .icon(IconName::RefreshCw)
                                    .tooltip(tr!("app.settings.refresh"))
                                    .loading(searching)
                                    .on_click(
                                        move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                            // Re-run the search for the current
                                            // keyword and keep typing focus.
                                            state.update(cx, |state, cx| state.schedule_search(cx));
                                            input.read(cx).focus_handle(cx).focus(window, cx);
                                        },
                                    ),
                            ),
                    )
            })
            .child(list_area)
            .child(
                h_flex()
                    .justify_center()
                    .gap_3()
                    .text_xs()
                    .text_color(muted)
                    .child(SharedString::from(format!(
                        "↑↓ {}",
                        tr!("app.search.hint_navigate")
                    )))
                    .child(SharedString::from(format!(
                        "⏎ {}",
                        tr!("app.search.hint_pick")
                    )))
                    .child(SharedString::from(format!(
                        "Esc {}",
                        tr!("app.search.hint_close")
                    ))),
            );

        dialog
            .width(px(480.))
            .title(tr!("app.symbol.search_title"))
            .child(body)
    });
}
