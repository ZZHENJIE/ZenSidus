//! Developer scratch panel: buttons that open the shared dialogs directly,
//! so they can be exercised without a feature entry point.

use gpui_kit::{
    App, Context, EventEmitter, FocusHandle, IntoElement, Render, Window, base::dock::PanelEvent,
    div,
};

use crate::panels::Panel;
use crate::utils::AppIcon;

pub struct ExperimentPanel {
    focus_handle: FocusHandle,
}

impl Panel for ExperimentPanel {
    type DataEntity = ();
    const NAME: &'static str = "ExperimentPanel";
    const ICON: AppIcon = AppIcon::Experiment;
    const TITLE_KEY: &'static str = "app.panels.experiment";

    fn new(_window: &mut Window, cx: &mut Context<Self>, _info: &serde_json::Value) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
        }
    }
}

impl gpui_kit::base::dock::Panel for ExperimentPanel {
    fn panel_name(&self) -> &'static str {
        Self::NAME
    }
}

impl EventEmitter<PanelEvent> for ExperimentPanel {}

impl gpui_kit::Focusable for ExperimentPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl gpui_kit::component::dock::Panel for ExperimentPanel {
    fn title(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        Self::default_title()
    }
}

impl Render for ExperimentPanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
