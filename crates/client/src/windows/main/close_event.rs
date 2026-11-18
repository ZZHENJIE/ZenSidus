//! The quit dialog shown when the main window's close button is pressed.

use crate::tr;
use gpui_kit::{
    App, ParentElement, Styled, Window,
    component::{
        WindowExt,
        button::{Button, ButtonVariants},
        dialog::DialogFooter,
    },
};

/// Intercept the platform close request: Cancel keeps running, "Don't save"
/// quits as is, "Save" persists every open window's geometry first. Always
/// returns `false` — the window itself never closes, the app quits instead.
pub(crate) fn on_close(window: &mut Window, cx: &mut App) -> bool {
    window.open_dialog(cx, |dialog, _, _| {
        dialog
            .title(tr!("app.windows.main.close_dialog_title"))
            .close_button(false)
            .keyboard(false)
            .child(
                DialogFooter::new()
                    .justify_center()
                    .child(
                        Button::new("cancel")
                            .flex_1()
                            .primary()
                            .label(tr!("app.windows.main.close_dialog_cancel"))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("no_save")
                            .flex_1()
                            .outline()
                            .label(tr!("app.windows.main.close_dialog_no_save"))
                            .on_click(|_, _, cx| {
                                // Settings mutations are debounced; flush them
                                // before quitting even on this path — only the
                                // window geometry is what "don't save" skips.
                                crate::utils::settings::flush_settings(cx);
                                cx.quit();
                            }),
                    )
                    .child(
                        Button::new("save")
                            .flex_1()
                            .primary()
                            .label(tr!("app.windows.main.close_dialog_save"))
                            .on_click(|_, _, cx| {
                                // This click runs inside the main window's own
                                // update, where its handle cannot be re-entered
                                // (`handle.update` fails with "window not
                                // found" and the main window's geometry would
                                // be the one skipped). Persist after the flush.
                                cx.defer(|cx| {
                                    crate::windows::persist_open_windows(cx);
                                    cx.quit();
                                });
                            }),
                    ),
            )
    });
    false
}
