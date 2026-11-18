//! The "Panels" title-bar menu, laid out to mirror the `panels/` module
//! tree: the news/screener/calendar families get a submenu each, and their
//! entries drop the family word — inside "Calendar" the label "Economic" is
//! unambiguous, so repeating it would be noise (dock tabs keep the full
//! title, they have no submenu for context). Standalone panels sit inline
//! with their full title. Every entry dispatches the single `OpenPanel`
//! action; the layout is hand-maintained — a new panel is added to
//! `panels::all` *and* here (the panel consts below make a stale entry a
//! compile error).

use gpui_kit::{
    InteractiveElement, IntoElement, SharedString,
    component::{
        Icon, Sizable,
        button::{Button, ButtonVariants},
        menu::{DropdownMenu, PopupMenu},
    },
};

use crate::{
    panels::{
        ChartPanel, EarningsCalendarPanel, EconomicCalendarPanel, ExperimentPanel, IPOCalendarPanel,
        MarketNewsPanel, MarketScreenerPanel, Panel, QuotePanel, StockNewsPanel,
        StockScreenerPanel, TradesPanel, WatchlistPanel, open_in_center,
    },
    tr,
    utils::{AppIcon, actions},
    windows::DockWindow,
};

/// One menu entry for `P`. Inside a family submenu the label drops the
/// family word (`app.menu.item.*`); standalone panels pass their full
/// `TITLE_KEY` label.
fn entry<P: Panel>(label: impl Into<SharedString>, menu: PopupMenu) -> PopupMenu {
    menu.menu_with_icon(
        label,
        P::ICON,
        Box::new(actions::OpenPanel {
            name: P::NAME.to_string(),
        }),
    )
}

pub fn menu() -> impl gpui_kit::IntoElement {
    Button::new("panels-menu")
        .ghost()
        .small()
        .label(tr!("app.menu.panels"))
        .dropdown_menu(|menu, window, cx| {
            let menu = entry::<WatchlistPanel>(tr!("app.panels.watchlist"), menu);
            let menu = menu.submenu_with_icon(
                Some(Icon::new(AppIcon::News)),
                tr!("app.menu.news"),
                window,
                cx,
                |sub, _, _| {
                    let sub = entry::<StockNewsPanel>(tr!("app.menu.item.stock"), sub);
                    entry::<MarketNewsPanel>(tr!("app.menu.item.market"), sub)
                },
            );
            let menu = entry::<QuotePanel>(tr!("app.panels.quote"), menu);
            let menu = entry::<TradesPanel>(tr!("app.panels.trades"), menu);
            let menu = menu.submenu_with_icon(
                Some(Icon::new(AppIcon::Filter)),
                tr!("app.menu.screener"),
                window,
                cx,
                |sub, _, _| {
                    let sub = entry::<StockScreenerPanel>(tr!("app.menu.item.stock"), sub);
                    entry::<MarketScreenerPanel>(tr!("app.menu.item.market"), sub)
                },
            );
            let menu = entry::<ChartPanel>(tr!("app.panels.chart"), menu);
            let menu = menu.submenu_with_icon(
                Some(Icon::new(AppIcon::Calendar)),
                tr!("app.menu.calendar"),
                window,
                cx,
                |sub, _, _| {
                    let sub = entry::<EconomicCalendarPanel>(tr!("app.menu.item.economic"), sub);
                    let sub = entry::<EarningsCalendarPanel>(tr!("app.menu.item.earnings"), sub);
                    entry::<IPOCalendarPanel>(tr!("app.menu.item.ipo"), sub)
                },
            );
            entry::<ExperimentPanel>(tr!("app.panels.experiment"), menu)
        })
}

/// Handle `OpenPanel` for a dock window: open the named panel in its center
/// tab group.
pub fn actions<T: DockWindow + 'static, E: IntoElement + InteractiveElement>(
    div: E,
    cx: &mut gpui_kit::Context<T>,
) -> E {
    div.on_action(cx.listener(|this, action: &actions::OpenPanel, window, cx| {
        this.dock_area().update(cx, |area, cx| {
            open_in_center(area, &action.name, window, cx);
        });
    }))
}
