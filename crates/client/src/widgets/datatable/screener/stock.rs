//! Finviz screener export rows for one saved preset, fetched through the
//! `finviz-api` SDK's generic CSV: columns are built from the export's
//! headers (a preset's `columns` parameter picks the set) and cells keep
//! the verbatim Finviz text for display plus the parsed numeric value for
//! sorting (see `protocol::Cell` — the server types them). The selected
//! preset name lives here (persisted by the panel) and is kept in step
//! with the shared settings by [`StockScreenerDelegate::sync_with_settings`].

use std::cmp::Ordering;

use crate::data::api::Cell;
use crate::tr;
use crate::utils::{settings, state::AppState};
use crate::widgets::datatable::{self, DataPage};
use gpui_kit::{
    App, Context, IntoElement, Pixels, Window,
    component::table::{Column, ColumnSort, TableDelegate, TableState},
};

#[derive(Clone)]
pub struct StockScreenerDelegate {
    /// The selected preset name; `None` = no presets exist.
    pub(crate) preset: Option<String>,
    /// The export page: typed cells aligned with the columns built from
    /// the headers, plus the fetch handshake (the panel stamps the
    /// generation it started with and discards superseded responses).
    pub(crate) page: DataPage<Vec<Cell>>,
    /// The rows as fetched (the Finviz `order_by` order) — the header
    /// sort's three-state "Default" step restores this.
    unsorted: Vec<Vec<Cell>>,
    columns: Vec<Column>,
}

impl StockScreenerDelegate {
    /// Build from the shared settings, preferring `preset_hint` when that
    /// preset still exists and falling back to the first one otherwise.
    pub fn new(preset_hint: Option<String>, cx: &App) -> Self {
        let mut this = Self {
            preset: preset_hint,
            page: DataPage::default(),
            unsorted: Vec::new(),
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
        self.clear_rows();
        true
    }

    /// Switch to `name`; returns whether the selection changed.
    pub fn select(&mut self, name: String) -> bool {
        if self.preset.as_deref() == Some(name.as_str()) {
            return false;
        }
        self.preset = Some(name);
        self.clear_rows();
        true
    }

    /// Drop the rows and invalidate any in-flight fetch for the previous
    /// preset.
    fn clear_rows(&mut self) {
        self.page.rows.clear();
        self.unsorted.clear();
        self.page.fetch_generation += 1;
    }

    /// Install a new export (headers + rows) and rebuild the columns from
    /// the headers. The caller must refresh the `TableState` afterwards —
    /// column groups are a construction-time snapshot.
    pub fn set_data(&mut self, headers: Vec<String>, rows: Vec<Vec<Cell>>) {
        self.columns = Self::columns(&headers);
        self.unsorted = rows.clone();
        self.page.rows = rows;
        self.page.is_loading = false;
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
            self.page
                .rows
                .iter()
                .map(|row| {
                    row.get(ix)
                        .map(|cell| cell.text.clone())
                        .unwrap_or_default()
                })
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
                Column::new(format!("col_{i}"), name).sortable()
            })
            .collect()
    }

    /// Save a preset into the shared settings (replacing a same-named one).
    /// Every Screener panel follows through its settings observer.
    pub fn add_preset(
        name: String,
        parameter: anysdk::finviz_sdk::screener::Parameter,
        cx: &mut App,
    ) {
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
        for row in &self.page.rows {
            if let Some(cell) = row.get(col_ix) {
                width = width.max(datatable::measure_text(window, &cell.text));
            }
        }
        Some(width)
    }

    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.page.rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        self.columns[col_ix].clone()
    }

    fn loading(&self, _cx: &App) -> bool {
        self.page.is_loading
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
        self.page
            .rows
            .get(row_ix)
            .and_then(|row| row.get(col_ix))
            .map(|cell| cell.text.clone())
            .unwrap_or_default()
    }

    /// 三态列头排序:Default 恢复导出原序(Finviz `order_by`),升/降按
    /// 单元格数值排、无值的沉底(文本列全部无值时退化为按文本排)。排序
    /// 只动行序,列组不变,`notify` 即可。
    fn perform_sort(
        &mut self,
        col_ix: usize,
        sort: ColumnSort,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) {
        if sort == ColumnSort::Default {
            self.page.rows = self.unsorted.clone();
            cx.notify();
            return;
        }
        let mut rows = std::mem::take(&mut self.page.rows);
        sort_rows(&mut rows, col_ix, sort);
        self.page.rows = rows;
        cx.notify();
    }
}

/// `perform_sort` 的行排序:带数值的按数值升序,无值的沉底(两个都
/// 无值时按文本);降序只反转有值/文本部分的次序,无值仍沉底。
fn sort_rows(rows: &mut [Vec<Cell>], col_ix: usize, sort: ColumnSort) {
    rows.sort_by(|a, b| {
        let (av, bv) = (
            a.get(col_ix).and_then(|cell| cell.value),
            b.get(col_ix).and_then(|cell| cell.value),
        );
        match (av, bv) {
            // 无值恒沉底,与方向无关——先于方向翻转判定。
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            _ => {
                let ord = compare_cells(a.get(col_ix), b.get(col_ix));
                if sort == ColumnSort::Descending {
                    ord.reverse()
                } else {
                    ord
                }
            }
        }
    });
}

/// 同方向下的单元格比较:带数值的按数值,两个都无值时按文本。
/// (Some/None 的沉底语义在 [`sort_rows`] 里先行判定。)
fn compare_cells(a: Option<&Cell>, b: Option<&Cell>) -> Ordering {
    match (a.and_then(|cell| cell.value), b.and_then(|cell| cell.value)) {
        (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
        (None, None) => {
            let a = a.map(|cell| cell.text.as_str()).unwrap_or("");
            let b = b.map(|cell| cell.text.as_str()).unwrap_or("");
            a.cmp(b)
        }
        // 沉底语义由 sort_rows 判定,这里不会到达;给个稳定值兜底。
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(cells: &[(&str, Option<f64>)]) -> Vec<Cell> {
        cells
            .iter()
            .map(|(text, value)| match value {
                Some(value) => Cell::valued(*text, *value),
                None => Cell::text(*text),
            })
            .collect()
    }

    /// 数值列:升序按值,None 沉底;降序反转但 None 仍沉底。
    #[test]
    fn sort_puts_values_first_and_none_last_in_both_directions() {
        let mut rows = [
            row(&[("AAPL", Some(2.0))]),
            row(&[("-", None)]),
            row(&[("TSLA", Some(1.0))]),
            row(&[("DOG", None)]),
        ];
        let cell = |r: &Vec<Cell>| r[0].clone();
        sort_rows(&mut rows, 0, ColumnSort::Ascending);
        assert_eq!(cell(&rows[0]).text, "TSLA");
        assert_eq!(cell(&rows[1]).text, "AAPL");
        assert_eq!(cell(&rows[2]).text, "-", "无值沉底");
        assert_eq!(cell(&rows[3]).text, "DOG");

        sort_rows(&mut rows, 0, ColumnSort::Descending);
        assert_eq!(cell(&rows[0]).text, "AAPL");
        assert_eq!(cell(&rows[1]).text, "TSLA");
        // 无值仍沉底(两个无值行之间的相对次序不作断言)。
        let tail: Vec<String> = rows[2..].iter().map(|r| r[0].text.clone()).collect();
        assert_eq!(tail.len(), 2);
        assert!(
            tail.contains(&"-".to_string()) && tail.contains(&"DOG".to_string()),
            "降序无值仍沉底: {tail:?}"
        );
    }

    /// 纯文本列(全无数值)退化为按文本比较,降序反转。
    #[test]
    fn text_only_columns_fall_back_to_text_order() {
        let mut rows = [
            row(&[("TSLA", None)]),
            row(&[("AAPL", None)]),
            row(&[("NVDA", None)]),
        ];
        sort_rows(&mut rows, 0, ColumnSort::Ascending);
        let names: Vec<String> = rows.iter().map(|r| r[0].text.clone()).collect();
        assert_eq!(names, ["AAPL", "NVDA", "TSLA"]);

        sort_rows(&mut rows, 0, ColumnSort::Descending);
        let names: Vec<String> = rows.iter().map(|r| r[0].text.clone()).collect();
        assert_eq!(names, ["TSLA", "NVDA", "AAPL"]);
    }
}
