//! Server auth-code dialog: a 6-cell OTP input for the current TOTP code.
//! The code is never persisted — it is typed fresh on every connect (30s
//! validity), so this dialog is the only path from a Connect button to the hub.

use std::cell::Cell;
use std::sync::Arc;

use gpui_kit::{
    App, AnyWindowHandle, AppContext as _, Focusable as _, ParentElement as _, Styled as _, Window,
    base::v_flex,
    component::{
        WindowExt,
        input::{OtpEvent, OtpInput, OtpState},
    },
    px,
};

use crate::tr;
use crate::widgets::dialog::dialog_footer;

/// Callback invoked with the confirmed code (6 digits).
pub type OnCode = Arc<dyn Fn(&str, &mut App) + 'static>;

const CODE_LENGTH: usize = 6;

/// Open the auth-code dialog. `on_confirm` runs with the entered code, then the
/// dialog closes. Filling the last cell confirms immediately (standard OTP
/// UX); the Confirm button only submits a complete code.
pub fn open_auth_code_dialog(
    window: &mut Window,
    cx: &mut App,
    on_confirm: impl Fn(&str, &mut App) + 'static,
) {
    let otp = cx.new(|cx| OtpState::new(CODE_LENGTH, window, cx));
    let window_handle: AnyWindowHandle = window.window_handle();
    let on_confirm: OnCode = Arc::new(on_confirm);

    // 填满最后一位即自动确认:此时值必为完整 6 位数字(OtpState 只收数字)。
    cx.subscribe(
        &otp,
        {
            let otp = otp.clone();
            let on_confirm = on_confirm.clone();
            move |_, event: &OtpEvent, cx| {
                if let OtpEvent::Complete = event {
                    let token = otp.read(cx).value().to_string();
                    on_confirm(&token, cx);
                    let _ = window_handle.update(cx, |_, window, cx| window.close_dialog(cx));
                }
            }
        },
    )
    .detach();

    // Focus the OTP cells once, when the dialog layer is first built; later
    // builder re-runs must not steal focus back.
    let initial_focus = Cell::new(true);

    window.open_dialog(cx, move |dialog, window, cx| {
        if initial_focus.replace(false) {
            otp.read(cx).focus_handle(cx).focus(window, cx);
        }

        let on_confirm = on_confirm.clone();
        let otp_view = OtpInput::new(&otp).groups(2);
        let otp = otp.clone();
        dialog
            .width(px(360.))
            .title(tr!("app.auth_code.title"))
            .child(
                v_flex()
                    .gap_3()
                    .items_center()
                    .child(otp_view)
                    .child(dialog_footer("token", move |_, window, cx| {
                        // 未填满(中途点到 Confirm)不提交。
                        if otp.read(cx).value().chars().count() != CODE_LENGTH {
                            return;
                        }
                        let token = otp.read(cx).value().to_string();
                        on_confirm(&token, cx);
                        window.close_dialog(cx);
                    })),
            )
    });
}
