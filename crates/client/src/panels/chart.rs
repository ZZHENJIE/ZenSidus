//! Chart panel — placeholder until the candlestick view lands. It is the
//! default center panel of new main and custom windows.

use gpui_kit::{
    App, Context, EventEmitter, FocusHandle, IntoElement, ParentElement as _, Render, Window,
    base::dock::PanelEvent, div,
};

use crate::panels::Panel;
use crate::utils::AppIcon;

pub struct ChartPanel {
    focus_handle: FocusHandle,
}

impl Panel for ChartPanel {
    type DataEntity = ();
    const NAME: &'static str = "ChartPanel";
    const ICON: AppIcon = AppIcon::Chart;
    const TITLE_KEY: &'static str = "app.panels.chart";

    fn new(_window: &mut Window, cx: &mut Context<Self>, _info: &serde_json::Value) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
        }
    }
}

impl gpui_kit::base::dock::Panel for ChartPanel {
    fn panel_name(&self) -> &'static str {
        Self::NAME
    }
}

impl EventEmitter<PanelEvent> for ChartPanel {}

impl gpui_kit::Focusable for ChartPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl gpui_kit::component::dock::Panel for ChartPanel {
    fn title(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        Self::default_title()
    }
}

impl Render for ChartPanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().child(crate::tr!("app.page.placeholder"))
    }
}
