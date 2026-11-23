//! K 线图的状态实体:历史 bars、视口(viewport)、实时蜡烛构建。
//!
//! 数据两条线:**历史**由面板在切换标的/周期时经 Bars RPC 拉取
//! (`set_history` 整体写入);此后的**实时蜡烛全部由 trade 流逐笔画出**
//! ([`KlineState::apply_trade`] 按当前周期归桶:同桶增量、越桶开新根、
//! 迟到丢弃)。所有变更方法返回"是否发生变化",由调用方 `cx.notify()`
//! ——本模块不持有 GPUI 上下文。

use chrono::Datelike as _;

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

    /// 分钟/小时周期返回桶宽(分钟);天/周/月返回 0(实时按美东日/周/
    /// 月界归桶,见 `KlineState::bucket_start`)。
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

    /// 是否分钟/小时周期。
    pub fn is_intraday(self) -> bool {
        self.bucket_minutes() > 0
    }

    /// `now_ms` 所在桶的结束时刻(= 下一桶起点):实时蜡烛的收盘边界,
    /// 收盘倒计时按它计算。盘中 = 分钟桶结束;日/周/月 = 对应的美东
    /// 次日界/下周界/下月界。
    pub fn bucket_end_ms(self, now_ms: i64) -> i64 {
        match self {
            Self::OneDay => scale::ny_date_ms(scale::ny_date(now_ms) + chrono::Duration::days(1)),
            Self::OneWeek => scale::ny_week_start_ms(now_ms + 7 * 86_400_000),
            Self::OneMonth => {
                let date = scale::ny_date(now_ms);
                let next_first = if date.month() == 12 {
                    chrono::NaiveDate::from_ymd_opt(date.year() + 1, 1, 1)
                } else {
                    chrono::NaiveDate::from_ymd_opt(date.year(), date.month() + 1, 1)
                };
                scale::ny_date_ms(next_first.expect("first of month is a valid date"))
            }
            tf => {
                scale::bucket_start_ms(now_ms, tf.bucket_minutes())
                    + (tf.bucket_minutes() as i64) * 60_000
            }
        }
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
    /// 视口,切周期/换标的后的首次加载重置视口。历史是此后实时蜡烛的
    /// 基线:`apply_trade` 从最后一根的桶起继续增量。
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

    /// 任意周期的时间 → 桶起点:盘中按分钟桶(UTC 整除对齐);日/周/月
    /// 按美东日界/周界/月界。实时成交与历史 bar 都经它归桶。
    fn bucket_start(&self, time_ms: i64) -> i64 {
        match self.timeframe {
            Timeframe::OneWeek => scale::ny_week_start_ms(time_ms),
            Timeframe::OneMonth => scale::ny_month_start_ms(time_ms),
            Timeframe::OneDay => scale::ny_date_ms(scale::ny_date(time_ms)),
            tf => scale::bucket_start_ms(time_ms, tf.bucket_minutes()),
        }
    }

    /// 实时蜡烛构建(所有周期统一):每笔成交按当前周期归桶——
    /// **同桶**:增量合并最后一根 bar(close/high/low、volume 与
    /// trade_count 累加、vwap 量加权);**越桶**(或历史为空):开新根
    /// (首笔成交定义 open);**早于最后一根的桶**:迟到成交,丢弃
    /// (重算结果与历史一致)。volume 由成交 size 累加,不再消费 bar 推送。
    pub fn apply_trade(&mut self, time_ms: i64, price: f64, size: i64) -> bool {
        if price <= 0.0 {
            return false;
        }
        let size = size.max(0);
        let bucket = self.bucket_start(time_ms);

        let (same_bucket, later) = match self.bars.last() {
            None => (false, true),
            Some(last) => {
                let last_bucket = self.bucket_start(last.time_ms);
                (last_bucket == bucket, bucket > last_bucket)
            }
        };
        if !same_bucket && !later {
            return false;
        }

        let (volume, vwap) = match (same_bucket, self.bars.last().map(|b| (b.volume, b.vwap))) {
            (true, Some((prev_volume, prev_vwap))) if prev_volume > 0 || size > 0 => {
                let volume = prev_volume + size;
                let vwap = (prev_vwap * prev_volume as f64 + price * size as f64) / volume as f64;
                (volume, vwap)
            }
            _ => (size, price),
        };

        match self.bars.last_mut() {
            Some(last) if same_bucket => {
                last.high = last.high.max(price);
                last.low = last.low.min(price);
                last.close = price;
                last.volume = volume;
                last.vwap = vwap;
                last.trade_count += 1;
                true
            }
            _ => {
                self.bars.push(Bar {
                    time_ms: bucket,
                    open: price,
                    high: price,
                    low: price,
                    close: price,
                    volume,
                    vwap,
                    trade_count: 1,
                });
                // ⚠ 历史重载在途(pending_viewport_reset)时**不动视口**:
                // 此刻开出的实时蜡烛是过渡画面,若在这里消耗视口重置,
                // 视口会钉在"只有一根实时蜡烛"的位置上——随后 set_history
                // 走保留视野分支,结果显示最老的历史(2026-11-24 实测)。
                // pending 留给 set_history 钉最新。
                if !self.pending_viewport_reset {
                    if self.bars.len() == 1 {
                        self.rebuild_viewport();
                    } else {
                        self.trim_viewport();
                    }
                }
                true
            }
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

    /// 盘中蜡烛由成交逐笔构建:同桶增量(high/low/close/volume/vwap/
    /// count),下一桶开新根,迟到成交丢弃。
    #[test]
    fn trades_build_and_roll_intraday_buckets() {
        let mut state = KlineState::new(Timeframe::FiveMin);
        let base = minute_ms(15, 0); // UTC 15:00 = 美东 10:00(冬令时),5Min 桶起点

        assert!(state.apply_trade(base, 100., 10));
        assert!(state.apply_trade(base + 60_000, 102., 10));
        assert!(state.apply_trade(base + 120_000, 99., 10));
        assert_eq!(state.bars().len(), 1);
        let b = &state.bars()[0];
        assert_eq!(b.time_ms, base);
        assert_eq!(b.open, 100.);
        assert_eq!(b.high, 102.);
        assert_eq!(b.low, 99.);
        assert_eq!(b.close, 99.);
        assert_eq!(b.volume, 30);
        assert_eq!(b.vwap, (100. * 10. + 102. * 10. + 99. * 10.) / 30.);
        assert_eq!(b.trade_count, 3);

        // 下一桶 → 新根。
        assert!(state.apply_trade(base + 5 * 60_000, 103., 5));
        assert_eq!(state.bars().len(), 2);

        // 迟到成交(旧桶)→ 丢弃,不影响已画蜡烛。
        assert!(!state.apply_trade(base + 60_000, 999., 10));
        assert_eq!(state.bars()[0].high, 102.);
        assert_eq!(state.bars().len(), 2);
    }

    /// 空历史(无标的/无数据)的首笔成交开出第一根。
    #[test]
    fn first_trade_creates_the_first_bar() {
        let mut state = KlineState::new(Timeframe::FiveMin);
        assert!(state.apply_trade(minute_ms(15, 0), 100., 10));
        assert_eq!(state.bars().len(), 1);
        assert_eq!(state.bars()[0].open, 100.);
        assert!(state.is_following());
    }

    /// 换标的后、历史加载在途时先到的成交,不得消耗视口重置——否则
    /// set_history 走保留视野分支,显示最老的历史(实测 bug)。
    #[test]
    fn trade_during_history_load_keeps_pending_reset() {
        let mut state = KlineState::new(Timeframe::OneDay);
        state.set_symbol(Some("NVDA".into()));
        let generation = state.begin_load();

        // 历史在途:先到一笔成交(空历史 → 开根),不动视口。
        assert!(state.apply_trade(minute_ms(15, 0), 100., 10));

        let bars: Vec<Bar> = (0..300)
            .map(|i| bar(i as i64 * 86_400_000, 10. + i as f64))
            .collect();
        assert!(state.set_history(generation, bars));

        // 钉在最新:显示最后 DEFAULT_SPAN 根,右缘 = 最新 + 留白。
        let vp = state.viewport();
        assert_eq!(vp.right_ix, 305.);
        assert!(state.is_following());
    }

    /// 桶收盘边界:盘中 = 分钟桶结束;1D = 美东次日零点(DST 由时区库
    /// 换算消化)。
    #[test]
    fn bucket_end_is_the_next_boundary() {
        let tf = Timeframe::FiveMin;
        // 15:02:30 → 桶 15:00,结束 15:05。
        assert_eq!(
            tf.bucket_end_ms(minute_ms(15, 2) + 30_000),
            minute_ms(15, 5)
        );

        // 1D:11-20 15:00 UTC → 美东次日零点(冬令时 = UTC 05:00)。
        let day = Timeframe::OneDay;
        let now = chrono::Utc
            .with_ymd_and_hms(2026, 11, 20, 15, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        let next_midnight = chrono::Utc
            .with_ymd_and_hms(2026, 11, 21, 5, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        assert_eq!(day.bucket_end_ms(now), next_midnight);
    }

    /// 1D 按美东日界归桶:同日增量、次日开新根;周/月同理按周界/月界。
    #[test]
    fn trades_build_daily_and_weekly_buckets() {
        // 1D:11-19(周四)同日两笔 → 一根;11-20 → 新根。
        let mut day = KlineState::new(Timeframe::OneDay);
        let thu = chrono::Utc
            .with_ymd_and_hms(2026, 11, 19, 15, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        let fri = chrono::Utc
            .with_ymd_and_hms(2026, 11, 20, 15, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        assert!(day.apply_trade(thu, 100., 10));
        assert!(day.apply_trade(thu + 60_000, 101., 10));
        assert_eq!(day.bars().len(), 1);
        assert_eq!(day.bars()[0].close, 101.);
        assert!(day.apply_trade(fri, 102., 10));
        assert_eq!(day.bars().len(), 2);

        // 1W:11-16(周一)与 11-17(周二)同周 → 一根;11-23(周一)→ 新根。
        let mut week = KlineState::new(Timeframe::OneWeek);
        let mon = chrono::Utc
            .with_ymd_and_hms(2026, 11, 16, 15, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        let tue = chrono::Utc
            .with_ymd_and_hms(2026, 11, 17, 15, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        let next_mon = chrono::Utc
            .with_ymd_and_hms(2026, 11, 23, 15, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        assert!(week.apply_trade(mon, 100., 10));
        assert!(week.apply_trade(tue, 101., 10));
        assert_eq!(week.bars().len(), 1);
        assert_eq!(week.bars()[0].close, 101.);
        assert!(week.apply_trade(next_mon, 102., 10));
        assert_eq!(week.bars().len(), 2);
    }

    /// 历史基线上的增量:成交量在最后一根 bar 上累加(不与历史重复计量,
    /// 基线是拉取时刻的聚合,其后的成交都是新的);落在更早桶的迟到成交
    /// 被丢弃。
    #[test]
    fn trades_increment_on_top_of_history() {
        let mut state = KlineState::new(Timeframe::FiveMin);
        let base = minute_ms(15, 0);
        let mut history = vec![bar(base, 100.), bar(base + 300_000, 101.)];
        history[1].volume = 30;
        load(&mut state, history);

        // 同桶(末根 15:05 桶内)增量。
        assert!(state.apply_trade(base + 300_000 + 10_000, 99., 20));
        assert_eq!(state.bars().len(), 2);
        assert_eq!(state.bars()[1].volume, 50);
        assert_eq!(state.bars()[1].low, 99.);
        assert_eq!(state.bars()[0].close, 100.);

        // 迟到成交(第一根的桶)→ 丢弃。
        assert!(!state.apply_trade(base + 10_000, 999., 10));
        assert_eq!(state.bars()[1].low, 99.);
    }
}
