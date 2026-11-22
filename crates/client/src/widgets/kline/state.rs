//! K 线图的状态实体:历史 bars、视口(viewport)、实时推送的聚合入口。
//!
//! 状态所有者是持有 [`KlineState`] 实体的视图(ChartPanel):面板把 RPC
//! 历史与推送喂进来,元素在绘制时读取、在交互时改写视口。所有变更方法
//! 返回"是否发生变化",由调用方 `cx.notify()` ——本模块不持有 GPUI 上下文。

use std::collections::BTreeMap;

use crate::data::api::Bar;

use super::scale;

/// 周期。`as_str` 是 Alpaca 的 timeframe 字符串(走 Bars RPC 原样透传),
/// `label` 是工具栏/菜单短标签(行业通用记法,不做本地化);菜单分组见
/// `timeframe_menu`(分/时/天/周/月)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Timeframe {
    OneMin,
    TwoMin,
    ThreeMin,
    FiveMin,
    TenMin,
    FifteenMin,
    ThirtyMin,
    OneHour,
    TwoHour,
    FourHour,
    #[default]
    OneDay,
    OneWeek,
    OneMonth,
}

impl Timeframe {
    /// 全部周期,按单位分组后从小到大排列(下拉菜单顺序)。
    pub const ALL: [Timeframe; 13] = [
        Timeframe::OneMin,
        Timeframe::TwoMin,
        Timeframe::ThreeMin,
        Timeframe::FiveMin,
        Timeframe::TenMin,
        Timeframe::FifteenMin,
        Timeframe::ThirtyMin,
        Timeframe::OneHour,
        Timeframe::TwoHour,
        Timeframe::FourHour,
        Timeframe::OneDay,
        Timeframe::OneWeek,
        Timeframe::OneMonth,
    ];

    /// 从 Alpaca timeframe 字符串解析(持久化 info 的 `timeframe` 字段)。
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tf| tf.as_str() == value)
    }

    /// Alpaca timeframe 字符串(Bars RPC 的 `timeframe` 参数)。
    pub fn as_str(self) -> &'static str {
        match self {
            Timeframe::OneMin => "1Min",
            Timeframe::TwoMin => "2Min",
            Timeframe::ThreeMin => "3Min",
            Timeframe::FiveMin => "5Min",
            Timeframe::TenMin => "10Min",
            Timeframe::FifteenMin => "15Min",
            Timeframe::ThirtyMin => "30Min",
            Timeframe::OneHour => "1Hour",
            Timeframe::TwoHour => "2Hour",
            Timeframe::FourHour => "4Hour",
            Timeframe::OneDay => "1Day",
            Timeframe::OneWeek => "1Week",
            Timeframe::OneMonth => "1Month",
        }
    }

    /// 工具栏/菜单短标签("1Mo" 用三字符与 "1m" 区分)。
    pub fn label(self) -> &'static str {
        match self {
            Timeframe::OneMin => "1m",
            Timeframe::TwoMin => "2m",
            Timeframe::ThreeMin => "3m",
            Timeframe::FiveMin => "5m",
            Timeframe::TenMin => "10m",
            Timeframe::FifteenMin => "15m",
            Timeframe::ThirtyMin => "30m",
            Timeframe::OneHour => "1h",
            Timeframe::TwoHour => "2h",
            Timeframe::FourHour => "4h",
            Timeframe::OneDay => "1D",
            Timeframe::OneWeek => "1W",
            Timeframe::OneMonth => "1Mo",
        }
    }

    /// 分钟/小时周期返回桶宽(分钟);天/周/月返回 0(实时走 DailyBar
    /// 推送,按对应的美东日/周/月聚合)。
    pub fn bucket_minutes(self) -> u32 {
        match self {
            Timeframe::OneMin => 1,
            Timeframe::TwoMin => 2,
            Timeframe::ThreeMin => 3,
            Timeframe::FiveMin => 5,
            Timeframe::TenMin => 10,
            Timeframe::FifteenMin => 15,
            Timeframe::ThirtyMin => 30,
            Timeframe::OneHour => 60,
            Timeframe::TwoHour => 120,
            Timeframe::FourHour => 240,
            Timeframe::OneDay | Timeframe::OneWeek | Timeframe::OneMonth => 0,
        }
    }

    /// 是否分钟/小时周期(决定订阅意愿与推送过滤)。
    pub fn is_intraday(self) -> bool {
        self.bucket_minutes() > 0
    }

    /// 历史加载条数(1Min 一天约 390 根,多留一些)。
    pub fn history_limit(self) -> u32 {
        if self == Timeframe::OneMin { 1000 } else { 500 }
    }
}

/// 视口:`right_ix` 是右边缘对应的连续 bar index(f32,允许亚桶平移),
/// `span` 是可见桶数。`start = right_ix - span`,左边界钳制在 0。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub right_ix: f32,
    pub span: f32,
}

/// 原始 bar(推送版快照;时间键 = 分钟起点或日线起点)。minute/updated 两
/// 种推送都是"这一分钟的当前完整快照",日线同理是"这一交易日的快照",
/// 所以直接整条替换、不做增量合并。
#[derive(Debug, Clone, Copy)]
struct RawBar {
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: i64,
    vwap: f64,
    trade_count: i64,
}

/// 原始分钟保留窗口(≈ 两个美股交易日)。
const MINUTE_RETENTION: i64 = 800;
/// 原始日线保留窗口(覆盖年桶回溯到 1 月 1 日)。
const DAY_RETENTION: i64 = 800;
/// 视口常量:最小/最大可见桶数、默认可见桶数、钉住最新时的右侧留白。
const MIN_SPAN: f32 = 15.;
const MAX_SPAN: f32 = 1000.;
const DEFAULT_SPAN: f32 = 120.;
const RIGHT_MARGIN: f32 = 5.;

/// K 线图的全部状态(见模块注释)。字段私有;读走方法。
#[derive(Debug)]
pub struct KlineState {
    symbol: Option<String>,
    timeframe: Timeframe,
    /// 历史 + 实时合并后的 bars,按 `time_ms` 升序。
    bars: Vec<Bar>,
    viewport: Viewport,
    /// 下一次 `set_history` 是否重置视口(切周期后重置,同周期刷新保留)。
    pending_viewport_reset: bool,
    /// RPC 请求代数:响应回来时代数不符即丢弃(标的/周期已再次切换)。
    load_generation: u64,
    loading: bool,
    /// 至少成功加载过一次(区分"无数据"与"还没选标的")。
    loaded: bool,
    /// 本次会话的原始分钟 bar(分钟/小时周期消费,聚合成分/时桶)。
    minutes: BTreeMap<i64, RawBar>,
    /// 本次会话的原始日线 bar(天/周/月/年周期消费,聚合到对应桶)。
    days: BTreeMap<i64, RawBar>,
}

impl KlineState {
    pub fn new(timeframe: Timeframe) -> Self {
        Self {
            symbol: None,
            timeframe,
            bars: Vec::new(),
            viewport: Viewport {
                right_ix: DEFAULT_SPAN,
                span: DEFAULT_SPAN,
            },
            pending_viewport_reset: true,
            load_generation: 0,
            loading: false,
            loaded: false,
            minutes: BTreeMap::new(),
            days: BTreeMap::new(),
        }
    }

    pub fn symbol(&self) -> Option<&str> {
        self.symbol.as_deref()
    }

    pub fn timeframe(&self) -> Timeframe {
        self.timeframe
    }

    pub fn bars(&self) -> &[Bar] {
        &self.bars
    }

    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    pub fn has_loaded(&self) -> bool {
        self.loaded
    }

    pub fn load_generation(&self) -> u64 {
        self.load_generation
    }

    /// 最新一根 bar 的收盘价(最新价线;空图返回 None)。
    pub fn last_close(&self) -> Option<f64> {
        self.bars.last().map(|bar| bar.close)
    }

    /// 切换标的:清空数据与视口,等待下一次历史加载。
    pub fn set_symbol(&mut self, symbol: Option<String>) -> bool {
        if self.symbol == symbol {
            return false;
        }
        self.symbol = symbol;
        self.bars.clear();
        self.minutes.clear();
        self.days.clear();
        self.loaded = false;
        self.pending_viewport_reset = true;
        true
    }

    /// 切换周期:保留旧图(历史到位前的过渡画面),重置标记视口。
    pub fn set_timeframe(&mut self, timeframe: Timeframe) -> bool {
        if self.timeframe == timeframe {
            return false;
        }
        self.timeframe = timeframe;
        self.pending_viewport_reset = true;
        true
    }

    /// 开始一次历史加载:置 loading 并推进请求代数,返回本次代数。
    pub fn begin_load(&mut self) -> u64 {
        self.loading = true;
        self.load_generation += 1;
        self.load_generation
    }

    /// 加载在发出前流产(离线、无标的):仅解除 loading。
    pub fn abort_load(&mut self, generation: u64) {
        if generation == self.load_generation {
            self.loading = false;
        }
    }

    /// 写入历史 bars(成功响应)。代数不符的响应被丢弃;同周期刷新保留
    /// 视口,切周期/换标的后的首次加载重置视口。
    pub fn set_history(&mut self, generation: u64, mut bars: Vec<Bar>) -> bool {
        if generation != self.load_generation {
            return false;
        }
        self.loading = false;
        self.loaded = true;
        bars.sort_unstable_by_key(|bar| bar.time_ms);
        bars.dedup_by_key(|bar| bar.time_ms);
        self.bars = bars;
        self.rebuild_viewport();
        true
    }

    /// 历史加载失败:仅解除 loading(错误已进消息中心)。
    pub fn fail_load(&mut self, generation: u64) -> bool {
        if generation != self.load_generation || !self.loading {
            return false;
        }
        self.loading = false;
        true
    }

    /// 日线实时推送(天/周/月/年周期消费):写入原始日线,按当前周期的
    /// 美东日/周/月/年桶重算并合并进 bars。分钟周期调用是 no-op。
    /// 键归一化到美东当日零点:同一交易日的多次推送(盘中快照)共享一键。
    pub fn apply_daily_bar(&mut self, bar: Bar) -> bool {
        if self.timeframe.bucket_minutes() != 0 {
            return false;
        }
        let day_key = scale::ny_date_ms(scale::ny_date(bar.time_ms));
        self.days.insert(
            day_key,
            RawBar {
                open: bar.open,
                high: bar.high,
                low: bar.low,
                close: bar.close,
                volume: bar.volume,
                vwap: bar.vwap,
                trade_count: bar.trade_count,
            },
        );
        self.prune_days();

        let bucket = self.daily_bucket_start(day_key);
        let Some(aggregate) = aggregate_days(&self.days, bucket, self.timeframe) else {
            return false;
        };

        let last_time = self.bars.last().map(|bar| bar.time_ms);
        match last_time {
            None => {
                self.bars.push(aggregate);
                self.rebuild_viewport();
                true
            }
            Some(time_ms) if self.daily_bucket_start(time_ms) == bucket => {
                let last = self.bars.last_mut().expect("checked above");
                *last = aggregate;
                true
            }
            Some(time_ms) if time_ms < bucket => {
                self.bars.push(aggregate);
                self.trim_viewport();
                true
            }
            // 落在最后一根之前的迟到推送:重算结果与历史一致,丢弃。
            Some(_) => false,
        }
    }

    /// 天/周/月/年周期:一根日线所属的桶起点(美东;OneDay 即日线本身)。
    fn daily_bucket_start(&self, day_ms: i64) -> i64 {
        match self.timeframe {
            Timeframe::OneWeek => scale::ny_week_start_ms(day_ms),
            Timeframe::OneMonth => scale::ny_month_start_ms(day_ms),
            _ => day_ms,
        }
    }

    /// 分钟实时推送(Alpaca minute/updated bars):写入原始分钟,重算所在
    /// 桶并合并进 bars。日线调用是 no-op。
    #[allow(clippy::too_many_arguments)]
    pub fn apply_minute_bar(
        &mut self,
        time_ms: i64,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
        volume: i64,
        vwap: f64,
        trade_count: i64,
    ) -> bool {
        let bucket_minutes = self.timeframe.bucket_minutes();
        if bucket_minutes == 0 {
            return false;
        }
        let minute = scale::bucket_start_ms(time_ms, 1);
        self.minutes.insert(
            minute,
            RawBar {
                open,
                high,
                low,
                close,
                volume,
                vwap,
                trade_count,
            },
        );
        self.prune_minutes();

        let bucket = scale::bucket_start_ms(minute, bucket_minutes);
        let bucket_end = bucket + (bucket_minutes as i64) * 60_000;
        let Some(aggregate) = aggregate_bars(self.minutes.range(bucket..bucket_end), bucket) else {
            return false;
        };

        let last_time = self.bars.last().map(|bar| bar.time_ms);
        match last_time {
            None => {
                self.bars.push(aggregate);
                self.rebuild_viewport();
                true
            }
            Some(time_ms) if scale::bucket_start_ms(time_ms, bucket_minutes) == bucket => {
                let last = self.bars.last_mut().expect("checked above");
                *last = aggregate;
                true
            }
            Some(time_ms) if time_ms < bucket => {
                self.bars.push(aggregate);
                self.trim_viewport();
                true
            }
            // 落在最后一根之前的迟到推送:重算结果与历史一致,丢弃。
            Some(_) => false,
        }
    }

    /// 以 `anchor_frac`(0..1,光标在价格区的水平占比)为锚点缩放:
    /// 光标下的 bar 缩放前后保持原位。
    pub fn zoom_at(&mut self, anchor_frac: f32, factor: f32) -> bool {
        let start = self.viewport.right_ix - self.viewport.span;
        let anchor_ix = start + anchor_frac * self.viewport.span;
        let span = (self.viewport.span * factor).clamp(MIN_SPAN, MAX_SPAN);
        let right_ix = anchor_ix + (1. - anchor_frac) * span;
        if (span - self.viewport.span).abs() < f32::EPSILON {
            return false;
        }
        self.viewport.span = span;
        self.viewport.right_ix = self.clamp_right(right_ix, span);
        true
    }

    /// 平移视图(正数 = 视野向左移,即内容右移)。拖拽方向已由元素换算。
    pub fn pan_by(&mut self, delta_ix: f32) -> bool {
        if delta_ix == 0. {
            return false;
        }
        let right = self.clamp_right(self.viewport.right_ix + delta_ix, self.viewport.span);
        if right == self.viewport.right_ix {
            return false;
        }
        self.viewport.right_ix = right;
        true
    }

    /// 回到最新(双击):右边缘钉在"最新 bar + 右侧留白"。
    pub fn pin_latest(&mut self) -> bool {
        let right = self.clamp_right(self.bars.len() as f32 + RIGHT_MARGIN, self.viewport.span);
        if right == self.viewport.right_ix {
            return false;
        }
        self.viewport.right_ix = right;
        true
    }

    /// 视口是否钉在最新(决定新 bar 到达时是否跟随右移)。
    pub fn is_following(&self) -> bool {
        self.viewport.right_ix >= self.bars.len() as f32
    }

    /// 右边缘的钳制:左界 = span(start ≥ 0),右界 = 最新 bar + 留白。
    fn clamp_right(&self, right_ix: f32, span: f32) -> f32 {
        let max_right = (self.bars.len() as f32 + RIGHT_MARGIN).max(span);
        right_ix.clamp(span, max_right)
    }

    /// 新 bar 追加后的视口维护:钉住最新则跟随右移,否则原位不动。
    fn trim_viewport(&mut self) {
        if self.is_following() {
            self.viewport.right_ix =
                self.clamp_right(self.bars.len() as f32 + RIGHT_MARGIN, self.viewport.span);
        } else {
            self.viewport.right_ix = self.clamp_right(self.viewport.right_ix, self.viewport.span);
        }
    }

    /// 全量重建视口:历史到位后调用。首次加载/换标的/切周期重置到默认
    /// 视野并钉住最新;同周期刷新保留视野(跟随状态照常维护)。
    fn rebuild_viewport(&mut self) {
        let n = self.bars.len() as f32;
        if self.pending_viewport_reset {
            self.pending_viewport_reset = false;
            self.viewport.span = DEFAULT_SPAN.clamp(MIN_SPAN, MAX_SPAN);
            self.viewport.right_ix = self.clamp_right(n + RIGHT_MARGIN, self.viewport.span);
            return;
        }
        self.viewport.span = self.viewport.span.clamp(MIN_SPAN, MAX_SPAN);
        self.viewport.right_ix = if self.is_following() {
            self.clamp_right(n + RIGHT_MARGIN, self.viewport.span)
        } else {
            self.clamp_right(self.viewport.right_ix, self.viewport.span)
        };
    }

    /// 只保留最近 `MINUTE_RETENTION` 根原始分钟。
    fn prune_minutes(&mut self) {
        let Some(&newest) = self.minutes.keys().next_back() else {
            return;
        };
        let cutoff = newest - MINUTE_RETENTION * 60_000;
        let stale = self
            .minutes
            .range(..cutoff)
            .map(|(k, _)| *k)
            .collect::<Vec<_>>();
        for key in stale {
            self.minutes.remove(&key);
        }
    }

    /// 只保留最近 `DAY_RETENTION` 根原始日线(覆盖年桶回溯到 1 月 1 日)。
    fn prune_days(&mut self) {
        let Some(&newest) = self.days.keys().next_back() else {
            return;
        };
        let cutoff = newest - DAY_RETENTION * 86_400_000;
        let stale = self
            .days
            .range(..cutoff)
            .map(|(k, _)| *k)
            .collect::<Vec<_>>();
        for key in stale {
            self.days.remove(&key);
        }
    }
}

/// 把一段原始 bar(分钟或日线)聚合成一个桶 bar(vwap 按量加权,
/// trade_count 求和)。
fn aggregate_bars<'a>(
    bars: impl Iterator<Item = (&'a i64, &'a RawBar)>,
    bucket: i64,
) -> Option<Bar> {
    let mut open = None;
    let mut high = f64::MIN;
    let mut low = f64::MAX;
    let mut close = 0.;
    let mut volume = 0i64;
    let mut vwap_sum = 0.;
    let mut trade_count = 0i64;
    for (_, raw) in bars {
        open.get_or_insert(raw.open);
        high = high.max(raw.high);
        low = low.min(raw.low);
        close = raw.close;
        volume += raw.volume;
        vwap_sum += raw.vwap * raw.volume as f64;
        trade_count += raw.trade_count;
    }
    let open = open?;
    Some(Bar {
        time_ms: bucket,
        open,
        high,
        low,
        close,
        volume,
        vwap: if volume > 0 {
            vwap_sum / volume as f64
        } else {
            0.
        },
        trade_count,
    })
}

/// 把天/周/月/年桶内的原始日线聚合成一个桶 bar(桶起点之后的连续同桶
/// 日线;遇到下一个桶即停)。
fn aggregate_days(days: &BTreeMap<i64, RawBar>, bucket: i64, timeframe: Timeframe) -> Option<Bar> {
    let same_bucket = |day_ms: i64| -> bool {
        match timeframe {
            Timeframe::OneWeek => scale::ny_week_start_ms(day_ms) == bucket,
            Timeframe::OneMonth => scale::ny_month_start_ms(day_ms) == bucket,
            _ => day_ms == bucket,
        }
    };
    let start = scale::ny_date_ms(scale::ny_date(bucket));
    let window = days
        .range(start..)
        .skip_while(|(day_ms, _)| **day_ms < bucket)
        .take_while(|(day_ms, _)| same_bucket(**day_ms));
    aggregate_bars(window, bucket)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone as _;

    fn bar(time_ms: i64, close: f64) -> Bar {
        Bar {
            time_ms,
            open: close,
            high: close,
            low: close,
            close,
            volume: 100,
            vwap: close,
            trade_count: 1,
        }
    }

    fn minute_ms(hour: u32, minute: u32) -> i64 {
        chrono::Utc
            .with_ymd_and_hms(2026, 3, 6, hour, minute, 0)
            .single()
            .unwrap()
            .timestamp_millis()
    }

    fn load(state: &mut KlineState, bars: Vec<Bar>) {
        let generation = state.begin_load();
        state.set_history(generation, bars);
    }

    #[test]
    fn set_symbol_clears_state() {
        let mut state = KlineState::new(Timeframe::OneDay);
        load(&mut state, vec![bar(minute_ms(0, 0), 10.)]);
        assert!(state.set_symbol(Some("NVDA".into())));
        assert!(!state.set_symbol(Some("NVDA".into())));
        assert!(state.bars().is_empty());
        assert!(!state.has_loaded());
        assert_eq!(state.symbol(), Some("NVDA"));
    }

    #[test]
    fn stale_history_is_discarded() {
        let mut state = KlineState::new(Timeframe::OneDay);
        let generation = state.begin_load();
        let newer = state.begin_load();
        assert!(state.set_history(newer, vec![bar(0, 10.)]));
        assert!(!state.set_history(generation, vec![bar(0, 99.)]));
        assert_eq!(state.bars()[0].close, 10.);
        assert!(!state.is_loading());
    }

    #[test]
    fn viewport_pins_latest_on_first_load() {
        let mut state = KlineState::new(Timeframe::OneDay);
        let bars: Vec<Bar> = (0..300)
            .map(|i| bar(i as i64 * 86_400_000, 10. + i as f64))
            .collect();
        load(&mut state, bars);
        let vp = state.viewport();
        assert_eq!(vp.span, DEFAULT_SPAN);
        assert_eq!(vp.right_ix, 305.);
        assert!(state.is_following());
    }

    #[test]
    fn refresh_keeps_viewport_but_tracks_growth() {
        let mut state = KlineState::new(Timeframe::OneDay);
        load(
            &mut state,
            (0..300).map(|i| bar(i as i64 * 86_400_000, 10.)).collect(),
        );
        assert!(state.pan_by(-50.));
        assert!(!state.is_following());
        let right = state.viewport().right_ix;

        load(
            &mut state,
            (0..310).map(|i| bar(i as i64 * 86_400_000, 10.)).collect(),
        );
        assert_eq!(state.viewport().right_ix, right); // 不跟随:原位
        assert!(state.pin_latest());
        assert!(state.is_following());
        assert_eq!(state.viewport().right_ix, 315.);
    }

    #[test]
    fn pan_clamps_to_bounds() {
        let mut state = KlineState::new(Timeframe::OneDay);
        load(
            &mut state,
            (0..200).map(|i| bar(i as i64 * 86_400_000, 10.)).collect(),
        );
        assert!(state.pan_by(-100_000.));
        assert_eq!(state.viewport().right_ix, state.viewport().span);
        assert!(state.pan_by(100_000.));
        assert_eq!(state.viewport().right_ix, 205.);
    }

    #[test]
    fn zoom_keeps_anchor_index() {
        let mut state = KlineState::new(Timeframe::OneDay);
        load(
            &mut state,
            (0..300).map(|i| bar(i as i64 * 86_400_000, 10.)).collect(),
        );
        let vp = state.viewport();
        let anchor_frac = 0.25;
        let anchor_ix = vp.right_ix - vp.span + anchor_frac * vp.span;

        assert!(state.zoom_at(anchor_frac, 0.5));
        let new_vp = state.viewport();
        let new_anchor = new_vp.right_ix - new_vp.span + anchor_frac * new_vp.span;
        assert!((new_anchor - anchor_ix).abs() < 0.01);
        assert_eq!(new_vp.span, DEFAULT_SPAN * 0.5);
    }

    #[test]
    fn daily_bar_replaces_same_day_appends_next_day() {
        let mut state = KlineState::new(Timeframe::OneDay);
        // 2026-11-19(美东交易日)两根盘中推送:同日替换。
        let day = |hour: u32| {
            chrono::Utc
                .with_ymd_and_hms(2026, 11, 19, hour, 0, 0)
                .single()
                .unwrap()
                .timestamp_millis()
        };
        assert!(state.apply_daily_bar(bar(day(14), 100.)));
        assert!(state.apply_daily_bar(bar(day(18), 101.)));
        assert_eq!(state.bars().len(), 1);
        assert_eq!(state.bars()[0].close, 101.);
        // 次日(11-20 05:00 UTC = 美东 11-20 00:00 冬令时)追加。
        let next = chrono::Utc
            .with_ymd_and_hms(2026, 11, 20, 5, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        assert!(state.apply_daily_bar(bar(next, 102.)));
        assert_eq!(state.bars().len(), 2);
        assert!(state.is_following());
    }

    #[test]
    fn minute_bars_aggregate_into_buckets() {
        let mut state = KlineState::new(Timeframe::FiveMin);
        load(&mut state, vec![]);

        // 10:00–10:03 三根原始分钟 → 一个 10:00 桶(美东;UTC 15:00)。
        let base = minute_ms(15, 0);
        for m in 0..3u32 {
            assert!(state.apply_minute_bar(
                base + (m as i64) * 60_000,
                100.,
                100. + m as f64,
                99.,
                100. + m as f64,
                10,
                100.,
                1,
            ));
        }
        assert_eq!(state.bars().len(), 1);
        let aggregated = &state.bars()[0];
        assert_eq!(aggregated.time_ms, base);
        assert_eq!(aggregated.open, 100.);
        assert_eq!(aggregated.high, 102.);
        assert_eq!(aggregated.close, 102.);
        assert_eq!(aggregated.volume, 30);
        assert_eq!(aggregated.vwap, 100.);

        // 10:05 落到下一个桶 → 追加。
        assert!(state.apply_minute_bar(base + 5 * 60_000, 103., 103., 102., 103., 5, 103., 1));
        assert_eq!(state.bars().len(), 2);
        // 迟到的 10:02 推送:重算 10:00 桶(与现值一致)→ 无追加。
        assert!(!state.apply_minute_bar(base + 2 * 60_000, 100., 102., 99., 102., 10, 100., 1));
        assert_eq!(state.bars().len(), 2);
    }

    #[test]
    fn updated_minute_replaces_raw_snapshot() {
        let mut state = KlineState::new(Timeframe::OneMin);
        load(&mut state, vec![]);
        let t = minute_ms(15, 0);
        assert!(state.apply_minute_bar(t, 100., 101., 99., 100., 50, 100., 3));
        // 更新推送是"该分钟的当前完整快照":整条替换,不累加量。
        assert!(state.apply_minute_bar(t, 100., 102., 99., 102., 60, 100.5, 4));
        assert_eq!(state.bars().len(), 1);
        assert_eq!(state.bars()[0].volume, 60);
        assert_eq!(state.bars()[0].high, 102.);
    }

    #[test]
    fn daily_push_ignored_on_intraday_and_vice_versa() {
        let mut state = KlineState::new(Timeframe::OneDay);
        assert!(!state.apply_minute_bar(minute_ms(15, 0), 1., 1., 1., 1., 1, 1., 1));

        let mut intraday = KlineState::new(Timeframe::FiveMin);
        assert!(!intraday.apply_daily_bar(bar(minute_ms(15, 0), 1.)));
    }

    /// 美东某日零点的日线 bar(2026-11 是冬令时,午夜 ET = 05:00 UTC)。
    fn day_bar(year: i32, month: u32, day: u32, close: f64) -> Bar {
        Bar {
            time_ms: chrono::Utc
                .with_ymd_and_hms(year, month, day, 5, 0, 0)
                .single()
                .unwrap()
                .timestamp_millis(),
            open: close - 1.,
            high: close + 1.,
            low: close - 2.,
            close,
            volume: 100,
            vwap: close,
            trade_count: 1,
        }
    }

    #[test]
    fn weekly_bar_aggregates_daily_pushes() {
        let mut state = KlineState::new(Timeframe::OneWeek);
        load(&mut state, vec![]);

        // 2026-11-16 是周一:16/17/18 三根日线聚合成一根周线。
        assert!(state.apply_daily_bar(day_bar(2026, 11, 16, 100.)));
        assert!(state.apply_daily_bar(day_bar(2026, 11, 17, 101.)));
        assert!(state.apply_daily_bar(day_bar(2026, 11, 18, 102.)));
        assert_eq!(state.bars().len(), 1);
        let week = &state.bars()[0];
        assert_eq!(week.open, 99.); // 首日 open
        assert_eq!(week.close, 102.); // 末日 close
        assert_eq!(week.high, 103.); // 各日 high 的最大
        assert_eq!(week.low, 98.); // 各日 low 的最小
        assert_eq!(week.volume, 300); // 各日 volume 求和
        assert!(state.is_following());

        // 同日重复推送(盘中快照):整日替换,周线量不翻倍。
        assert!(state.apply_daily_bar(day_bar(2026, 11, 18, 102.)));
        assert_eq!(state.bars().len(), 1);
        assert_eq!(state.bars()[0].volume, 300);

        // 下一周一(11-23)→ 追加新周桶。
        assert!(state.apply_daily_bar(day_bar(2026, 11, 23, 103.)));
        assert_eq!(state.bars().len(), 2);
    }

    #[test]
    fn monthly_bar_spans_calendar_month() {
        let mut state = KlineState::new(Timeframe::OneMonth);
        load(&mut state, vec![]);
        // 10-30(周五)与 11-02(周一):不同月 → 两根月线。
        assert!(state.apply_daily_bar(day_bar(2026, 10, 30, 100.)));
        assert!(state.apply_daily_bar(day_bar(2026, 11, 2, 101.)));
        assert_eq!(state.bars().len(), 2);
        assert_eq!(
            scale::ny_date(state.bars()[0].time_ms).to_string(),
            "2026-10-01"
        );
        assert_eq!(
            scale::ny_date(state.bars()[1].time_ms).to_string(),
            "2026-11-01"
        );
    }
}
