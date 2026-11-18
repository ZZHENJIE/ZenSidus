//! The generic "are you sure?" dialog and the standard dialog footer.

use std::rc::Rc;

use gpui_kit::{
    App, ClickEvent, IntoElement, ParentElement as _, SharedString, Styled as _, Window,
    base::h_flex,
    component::{
        Sizable as _, WindowExt,
        button::{Button, ButtonVariants as _},
    },
    div,
};

use crate::tr;

/// A yes/no confirmation: `message` under `title`; Cancel closes, Confirm
/// runs `on_confirm` and then closes.
pub fn open_confirm_dialog(
    window: &mut Window,
    cx: &mut App,
    title: SharedString,
    message: SharedString,
    on_confirm: impl Fn(&mut App) + 'static,
) {
    let on_confirm = Rc::new(on_confirm);

    window.open_dialog(cx, move |dialog, _window, _cx| {
        let on_confirm = on_confirm.clone();
        dialog
            .title(title.clone())
            .child(div().child(message.clone()))
            .footer(dialog_footer("confirm", move |_, window, cx| {
                on_confirm(cx);
                window.close_dialog(cx);
            }))
    });
}

/// The standard Cancel / Confirm pair for dialog footers. Cancel closes the
/// dialog; Confirm runs `on_confirm` (which closes it itself when done).
/// `id` prefixes the two element ids.
pub fn dialog_footer(
    id: &'static str,
    on_confirm: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    h_flex()
        .justify_end()
        .gap_2()
        .child(
            Button::new(SharedString::from(format!("{id}-cancel")))
                .outline()
                .small()
                .label(tr!("app.dialog.cancel"))
                .on_click(|_, window, cx| window.close_dialog(cx)),
        )
        .child(
            Button::new(SharedString::from(format!("{id}-confirm")))
                .primary()
                .small()
                .label(tr!("app.dialog.confirm"))
                .on_click(on_confirm),
        )
}
