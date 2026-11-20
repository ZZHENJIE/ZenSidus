//! Server auth-code dialog: a single input for the current TOTP code.
//! The code is never persisted — it is typed fresh on every connect (30s
//! validity), so this dialog is the only path from a Connect button to the hub.

use std::cell::Cell;
use std::sync::Arc;

use gpui_kit::{
    App, AnyWindowHandle, AppContext as _, Focusable as _, ParentElement as _, Styled as _, Window,
    base::v_flex,
    component::{
        WindowExt,
        input::{Input, InputEvent, InputState},
    },
    px,
};

use crate::tr;
use crate::widgets::dialog::dialog_footer;

/// Callback invoked with the confirmed code (already trimmed, non-empty).
pub type OnCode = Arc<dyn Fn(&str, &mut App) + 'static>;

/// Open the auth-code dialog. `on_confirm` runs with the entered code, then the
/// dialog closes. Empty input is a no-op (Confirm does nothing, Enter is
/// ignored) — a trimmed-empty code is never handed upstream.
pub fn open_auth_code_dialog(
    window: &mut Window,
    cx: &mut App,
    on_confirm: impl Fn(&str, &mut App) + 'static,
) {
    let input = cx.new(|cx| InputState::new(window, cx).placeholder(tr!("app.auth_code.placeholder")));
    let window_handle: AnyWindowHandle = window.window_handle();
    let on_confirm: OnCode = Arc::new(on_confirm);

    // Enter confirms (the input's PressEnter is the only reliable path — a
    // key-down handler would fire in addition and confirm twice).
    cx.subscribe(
        &input,
        {
            let input = input.clone();
            let on_confirm = on_confirm.clone();
            move |_, event: &InputEvent, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    let token = input.read(cx).value().trim().to_string();
                    if token.is_empty() {
                        return;
                    }
                    on_confirm(&token, cx);
                    let _ = window_handle.update(cx, |_, window, cx| window.close_dialog(cx));
                }
            }
        },
    )
    .detach();

    // Focus the input once, when the dialog layer is first built; later
    // builder re-runs must not steal focus back.
    let initial_focus = Cell::new(true);

    window.open_dialog(cx, move |dialog, window, cx| {
        if initial_focus.replace(false) {
            input.read(cx).focus_handle(cx).focus(window, cx);
        }

        let on_confirm = on_confirm.clone();
        let input_view = Input::new(&input);
        let input = input.clone();
        dialog
            .width(px(360.))
            .title(tr!("app.auth_code.title"))
            .child(
                v_flex()
                    .gap_2()
                    .child(input_view)
                    .child(dialog_footer("token", move |_, window, cx| {
                        let token = input.read(cx).value().trim().to_string();
                        if token.is_empty() {
                            return;
                        }
                        on_confirm(&token, cx);
                        window.close_dialog(cx);
                    })),
            )
    });
}
