//! Finviz screener export rows for one saved preset, fetched through the
//! `finviz-api` SDK's generic CSV: columns are built from the export's
//! headers (a preset's `columns` parameter picks the set) and rows are the
//! raw cells, already Finviz-formatted. The selected preset name lives here
//! (persisted by the panel) and is kept in step with the shared settings by
//! [`StockScreenerDelegate::sync_with_settings`].

use crate::tr;
use crate::utils::{settings, state::AppState};
use crate::widgets::datatable;
use gpui_kit::{
    App, Context, IntoElement, Pixels, Window,
    component::table::{Column, TableDelegate, TableState},
};

#[derive(Clone)]
pub struct StockScreenerDelegate {
    /// The selected preset name; `None` = no presets exist.
    pub(crate) preset: Option<String>,
    /// Raw CSV rows, aligned with the columns built from the export's
    /// headers.
    pub(crate) data: Vec<Vec<String>>,
    pub(crate) is_loading: bool,
    /// Monotonic fetch counter: the panel's fetch stamps the generation it
    /// started with and discards the response when a newer fetch (or a
    /// preset switch) has superseded it.
    pub(crate) fetch_generation: u64,
    columns: Vec<Column>,
}

impl StockScreenerDelegate {
    /// Build from the shared settings, preferring `preset_hint` when that
    /// preset still exists and falling back to the first one otherwise.
    pub fn new(preset_hint: Option<String>, cx: &App) -> Self {
        let mut this = Self {
            preset: preset_hint,
            data: Vec::new(),
            is_loading: false,
            fetch_generation: 0,
            columns: Vec::new(),
        };
        this.sync_with_settings(cx);
        this
    }

    /// Preset names in settings order (a `BTreeMap`, so "first" is stable).
    pub fn preset_names(cx: &App) -> Vec<String> {
        cx.global::<AppState>()
            .settings
            .finviz
            .screener_parameter
            .keys()
            .cloned()
            .collect()
    }

    /// Re-read the preset list from settings: a selection that vanished
    /// (deleted here or on the settings page), or none at all, falls back to
    /// the first preset. Returns whether the selection changed — the caller
    /// refetches then.
    pub fn sync_with_settings(&mut self, cx: &App) -> bool {
        let presets = &cx.global::<AppState>().settings.finviz.screener_parameter;
        let next = self
            .preset
            .clone()
            .filter(|name| presets.contains_key(name))
            .or_else(|| presets.keys().next().cloned());
        if next == self.preset {
            return false;
        }
        self.preset = next;
        self.data.clear();
        // Invalidate any in-flight fetch for the previous preset.
        self.fetch_generation += 1;
        true
    }

    /// Switch to `name`; returns whether the selection changed.
    pub fn select(&mut self, name: String) -> bool {
        if self.preset.as_deref() == Some(name.as_str()) {
            return false;
        }
        self.preset = Some(name);
        self.data.clear();
        // Invalidate any in-flight fetch for the previous preset.
        self.fetch_generation += 1;
        true
    }

    /// Install a new export (headers + rows) and rebuild the columns from
    /// the headers. The caller must refresh the `TableState` afterwards —
    /// column groups are a construction-time snapshot.
    pub fn set_data(&mut self, headers: Vec<String>, rows: Vec<Vec<String>>) {
        self.columns = Self::columns(&headers);
        self.data = rows;
        self.is_loading = false;
    }

    /// The export's Ticker column (by header), when the preset includes it —
    /// gates the chart view.
    pub fn ticker_column(&self) -> Option<usize> {
        self.columns
            .iter()
            .position(|column| column.name == "Ticker")
    }

    /// The tickers of the result in row order; requires a Ticker column.
    pub fn tickers(&self) -> Option<Vec<String>> {
        let ix = self.ticker_column()?;
        Some(
            self.data
                .iter()
                .map(|row| row.get(ix).cloned().unwrap_or_default())
                .collect(),
        )
    }

    fn columns(headers: &[String]) -> Vec<Column> {
        headers
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let name = name.trim();
                let name = if name.is_empty() {
                    tr!("app.stock_screener.unnamed_column", index = i + 1).to_string()
                } else {
                    name.to_string()
                };
                Column::new(format!("col_{i}"), name)
            })
            .collect()
    }

    /// Save a preset into the shared settings (replacing a same-named one).
    /// Every Screener panel follows through its settings observer.
    pub fn add_preset(name: String, parameter: anysdk::finviz_sdk::screener::Parameter, cx: &mut App) {
        settings::update_settings(cx, |s| {
            s.finviz.screener_parameter.insert(name, parameter);
        });
    }

    /// Save a preset, renaming it when it used to live under `previous`
    /// (the old entry is removed; a same-named target is overwritten, the
    /// same rule the new-preset form has always had).
    pub fn upsert_preset(
        previous: Option<&str>,
        name: String,
        parameter: anysdk::finviz_sdk::screener::Parameter,
        cx: &mut App,
    ) {
        settings::update_settings(cx, |s| {
            let presets = &mut s.finviz.screener_parameter;
            if let Some(previous) = previous
                && previous != name
            {
                presets.remove(previous);
            }
            presets.insert(name, parameter);
        });
    }

    /// Remove a preset from the shared settings; panels showing it fall back
    /// to the first remaining one through their settings observer.
    pub fn remove_preset(name: &str, cx: &mut App) {
        settings::update_settings(cx, |s| {
            s.finviz.screener_parameter.remove(name);
        });
    }
}

impl TableDelegate for StockScreenerDelegate {
    /// Fit to the widest drawn cell or header; the table adds the cell
    /// padding and clamps to the column's min/max on top of this.
    fn fit_column(
        &mut self,
        col_ix: usize,
        window: &mut Window,
        _cx: &mut Context<TableState<Self>>,
    ) -> Option<Pixels> {
        self.columns.get(col_ix)?;
        let mut width = datatable::measure_text(window, &self.columns[col_ix].name);
        for row in &self.data {
            if let Some(cell) = row.get(col_ix) {
                width = width.max(datatable::measure_text(window, cell));
            }
        }
        Some(width)
    }

    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.data.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        self.columns[col_ix].clone()
    }

    fn loading(&self, _cx: &App) -> bool {
        self.is_loading
    }

    fn render_loading(
        &mut self,
        _size: gpui_kit::component::Size,
        _window: &mut Window,
        _cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        datatable::loading_view()
    }

    /// No preset yet → how to get one; a preset with no rows → say so.
    fn render_empty(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let hint = if self.preset.is_some() {
            tr!("app.stock_screener.no_data")
        } else {
            tr!("app.stock_screener.empty")
        };
        datatable::empty_view(hint, cx)
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        self.data
            .get(row_ix)
            .and_then(|row| row.get(col_ix))
            .cloned()
            .unwrap_or_default()
    }
}
