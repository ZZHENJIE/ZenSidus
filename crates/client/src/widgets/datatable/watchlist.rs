//! One watchlist group with live prices from the Alpaca stream. Rows are
//! colored by the day change against the daily-bar open.

use std::collections::BTreeMap;

use gpui_kit::{
    App, ClickEvent, Context, Div, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled, Window,
    component::{
        ActiveTheme as _,
        table::{Column, TableDelegate, TableState},
    },
    div, px,
};

use crate::tr;
use crate::utils::{settings, state::AppState};
use crate::widgets::datatable;

/// Live quote state for one symbol. Held in a map that survives group
/// switches, so re-entering a group restores prices already seen.
#[derive(Clone, Debug, Default)]
pub struct Quote {
    /// Latest trade price (trades channel).
    pub last: Option<f64>,
    /// Today's open (daily-bars channel); the change baseline.
    pub open: Option<f64>,
    /// Latest update, formatted at receive time.
    pub updated: Option<SharedString>,
}

impl Quote {
    fn change(&self) -> Option<f64> {
        Some(self.last? - self.open?)
    }
}

/// Delegate backing the watchlist table. Owns the selected group, its symbol
/// order and the live quote values; `reload_from_settings` re-reads the
/// shared settings while keeping quote history for symbols that stay.
#[derive(Default)]
pub struct WatchlistDelegate {
    pub(crate) group: Option<String>,
    /// Symbols of the selected group, in settings order.
    pub(crate) symbols: Vec<String>,
    pub(crate) quotes: BTreeMap<String, Quote>,
}

impl WatchlistDelegate {
    /// Build from the shared settings, preferring `group_hint` when that
    /// group still exists and falling back to the first one otherwise.
    pub fn new(group_hint: Option<String>, cx: &App) -> Self {
        let groups = Self::groups(cx);
        let group = group_hint
            .filter(|name| groups.contains_key(name))
            .or_else(|| groups.keys().next().cloned());
        let mut this = Self {
            group,
            ..Self::default()
        };
        this.reload_from_settings(cx);
        this
    }

    fn groups(cx: &App) -> &BTreeMap<String, Vec<String>> {
        &cx.global::<AppState>().settings.watchlists
    }

    /// Re-read the shared settings: follow group deletions (fall back to the
    /// first remaining group) and symbol edits. Quote history is kept for
    /// symbols that stay.
    pub fn reload_from_settings(&mut self, cx: &App) {
        let groups = Self::groups(cx);
        if let Some(name) = &self.group
            && !groups.contains_key(name)
        {
            self.group = None;
        }
        if self.group.is_none() {
            self.group = groups.keys().next().cloned();
        }
        self.symbols = self
            .group
            .as_ref()
            .and_then(|name| groups.get(name))
            .cloned()
            .unwrap_or_default();
        for symbol in &self.symbols {
            self.quotes.entry(symbol.clone()).or_default();
        }
        // Drop history for symbols that left the group, so a long session of
        // group switches cannot grow the map without bound.
        self.quotes
            .retain(|symbol, _| self.symbols.contains(symbol));
    }

    pub fn apply_trade(&mut self, symbol: &str, price: f64, time: SharedString) -> bool {
        let Some(quote) = self.quotes.get_mut(symbol) else {
            return false;
        };
        quote.last = Some(price);
        quote.updated = Some(time);
        true
    }

    pub fn apply_open(&mut self, symbol: &str, open: f64) -> bool {
        self.quotes
            .get_mut(symbol)
            .map(|quote| quote.open = Some(open))
            .is_some()
    }
}

fn change_color(change: f64, cx: &App) -> Hsla {
    let theme = cx.theme();
    if change > 0. {
        theme.chart_bullish
    } else if change < 0. {
        theme.chart_bearish
    } else {
        theme.muted_foreground
    }
}

impl TableDelegate for WatchlistDelegate {
    fn columns_count(&self, _: &App) -> usize {
        5
    }

    fn rows_count(&self, _: &App) -> usize {
        self.symbols.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        match col_ix {
            0 => Column::new("symbol", tr!("app.watchlist.code")).width(px(72.)),
            1 => Column::new("last", tr!("app.watchlist.price"))
                .width(px(76.))
                .text_right(),
            2 => Column::new("change", tr!("app.watchlist.change"))
                .width(px(76.))
                .text_right(),
            3 => Column::new("change_pct", tr!("app.watchlist.change_pct"))
                .width(px(80.))
                .text_right(),
            _ => Column::new("time", tr!("app.watchlist.time"))
                .width(px(84.))
                .text_right(),
        }
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let Some(symbol) = self.symbols.get(row_ix).cloned() else {
            return div().into_any_element();
        };
        let quote = self.quotes.get(&symbol).cloned().unwrap_or_default();
        let muted = cx.theme().muted_foreground;

        match col_ix {
            0 => div().child(SharedString::from(symbol)).into_any_element(),
            1 => div()
                .text_right()
                .text_color(quote.last.map(|_| cx.theme().foreground).unwrap_or(muted))
                .child(SharedString::from(
                    quote
                        .last
                        .map(|v| format!("{v:.2}"))
                        .unwrap_or_else(|| "-".into()),
                ))
                .into_any_element(),
            2 => {
                let cell = match quote.change() {
                    Some(change) => div()
                        .text_right()
                        .text_color(change_color(change, cx))
                        .child(SharedString::from(format!("{change:+.2}"))),
                    None => div().text_right().text_color(muted).child("-"),
                };
                cell.into_any_element()
            }
            3 => {
                let cell = match quote.change() {
                    // A zero open would divide by zero (inf/NaN in the cell):
                    // fall through to the muted placeholder.
                    Some(change) if quote.open.unwrap_or_default() != 0. => {
                        let pct = change / quote.open.unwrap_or_default() * 100.;
                        div()
                            .text_right()
                            .text_color(change_color(change, cx))
                            .child(SharedString::from(format!("{pct:+.2}%")))
                    }
                    _ => div().text_right().text_color(muted).child("-"),
                };
                cell.into_any_element()
            }
            _ => div()
                .text_right()
                .text_color(muted)
                .child(quote.updated.unwrap_or_else(|| "-".into()))
                .into_any_element(),
        }
    }

    /// Ctrl + click on a row removes that symbol from the selected group.
    /// The removal goes through settings; the panel's settings observer
    /// reloads the table and resubscribes.
    fn render_tr(
        &mut self,
        row_ix: usize,
        _window: &mut Window,
        _cx: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        let mut row = div().id(("watchlist-row", row_ix));

        if let Some(symbol) = self.symbols.get(row_ix).cloned() {
            let group = self.group.clone();
            row = row.cursor_pointer().on_click(move |event, _window, cx| {
                if let ClickEvent::Mouse(mouse) = event
                    && mouse.down.modifiers.control
                {
                    settings::update_settings(cx, |s| {
                        if let Some(group) = &group
                            && let Some(symbols) = s.watchlists.get_mut(group)
                        {
                            symbols.retain(|existing| existing != &symbol);
                        }
                    });
                }
            });
        }
        row
    }

    fn render_empty(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        datatable::empty_view(tr!("app.watchlist.empty"), cx)
    }
}
