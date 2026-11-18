//! Scaffolding shared by the two calendar panels (economic + earnings):
//! both ride a DatePicker in the tab title bar with the same pinning rules.

use gpui_kit::{
    Entity, IntoElement, SharedString, Styled as _, px,
    component::{Sizable as _, date_picker::{DatePicker, DatePickerState}},
};

/// The calendar panel's date picker, pinned to a fixed width. The picker's
/// own root is `w_full` — left alone it eats the whole tab row and pushes
/// the toolbar (refresh button) out of view, so the width is pinned here.
pub(crate) fn pinned_date_picker(
    state: &Entity<DatePickerState>,
    placeholder: SharedString,
) -> impl IntoElement {
    DatePicker::new(state)
        .small()
        .placeholder(placeholder)
        .w(px(190.))
}
