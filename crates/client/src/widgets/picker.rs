//! A single-select combobox over a list of names, shared by the Watchlist
//! (groups) and Screener (presets) toolbars — with the management footer
//! (add / edit / delete) the three panels share.

use std::rc::Rc;

use gpui_kit::{
    AnyElement, App, AppContext as _, Entity, IntoElement, ParentElement as _, SharedString,
    Styled as _, Window,
    base::h_flex,
    component::{
        Icon, Sizable as _,
        combobox::{Combobox, ComboboxState},
        searchable_list::SearchableVec,
    },
};

use crate::widgets::icon_button;

/// Reusable single-select combobox over a plain string list, shared by the
/// Watchlist (groups) and Screener (presets) panels. Owns its state; callers
/// subscribe to `state()` for `ComboboxEvent::Change` and resync items via
/// [`Self::set_items`].
pub struct StringPicker {
    state: Entity<ComboboxState<SearchableVec<String>>>,
}

impl StringPicker {
    /// Create the picker with an initial item list and (optional) selection.
    pub fn new(
        items: Vec<String>,
        selected: Option<String>,
        window: &mut Window,
        cx: &mut App,
    ) -> Self {
        let state = cx.new(|cx| {
            ComboboxState::new(SearchableVec::new(items), vec![], window, cx).searchable(false)
        });
        let picker = Self { state };
        picker.set_selected(selected, window, cx);
        picker
    }

    pub fn state(&self) -> &Entity<ComboboxState<SearchableVec<String>>> {
        &self.state
    }

    /// Replace the item list, keeping `selected` selected when it still
    /// exists in the list.
    pub fn set_items(&self, items: Vec<String>, window: &mut Window, cx: &mut App) {
        self.state.update(cx, |state, cx| {
            state.set_items(SearchableVec::new(items), window, cx);
        });
    }

    /// Programmatically select one value (must exist in the current list).
    pub fn set_selected(&self, selected: Option<String>, window: &mut Window, cx: &mut App) {
        self.state.update(cx, |state, cx| match selected {
            Some(name) => {
                state.set_selected_values(&[name], window, cx);
            }
            None => state.clear_selection(cx),
        });
    }

    /// The styled combobox element for the panel layout. `footer` renders
    /// management buttons inside the dropdown (e.g. add/delete entries).
    pub fn element(
        &self,
        placeholder: SharedString,
        footer: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
    ) -> impl IntoElement {
        Combobox::new(&self.state)
            .small()
            .w_full()
            .placeholder(placeholder)
            .cleanable(false)
            .footer(footer)
    }

    /// The styled combobox with the management footer the Watchlist and both
    /// Screener panels share: optional **add** / **edit** / **delete**
    /// buttons under the dropdown. `add` is always enabled; `edit` and
    /// `delete` render disabled while nothing is selected. `selected`
    /// mirrors the panel's current selection (it drives the disabled states
    /// and is handed to every click).
    pub fn managed_element(
        &self,
        placeholder: SharedString,
        selected: impl Fn(&App) -> Option<String> + 'static,
        add: Option<PickerAction>,
        edit: Option<PickerAction>,
        delete: Option<PickerAction>,
    ) -> impl IntoElement {
        self.element(placeholder, move |_window, cx| {
            let selected_now = selected(cx);
            let button = |action: &PickerAction, disabled: bool| {
                let on_click = action.on_click.clone();
                let selected = selected_now.clone();
                icon_button(
                    action.id,
                    action.icon.clone(),
                    action.tooltip.clone(),
                    disabled,
                    move |_, window, cx| on_click(window, cx, selected.clone()),
                )
            };
            h_flex()
                .w_full()
                .gap_1()
                .children(add.as_ref().map(|action| button(action, false)))
                .children(
                    edit.as_ref()
                        .map(|action| button(action, selected_now.is_none())),
                )
                .children(
                    delete
                        .as_ref()
                        .map(|action| button(action, selected_now.is_none())),
                )
                .into_any_element()
        })
    }
}

/// The click callback of a [`PickerAction`]: `selected` is the picker's
/// current selection at click time (`None` = nothing selected).
pub type PickerClick = dyn Fn(&mut Window, &mut App, Option<String>);

/// One optional management button of the picker dropdown's footer (see
/// [`StringPicker::managed_element`]). Its click receives the picker's
/// current selection at click time (`None` = nothing selected).
pub struct PickerAction {
    pub id: &'static str,
    pub icon: Icon,
    pub tooltip: SharedString,
    pub on_click: Rc<PickerClick>,
}

impl PickerAction {
    pub fn new(
        id: &'static str,
        icon: impl Into<Icon>,
        tooltip: impl Into<SharedString>,
        on_click: impl Fn(&mut Window, &mut App, Option<String>) + 'static,
    ) -> Self {
        Self {
            id,
            icon: icon.into(),
            tooltip: tooltip.into(),
            on_click: Rc::new(on_click),
        }
    }
}
