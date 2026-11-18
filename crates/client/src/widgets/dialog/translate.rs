//! Translate dialog: shows `text` and its translation between the two
//! persisted languages, with dropdowns to change them and a refresh button.

use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, ClickEvent, Entity, InteractiveElement as _,
    IntoElement, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled,
    WeakEntity, Window,
    assets::IconName,
    base::{h_flex, v_flex},
    component::{
        ActiveTheme as _, Sizable as _, WindowExt,
        button::{Button, ButtonVariants as _},
        menu::{DropdownMenu as _, PopupMenu, PopupMenuItem},
        spinner::Spinner,
    },
    div, px,
};

use relay_protocol::{RpcRequestPayload, RpcResponsePayload};

use crate::tr;
use crate::utils::settings::{self, LANGUAGES, language_label};
use crate::utils::state::AppState;

/// Dialog-local state: the selected languages plus the latest translation.
/// Created per dialog opening; closing the dialog drops it.
struct TranslateState {
    from: String,
    to: String,
    /// `None` while a request is in flight.
    result: Option<Result<SharedString, SharedString>>,
    /// Monotonic counter bumped on every fetch and language change; an
    /// in-flight response whose generation no longer matches is discarded
    /// (double-click refresh, language switch during a request).
    generation: u64,
}

/// Open the translate dialog for `text`. Source/target default to the
/// persisted `settings.translate` languages; the footer offers both
/// dropdowns (persisted on change) and a refresh button that re-sends the
/// request. The first request fires immediately.
pub fn open_translate_dialog(window: &mut Window, cx: &mut App, text: impl Into<SharedString>) {
    let text: SharedString = text.into();
    let (from, to) = {
        let translate = &cx.global::<AppState>().settings.translate;
        (translate.from_lang.clone(), translate.to_lang.clone())
    };
    let state = cx.new(|_| TranslateState {
        from,
        to,
        result: None,
        generation: 0,
    });
    let window_handle = window.window_handle();

    fetch(&state.downgrade(), text.clone(), window_handle, cx);

    window.open_dialog(cx, move |dialog, _window, cx| {
        let snapshot = state.read(cx);
        let muted = cx.theme().muted_foreground;
        let danger = cx.theme().danger;

        let result_area = match &snapshot.result {
            None => div().child(Spinner::new().xsmall()).into_any_element(),
            Some(Ok(translation)) => div()
                .id("translate-result")
                .max_h(px(200.))
                .overflow_y_scroll()
                .child(translation.clone())
                .into_any_element(),
            Some(Err(detail)) => v_flex()
                .gap_1()
                .child(div().text_color(danger).child(tr!("app.translate.failed")))
                .child(div().text_xs().text_color(muted).child(detail.clone()))
                .into_any_element(),
        };

        let body = v_flex()
            .gap_2()
            .text_sm()
            .child(div().text_color(muted).child(tr!("app.translate.source")))
            .child(
                div()
                    .id("translate-source")
                    .max_h(px(140.))
                    .overflow_y_scroll()
                    .child(text.clone()),
            )
            .child(
                div()
                    .pt_1()
                    .text_color(muted)
                    .child(tr!("app.translate.result")),
            )
            .child(result_area);

        dialog.title(tr!("app.translate.title")).child(body).footer(
            h_flex()
                .gap_2()
                .child(lang_dropdown("translate-from", &state, Which::From, cx))
                .child(div().text_color(muted).child(SharedString::from("→")))
                .child(lang_dropdown("translate-to", &state, Which::To, cx))
                .child(div().flex_1())
                .child({
                    let state = state.clone();
                    let text = text.clone();
                    let window_handle = window_handle;
                    Button::new("translate-refresh")
                        .ghost()
                        .small()
                        .icon(IconName::RefreshCw)
                        .tooltip(tr!("app.settings.refresh"))
                        .on_click(move |_: &ClickEvent, _, cx: &mut App| {
                            fetch(&state.downgrade(), text.clone(), window_handle, cx);
                        })
                }),
        )
    });
}

#[derive(Clone, Copy, PartialEq)]
enum Which {
    From,
    To,
}

/// One language dropdown bound to the dialog state; picking a language
/// persists it into `settings.translate` but does not re-send the request.
fn lang_dropdown(
    id: &'static str,
    state: &Entity<TranslateState>,
    which: Which,
    cx: &App,
) -> impl IntoElement {
    let code = match which {
        Which::From => &state.read(cx).from,
        Which::To => &state.read(cx).to,
    };
    let selected = code.clone();
    let weak = state.downgrade();

    Button::new(id)
        .outline()
        .small()
        .label(language_label(code))
        .dropdown_menu(move |menu, _window, _cx| {
            let mut menu: PopupMenu = menu;
            for (value, label) in LANGUAGES {
                let weak = weak.clone();
                let value = SharedString::from(value);
                menu = menu.item(
                    PopupMenuItem::new(label)
                        .checked(selected.as_str() == value.as_ref())
                        .on_click(move |_: &ClickEvent, _, cx: &mut App| {
                            let value = value.clone();
                            let _ = weak.update(cx, |state, cx| {
                                match which {
                                    Which::From => state.from = value.to_string(),
                                    Which::To => state.to = value.to_string(),
                                }
                                // In-flight responses for the old language
                                // pair are no longer interesting.
                                state.generation += 1;
                                cx.notify();
                            });
                            settings::update_settings(cx, |s| match which {
                                Which::From => s.translate.from_lang = value.to_string(),
                                Which::To => s.translate.to_lang = value.to_string(),
                            });
                        }),
                );
            }
            menu
        })
}

/// Send one translate request for `text` with the state's current languages
/// and write the outcome back into the dialog state. The dialog re-renders
/// from the window's frame — `TranslateState` has no observers, so a bare
/// `cx.notify()` never becomes visible — hence the forced redraw through
/// `window_handle` once the response lands.
fn fetch(
    state: &WeakEntity<TranslateState>,
    text: SharedString,
    window_handle: AnyWindowHandle,
    cx: &mut App,
) {
    let Ok((from, to, generation)) = state.update(cx, |state, cx| {
        state.generation += 1;
        // Back to the spinner while this request is in flight (the old
        // result would otherwise sit there unchanged).
        state.result = None;
        cx.notify();
        (state.from.clone(), state.to.clone(), state.generation)
    }) else {
        return;
    };
    // The spinner must show immediately, before any interaction happens:
    // the state entity has no observers, so force a frame.
    let _ = window_handle.update(cx, |_, window, _| window.refresh());

    let state = state.clone();
    cx.spawn(async move |cx| {
        let relay = cx.update(|cx| AppState::relay(cx));
        let result = match relay {
            Ok(relay) => {
                match relay
                    .rpc_checked(RpcRequestPayload::Translate {
                        from: from.clone(),
                        to: to.clone(),
                        text: text.to_string(),
                    })
                    .await
                {
                    Ok(RpcResponsePayload::Translate(text)) => Ok(text),
                    Ok(other) => Err(format!("unexpected rpc response: {other:?}")),
                    Err(err) => Err(format!("{err:#}")),
                }
            }
            Err(err) => Err(format!("{err:#}")),
        };
        let result = result
            .map(SharedString::from)
            .map_err(SharedString::from);

        let _ = state.update(cx, |state, cx| {
            if state.generation != generation {
                // Superseded by another refresh or a language switch while
                // this request was in flight.
                return;
            }
            state.result = Some(result);
            cx.notify();
        });
        let _ = window_handle.update(cx, |_, window, _| window.refresh());
    })
    .detach();
}
