//! The dock panels. Each panel type implements [`Panel`] (identity and
//! construction) on top of gpui-kit's two dock traits; [`all`] is the single
//! list of shipped panels and drives registration. The "Panels" menu renders
//! these types via `widgets/menu/panels.rs` as family submenus whose entries
//! drop the family word — adding a panel means implementing it, one entry in
//! [`all`] and one in that menu.

pub mod calendar;
pub mod chart;
pub mod experiment;
pub mod news;
pub mod quote;
pub mod screener;
pub mod trades;
pub mod watchlist;

pub use calendar::{EarningsCalendarPanel, EconomicCalendarPanel, IPOCalendarPanel};
pub use chart::ChartPanel;
pub use experiment::ExperimentPanel;
pub use news::{MarketNewsPanel, StockNewsPanel};
pub use quote::QuotePanel;
pub use screener::{MarketScreenerPanel, StockScreenerPanel};
pub use trades::TradesPanel;
pub use watchlist::WatchlistPanel;

use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, SharedString,
    Styled as _, WeakEntity, Window,
    base::{
        dock::{
            DockArea, DockLayout, DockPlacement, PanelBuildContext, PanelInfo, PanelRegistry,
            PanelState, register_panel,
        },
        h_flex,
    },
    component::{Icon, button::Button},
};

use crate::tr;
use crate::utils::AppIcon;
use crate::utils::state::{AppState, HubEvent, ServerStatus};
use crate::widgets::icon_button;

/// What every ZenSidus panel adds on top of gpui-kit's dock traits.
pub trait Panel: Sized + gpui_kit::component::dock::Panel + gpui_kit::base::dock::Panel {
    /// The entity holding the panel's data (a `TableState<Delegate>` for the
    /// table panels, `()` for placeholders); [`Self::refresh`] reloads it.
    type DataEntity: 'static;

    /// Identifies the panel in persisted layouts and in the registry. Once
    /// chosen, never change it: unknown names come back as placeholders.
    const NAME: &'static str;
    /// Icon of the tab title and the "Panels" menu entry.
    const ICON: AppIcon;
    /// i18n key of the default tab title and the menu label.
    const TITLE_KEY: &'static str;

    /// Build the panel; `info` is the payload a previous `dump` persisted,
    /// `Null` for a fresh instance.
    fn new(window: &mut Window, cx: &mut Context<Self>, info: &serde_json::Value) -> Self;

    /// Reload the data entity from its source. Called once from `new` for the
    /// startup load and again from the toolbar refresh button.
    fn refresh(_entity: WeakEntity<Self::DataEntity>, _cx: &mut App) {}

    /// Register the builder the dock uses to restore this panel by name.
    fn register(cx: &mut App) {
        register_panel(cx, Self::NAME, |context, window, cx| {
            let info = info_payload(context.state()).clone();
            gpui_kit::component::dock::panel_handle(cx.new(|cx| Self::new(window, cx, &info)))
        })
    }

    /// The default tab title: icon plus the translated [`Self::TITLE_KEY`].
    /// Panels with a dynamic label (StockNews, StockScreener) build their own.
    fn default_title() -> impl IntoElement {
        panel_title(Self::ICON, tr!(Self::TITLE_KEY))
    }

    /// Type-erased descriptor for [`all`].
    fn spec() -> PanelSpec {
        PanelSpec {
            name: Self::NAME,
            icon: Self::ICON,
            title_key: Self::TITLE_KEY,
            register: Self::register,
        }
    }
}

/// One shipped panel type, as the menu and the registry see it.
#[derive(Clone, Copy)]
pub struct PanelSpec {
    pub name: &'static str,
    pub icon: AppIcon,
    pub title_key: &'static str,
    register: fn(&mut App),
}

/// Every shipped panel, in "Panels" menu order. Family members (news /
/// screener / calendar) sit adjacent, mirroring the `panels/` module tree.
pub fn all() -> [PanelSpec; 12] {
    [
        WatchlistPanel::spec(),
        StockNewsPanel::spec(),
        MarketNewsPanel::spec(),
        QuotePanel::spec(),
        TradesPanel::spec(),
        StockScreenerPanel::spec(),
        MarketScreenerPanel::spec(),
        ChartPanel::spec(),
        EconomicCalendarPanel::spec(),
        EarningsCalendarPanel::spec(),
        IPOCalendarPanel::spec(),
        ExperimentPanel::spec(),
    ]
}

/// Register every panel builder. Must run before any window restores a
/// persisted layout.
pub fn init(cx: &mut App) {
    for spec in all() {
        (spec.register)(cx);
    }
}

/// A fresh instance of `P` (no persisted payload), for default layouts.
pub fn fresh<P: Panel>(window: &mut Window, cx: &mut App) -> Entity<P> {
    cx.new(|cx| P::new(window, cx, &serde_json::Value::Null))
}

/// Open a fresh instance of the panel registered under `name` in the center
/// tab group; repeated calls add more tabs. Goes through the same registry
/// the layout restore uses, so the caller needs no concrete panel type.
pub fn open_in_center(
    area: &mut DockArea,
    name: &str,
    window: &mut Window,
    cx: &mut Context<DockArea>,
) {
    let state = PanelState::new(name);
    let info = PanelInfo::Panel(serde_json::Value::Null);
    let context = PanelBuildContext::new(cx.weak_entity(), &state, &info);
    let Some(panel) = PanelRegistry::build_panel(name, context, window, cx) else {
        tracing::error!(
            "{}",
            tr!("app.log.panel_not_registered", name = format!("{name:?}"))
        );
        return;
    };

    if area.is_empty(DockPlacement::Center, cx) {
        // The empty center tree has no tab group to merge into; seed the
        // layout directly instead of adding beside the empty root split.
        area.set_center(DockLayout::tabs().panel_view(panel, cx), window, cx);
    } else {
        area.add_panel_view(panel, DockPlacement::Center, None, window, cx);
    }
}

/// Tab title: an icon followed by the label.
pub(super) fn panel_title(icon: AppIcon, label: impl Into<SharedString>) -> impl IntoElement {
    h_flex()
        .gap_1p5()
        .items_center()
        .child(Icon::new(icon))
        .child(label.into())
}

/// Title-bar button that re-runs [`Panel::refresh`] on the panel's data.
pub(super) fn refresh_button<P: Panel>(
    id: &'static str,
    entity: WeakEntity<P::DataEntity>,
) -> Button {
    icon_button(
        id,
        AppIcon::Refresh,
        tr!("app.settings.refresh"),
        false,
        move |_, _, cx| P::refresh(entity.clone(), cx),
    )
}

/// Re-run the panel's data load when the hub reports a fresh `Connected`:
/// startup auto-load stays silent while the server is down and the panel
/// fills in once the connection lands (reconnects included). Wire it in
/// `new` right after the startup fetch.
pub(crate) fn reload_on_connect<P>(data: WeakEntity<P::DataEntity>, cx: &mut Context<P>)
where
    P: Panel + 'static,
{
    let hub = cx.global::<AppState>().hub.clone();
    cx.subscribe(&hub, move |_, hub, _: &HubEvent, cx| {
        if matches!(hub.read(cx).status(), ServerStatus::Connected) {
            P::refresh(data.clone(), cx);
        }
    })
    .detach();
}

fn info_payload(state: &PanelState) -> &serde_json::Value {
    match &state.info {
        PanelInfo::Panel(value) => value,
        _ => &serde_json::Value::Null,
    }
}
