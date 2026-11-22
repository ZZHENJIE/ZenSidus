//! K 线图的自定义绘制元素:蜡烛 + 成交量双区、右侧价格轴、底部时间轴、
//! 最新价虚线、十字光标与 OHLCV 图例,以及指针交互(拖拽平移、滚轮缩放、
//! 双击回最新)。
//!
//! 这是一个填充父容器的自定义 Element(三阶段:request_layout 占满、
//! prepaint 建 hitbox、paint 绘制 + 注册窗口级鼠标监听);交互回调一律
//! 经 hitbox 过滤,状态变更 notify 状态实体(面板 render 依赖它,自动
//! 重绘),十字光标作为元素局部状态只 notify 本视图——与 gpui-base 的
//! PlotElement 同一套模式。
//!
//! 画布度量(轴宽、字号等)是设计指南允许的例外:它们是绘图画布自身的
//! 几何,不参与 rem 缩放;文本每帧即席塑形,不存在跨帧缓存需要失效。

use std::{cell::Cell, rc::Rc};

use chrono::Datelike as _;
use gpui_kit::{
    App, BorderStyle, Bounds, ContentMask, Edges, Element, ElementId, GlobalElementId, Hsla,
    InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Point, ScrollDelta, ScrollEvent, ShapedLine, SharedString, Size, Style,
    TextAlign, TextRun, Window, Entity, component::ActiveTheme as _, fill, point, px, quad, size,
    transparent_black,
};

use super::{
    scale,
    state::{KlineState, Timeframe, Viewport},
};

// ── 画布度量 ─────────────────────────────────────────────────────────────

/// 底部时间轴高度。
const TIME_AXIS_H: f32 = 24.;
/// 成交量副图占绘图区高度的比例(价格区拿其余部分)。
const VOL_SHARE: f32 = 0.22;
/// 轴与图例的字号(px)。
const AXIS_FONT_SIZE: f32 = 11.;
const LEGEND_FONT_SIZE: f32 = 12.;
/// 文本行高(同字号的比例系数)。
const LINE_HEIGHT: f32 = 1.3;
/// 时间轴相邻标签的最小间距(选步长的输入)。
const TIME_LABEL_MIN_GAP: f32 = 80.;
/// 价格刻度的目标数量。
const PRICE_TICK_TARGET: usize = 5;
/// 图例/轴文本的内边距。
const TEXT_PAD: f32 = 8.;
/// 蜡烛实体的最大宽度(桶很宽时不无限膨胀)。
const MAX_BODY_W: f32 = 24.;

/// K 线图元素。填充父容器;数据与视口都读自状态实体。
pub struct Kline {
    state: Entity<KlineState>,
    /// 滚轮缩放开关:嵌入可滚动列表(筛选器图表)时关闭——滚轮让位给
    /// 列表滚动,图表只保留拖拽平移与十字光标。
    wheel_zoom: bool,
    id: ElementId,
}

impl Kline {
    pub fn new(state: Entity<KlineState>) -> Self {
        Self {
            state,
            wheel_zoom: true,
            id: "kline".into(),
        }
    }

    /// 滚轮是否缩放(默认开;嵌入可滚动列表时关闭)。
    pub fn wheel_zoom(mut self, enabled: bool) -> Self {
        self.wheel_zoom = enabled;
        self
    }

    /// 元素 id:同屏多个实例必须彼此不同,否则元素局部状态(光标/拖拽)
    /// 互相串。
    pub fn with_id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = id.into();
        self
    }
}

impl IntoElement for Kline {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

/// 元素局部状态(生命周期跟随元素键):十字光标位置与拖拽锚点。都是
/// 元素相对坐标;`None` = 未悬停 / 未拖拽。十字光标是纯呈现,不进状态实体。
#[derive(Clone, Default)]
struct InteractState {
    cursor: Rc<Cell<Option<Point<Pixels>>>>,
    drag: Rc<Cell<Option<Point<Pixels>>>>,
}

impl Element for Kline {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let style = Style {
            size: Size::full(),
            ..Default::default()
        };
        (window.request_layout(style, None, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Window,
        _: &mut App,
    ) {}

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        // 主题语义色先拷出(绘制过程中还要 &mut cx)。
        let theme = cx.theme();
        let palette = Palette {
            background: theme.background,
            border: theme.border,
            grid: theme.chart_grid,
            muted_text: theme.muted_foreground,
            text: theme.foreground,
            secondary: theme.secondary,
            secondary_text: theme.secondary_foreground,
            bullish: theme.chart_bullish,
            bearish: theme.chart_bearish,
        };

        // 状态快照:viewport/方向/最新价都是 Copy,可见区间 bar 拷成帧数据,
        // 之后的绘制与交互注册不再持有 cx 借用。
        let state = self.state.read(cx);
        let frame = FrameData::snapshot(state, &state.viewport());
        let viewport = state.viewport();
        let symbol = state.symbol().map(str::to_owned);

        window.paint_quad(fill(bounds, palette.background));

        // 元素局部交互状态(光标/拖拽),先取再用:绘制阶段的十字光标位置
        // 来自它,之后的鼠标监听也挂在它上面。
        let interact = global_id.map(|global_id| {
            window.with_element_state(global_id, |prev, _| {
                let state: InteractState = prev.unwrap_or_default();
                (state.clone(), state)
            })
        });

        // 布局可能把图移到静止的光标下而没有 move 事件:每帧按鼠标位置重推,
        // 只有可见性翻转才补一帧(PlotElement 同款)。判定用元素自身 bounds
        // 而不是 hitbox 的遮挡感知 hit-test(dock/悬浮层里 hit-test 会瞬间
        // 报 false,徒增冻结路径);cell 存元素相对坐标,消费方见
        // `paint_crosshair` 的坐标语义注释。
        let crosshair = interact.as_ref().map(|interact| {
            let mouse = window.mouse_position();
            let next = bounds
                .contains(&mouse)
                .then(|| mouse - bounds.origin);
            if interact.cursor.get() != next {
                let visibility_flipped = interact.cursor.get().is_some() != next.is_some();
                interact.cursor.set(next);
                if visibility_flipped {
                    window.request_animation_frame();
                }
            }
            next
        });

        // 价格轴宽按标签实宽量出(先算刻度再量宽,最后定绘图区)。
        let scale = compute_price_scale(&frame);
        let axis_w = measure_axis_width(&scale, frame.last_close, window);
        let mut geometry = Geometry::new(bounds, viewport, axis_w, scale, &frame);
        geometry.paint(
            &frame,
            &palette,
            symbol.as_deref(),
            crosshair.flatten(),
            window,
            cx,
        );

        let Some(interact) = interact else {
            return;
        };
        // `current_view` 只在 paint 阶段合法:光标回调要用的 view id 这里取好。
        let view = window.current_view();
        let body_w = geometry.body_w;
        let plot_w = geometry.plot_w();
        // 交互回调改写状态实体(注意:上面的 `state` 是 read 的借用,这里
        // 必须换成本体克隆)。变更后 `cx.notify()` 打在状态实体上:面板的
        // render 读过它,GPUI 的依赖追踪会自动重绘该视图。
        let state = self.state.clone();

        // 每个监听闭包持有一份克隆(Rc/Entity 克隆都很廉价)。
        let (cursor, drag) = (interact.cursor, interact.drag);
        window.on_mouse_event({
            let (cursor, drag, state) = (cursor.clone(), drag.clone(), state.clone());
            move |e: &MouseMoveEvent, phase, _: &mut Window, cx: &mut App| {
                if !phase.bubble() {
                    return;
                }
                // 拖拽中即使移出图表也继续平移。
                if let Some(last) = drag
                    .get()
                    .filter(|_| e.pressed_button == Some(MouseButton::Left))
                {
                    let dx = e.position.x.as_f32() - last.x.as_f32();
                    drag.set(Some(e.position));
                    if dx != 0. {
                        state.update(cx, |state, cx| {
                            if state.pan_by(-dx / body_w) {
                                cx.notify();
                            }
                        });
                    }
                }
                let next = bounds
                    .contains(&e.position)
                    .then(|| e.position - bounds.origin);
                if cursor.get() != next {
                    cursor.set(next);
                    // 光标是元素局部状态,不经实体:只重绘本视图。
                    cx.notify(view);
                }
            }
        });

        window.on_mouse_event({
            let (drag, state) = (drag.clone(), state.clone());
            move |e: &MouseDownEvent, phase, _: &mut Window, cx: &mut App| {
                if !phase.bubble() || e.button != MouseButton::Left {
                    return;
                }
                if !bounds.contains(&e.position) {
                    return;
                }
                // 双击回到最新。
                if e.click_count >= 2 {
                    state.update(cx, |state, cx| {
                        if state.pin_latest() {
                            cx.notify();
                        }
                    });
                }
                drag.set(Some(e.position));
            }
        });

        window.on_mouse_event({
            let drag = drag.clone();
            move |e: &MouseUpEvent, phase, _: &mut Window, _: &mut App| {
                if !phase.bubble() || e.button != MouseButton::Left {
                    return;
                }
                drag.set(None);
            }
        });

        // 滚轮缩放仅在 wheel_zoom 开启时注册:嵌入可滚动列表(筛选器
        // 图表)时关闭,滚轮事件放行给列表滚动。
        if self.wheel_zoom {
            window.on_mouse_event({
                let state = state.clone();
                move |e: &ScrollEvent, phase, window: &mut Window, cx: &mut App| {
                    if !phase.bubble() || !bounds.contains(&e.position) {
                        return;
                    }
                    let dy = match e.delta {
                        ScrollDelta::Pixels(delta) => delta.y.as_f32(),
                        ScrollDelta::Lines(lines) => lines.y * 24.,
                    };
                    if dy == 0. {
                        return;
                    }
                    // 向上滚 = 放大(span 变小);指数系数决定缩放手感。
                    let factor = (-dy * 0.0016).exp().clamp(0.2, 5.);
                    let anchor =
                        ((e.position.x.as_f32() - bounds.origin.x.as_f32()) / plot_w).clamp(0., 1.);
                    state.update(cx, |state, cx| {
                        if state.zoom_at(anchor, factor) {
                            cx.notify();
                        }
                    });
                    // 滚轮只服务于缩放:挡住向父容器的滚动冒泡。
                    window.prevent_default();
                }
            });
        }
    }
}

// ── 帧数据 ───────────────────────────────────────────────────────────────

/// 一帧内绘制的单根 bar 快照(从实体拷出,paint 不再持有 cx 借用)。
#[derive(Debug, Clone, Copy)]
struct SnapshotBar {
    time_ms: i64,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: i64,
}

/// 十字光标的一次吸附结果(线与签共用)。
struct CrosshairState {
    /// 竖线 x(吸附到 bar 中心,窗口绝对坐标)。
    snap_x: f32,
    /// 横线 y(窗口绝对坐标)。
    y: f32,
    /// 吸附的 bar(空图/越界为 None)。
    bar: Option<SnapshotBar>,
}

/// 一帧的绘制输入:可见区间 bar(带绝对 index)、最新一根(图例兜底,
/// 可能不在可见区间内)、最新收盘(最新价线)、周期与总数。
struct FrameData {
    bars: Vec<SnapshotBar>,
    first_ix: usize,
    total: usize,
    latest: Option<SnapshotBar>,
    last_close: Option<f64>,
    timeframe: Timeframe,
}

impl FrameData {
    fn snapshot(state: &KlineState, viewport: &Viewport) -> Self {
        let bars_all = state.bars();
        let total = bars_all.len();
        let latest = bars_all.last().map(|bar| SnapshotBar {
            time_ms: bar.time_ms,
            open: bar.open,
            high: bar.high,
            low: bar.low,
            close: bar.close,
            volume: bar.volume,
        });
        let first_ix = if total > 0 {
            ((viewport.right_ix - viewport.span).floor().max(0.) as usize).min(total - 1)
        } else {
            0
        };
        let last_ix = if total > 0 {
            (viewport.right_ix.floor() as usize).min(total - 1)
        } else {
            0
        };
        let bars = if total > 0 && first_ix <= last_ix {
            bars_all[first_ix..=last_ix]
                .iter()
                .map(|bar| SnapshotBar {
                    time_ms: bar.time_ms,
                    open: bar.open,
                    high: bar.high,
                    low: bar.low,
                    close: bar.close,
                    volume: bar.volume,
                })
                .collect()
        } else {
            Vec::new()
        };
        Self {
            bars,
            first_ix,
            total,
            latest,
            last_close: state.last_close(),
            timeframe: state.timeframe(),
        }
    }

    /// 绝对 index → 可见快照。
    fn bar_at(&self, ix: usize) -> Option<SnapshotBar> {
        ix.checked_sub(self.first_ix)
            .and_then(|offset| self.bars.get(offset))
            .copied()
    }
}

/// 本帧用到的主题语义色(拷贝自 `cx.theme()`)。
#[derive(Debug, Clone, Copy)]
struct Palette {
    background: Hsla,
    border: Hsla,
    /// 网格线(主题的 `chart_grid` token,应用主题已定制)。
    grid: Hsla,
    muted_text: Hsla,
    text: Hsla,
    secondary: Hsla,
    secondary_text: Hsla,
    bullish: Hsla,
    bearish: Hsla,
}

impl Palette {
    /// 涨跌方向色(收盘 ≥ 开盘 = 阳线)。
    fn direction(&self, open: f64, close: f64) -> Hsla {
        if close >= open {
            self.bullish
        } else {
            self.bearish
        }
    }

    /// 半透明方向色(成交量柱)。
    fn volume(&self, open: f64, close: f64) -> Hsla {
        self.direction(open, close).opacity(0.55)
    }
}

// ── 几何 ─────────────────────────────────────────────────────────────────

/// 价格轴刻度:区间(含 8% 上下留白)、步长与刻度值。
struct PriceScale {
    min: f64,
    max: f64,
    step: f64,
    ticks: Vec<f64>,
}

/// 由可见 bar 推价格区间与刻度(不依赖像素;轴宽随后按标签实宽量出)。
fn compute_price_scale(frame: &FrameData) -> PriceScale {
    let (mut lo, mut hi) = frame
        .bars
        .first()
        .map(|bar| (bar.low, bar.high))
        .unwrap_or((0., 1.));
    for bar in &frame.bars {
        lo = lo.min(bar.low);
        hi = hi.max(bar.high);
    }
    if hi <= lo {
        // 平盘兜底:围绕现价 ±1%。
        let pad = hi.abs() * 0.01 + 1.;
        lo -= pad;
        hi += pad;
    }
    let pad = (hi - lo) * 0.08;
    let min = lo - pad;
    let max = hi + pad;
    let step = scale::price_step(max - min, PRICE_TICK_TARGET);
    PriceScale {
        min,
        max,
        step,
        ticks: scale::price_ticks(min, max, PRICE_TICK_TARGET),
    }
}

/// 价格轴宽 = 最宽标签(刻度 + 极值/最新价的图例格式)+ 两侧内边距,
/// 下限 24px。轴随内容自适应,不再用固定宽度(实测 72px 对短价格太空)。
fn measure_axis_width(scale: &PriceScale, last_close: Option<f64>, window: &mut Window) -> f32 {
    let mut max_w = 0f32;
    let mut measure = |text: String, max_w: &mut f32| {
        let line = shape_line(text.into(), AXIS_FONT_SIZE, Hsla::default(), window);
        *max_w = max_w.max(line.width().as_f32());
    };
    for tick in &scale.ticks {
        measure(scale::format_price(*tick, scale.step), &mut max_w);
    }
    measure(scale::format_quote(scale.min), &mut max_w);
    measure(scale::format_quote(scale.max), &mut max_w);
    if let Some(close) = last_close {
        measure(scale::format_quote(close), &mut max_w);
    }
    (max_w + TEXT_PAD * 2.).max(24.)
}

/// 画布几何:分区、坐标映射、刻度。paint 前一次性构造。
struct Geometry {
    bounds: Bounds<Pixels>,
    /// 价格 + 成交量绘图区(不含右侧价格轴与底部时间轴)。
    plot: Bounds<Pixels>,
    price_h: f32,
    vol_h: f32,
    body_w: f32,
    start_ix: f32,
    /// 右侧价格轴宽(按标签实宽量出)。
    axis_w: f32,
    /// 价格区间与刻度步长。
    scale: PriceScale,
    vol_max: f64,
}

impl Geometry {
    fn new(
        bounds: Bounds<Pixels>,
        viewport: Viewport,
        axis_w: f32,
        scale: PriceScale,
        frame: &FrameData,
    ) -> Self {
        let plot = Bounds::new(
            bounds.origin,
            size(
                px((bounds.size.width.as_f32() - axis_w).max(0.)),
                px((bounds.size.height.as_f32() - TIME_AXIS_H).max(0.)),
            ),
        );
        let plot_h = plot.size.height.as_f32();
        let price_h = plot_h * (1. - VOL_SHARE);
        let vol_h = plot_h - price_h;
        let body_w = if viewport.span > 0. {
            plot.size.width.as_f32() / viewport.span
        } else {
            0.
        };
        let vol_max = frame
            .bars
            .iter()
            .map(|bar| bar.volume)
            .max()
            .unwrap_or(1)
            .max(1) as f64;
        Self {
            bounds,
            plot,
            price_h,
            vol_h,
            body_w,
            start_ix: viewport.right_ix - viewport.span,
            axis_w,
            scale,
            vol_max,
        }
    }

    fn plot_w(&self) -> f32 {
        self.plot.size.width.as_f32()
    }

    fn plot_h(&self) -> f32 {
        self.plot.size.height.as_f32()
    }

    /// 绝对 bar index(连续)→ 绘图区 x 中心。
    fn x_center(&self, ix: f32) -> f32 {
        self.plot.origin.x.as_f32() + (ix - self.start_ix + 0.5) * self.body_w
    }

    /// 绘图区 x → 连续 bar index(光标吸附时由调用方 round)。
    fn ix_at(&self, x: f32) -> f32 {
        self.start_ix + (x - self.plot.origin.x.as_f32()) / self.body_w - 0.5
    }

    /// 价格 → 价格区 y。
    fn y_price(&self, price: f64) -> f32 {
        let range = (self.scale.max - self.scale.min).max(f64::EPSILON);
        let offset = ((self.scale.max - price) / range) * self.price_h as f64;
        self.plot.origin.y.as_f32() + offset as f32
    }

    /// 价格区 y → 价格。
    fn price_at(&self, y: f32) -> f64 {
        let price_h = self.price_h.max(1.);
        let frac = (y - self.plot.origin.y.as_f32()) / price_h;
        self.scale.max - frac as f64 * (self.scale.max - self.scale.min)
    }

    /// 成交量 → 副图柱顶 y(底部 2px 余量)。
    fn y_volume(&self, volume: i64) -> f32 {
        let vol_bottom = self.plot.origin.y.as_f32() + self.plot_h();
        let height = (volume as f64 / self.vol_max.max(f64::EPSILON)) * (self.vol_h - 2.) as f64;
        vol_bottom - height as f32
    }

    /// 蜡烛/量柱的 x 与宽(最小 1px,最大 [`MAX_BODY_W`],占桶宽 70%)。
    fn bar_rect(&self, ix: f32) -> (f32, f32) {
        let width = (self.body_w * 0.7).clamp(1., MAX_BODY_W);
        (self.x_center(ix) - width * 0.5, width)
    }

    /// 可见范围内的时间刻度(可见序 + 绝对序):步长保证标签间距,且按
    /// 绝对 index 对齐,平移时刻度稳定不闪。
    fn time_ticks(&self, frame: &FrameData) -> Vec<(usize, usize)> {
        let step = scale::time_tick_step(self.body_w, TIME_LABEL_MIN_GAP);
        (0..frame.bars.len())
            .filter(|offset| (frame.first_ix + *offset).is_multiple_of(step))
            .map(|offset| (offset, frame.first_ix + offset))
            .collect()
    }
}

// ── 绘制 ─────────────────────────────────────────────────────────────────

impl Geometry {
    fn paint(
        &mut self,
        frame: &FrameData,
        palette: &Palette,
        symbol: Option<&str>,
        cursor: Option<Point<Pixels>>,
        window: &mut Window,
        cx: &mut App,
    ) {
        // 空图(无标的/无数据)只画分隔线:退化刻度没有信息量,空态提示
        // 由面板的 overlay 负责。
        if frame.total == 0 {
            self.paint_separators(palette, window);
            return;
        }

        // 绘图区过小(虚拟列表首帧按测量宽度布局、dock 挤压等):只画底色。
        // 退化宽度下轴刻度/时间签的 clamp 区间会反转,直接绘制必 panic。
        if self.plot_w() < 60. || self.plot_h() < 60. {
            return;
        }

        // 十字光标状态一次算好:线画在绘图区(裁剪),签画在轴区(不裁)。
        let crosshair = self.crosshair_state(frame, cursor);

        // 绘图区内容裁剪在 plot 内:缩放/平移后 right_ix 带小数,可见最后
        // 一根的中心会越过轴界,不裁就把蜡烛画到价格轴上(实测踩过)。
        window.with_content_mask(Some(ContentMask { bounds: self.plot }), |window| {
            self.paint_grid(frame, palette, window);
            self.paint_volume(frame, palette, window);
            self.paint_candles(frame, palette, window);
            self.paint_last_price_line(frame, palette, window);
            if let Some(state) = &crosshair {
                self.paint_crosshair_lines(state, palette, window);
            }
        });

        self.paint_separators(palette, window);
        if let Some(state) = &crosshair {
            self.paint_crosshair_tags(state, frame, palette, window, cx);
        }
        self.paint_last_price_tag(frame, palette, window, cx);
        self.paint_axes(frame, palette, window, cx);
        self.paint_legend(
            frame,
            palette,
            symbol,
            crosshair.as_ref().and_then(|state| state.bar),
            window,
            cx,
        );
    }

    fn paint_grid(&self, frame: &FrameData, palette: &Palette, window: &mut Window) {
        let grid = palette.grid;
        for &tick in &self.scale.ticks {
            let y = self.y_price(tick);
            if y <= self.plot.origin.y.as_f32() || y >= self.plot.origin.y.as_f32() + self.price_h
            {
                continue;
            }
            self.paint_h_line(window, y, self.plot.origin.x.as_f32(), self.plot_w(), grid, false);
        }
        for (_, abs_ix) in self.time_ticks(frame) {
            let x = self.x_center(abs_ix as f32);
            if x < self.plot.origin.x.as_f32() || x > self.plot.origin.x.as_f32() + self.plot_w() {
                continue;
            }
            self.paint_v_line(window, x, self.plot.origin.y.as_f32(), self.plot_h(), grid);
        }
    }

    fn paint_volume(&self, frame: &FrameData, palette: &Palette, window: &mut Window) {
        let vol_bottom = self.plot.origin.y.as_f32() + self.plot_h();
        for (offset, bar) in frame.bars.iter().enumerate() {
            let color = palette.volume(bar.open, bar.close);
            let (x, w) = self.bar_rect(offset as f32 + frame.first_ix as f32);
            let top = self.y_volume(bar.volume);
            window.paint_quad(fill(
                Bounds::new(
                    point(px(x), px(top)),
                    size(px(w), px((vol_bottom - top).max(1.))),
                ),
                color,
            ));
        }
    }

    fn paint_candles(&self, frame: &FrameData, palette: &Palette, window: &mut Window) {
        for (offset, bar) in frame.bars.iter().enumerate() {
            let ix = offset as f32 + frame.first_ix as f32;
            let color = palette.direction(bar.open, bar.close);
            let (x, w) = self.bar_rect(ix);
            let x_center = x + w * 0.5;
            // 影线(1px,先画,压在实体之下)。
            let wick_top = self.y_price(bar.high);
            let wick_bottom = self.y_price(bar.low);
            window.paint_quad(fill(
                Bounds::new(
                    point(px(x_center - 0.5), px(wick_top)),
                    size(px(1.), px((wick_bottom - wick_top).max(1.))),
                ),
                color,
            ));
            // 实体:十字星(开=收)保 1px 高度可见。
            let top = self.y_price(bar.open.max(bar.close));
            let height = (self.y_price(bar.open.min(bar.close)) - top).max(1.);
            window.paint_quad(fill(
                Bounds::new(point(px(x), px(top)), size(px(w), px(height))),
                color,
            ));
        }
    }

    fn paint_separators(&self, palette: &Palette, window: &mut Window) {
        let border = palette.border;
        let origin_x = self.bounds.origin.x.as_f32();
        let origin_y = self.bounds.origin.y.as_f32();
        // 右轴分隔线。
        self.paint_v_line(
            window,
            origin_x + self.plot_w(),
            origin_y,
            self.bounds.size.height.as_f32(),
            border,
        );
        // 时间轴分隔线。
        self.paint_h_line(
            window,
            origin_y + self.plot_h(),
            origin_x,
            self.bounds.size.width.as_f32(),
            border,
            false,
        );
    }

    /// 最新价虚线(裁剪在绘图区内)。
    fn paint_last_price_line(&self, frame: &FrameData, palette: &Palette, window: &mut Window) {
        let (Some(latest), Some(close)) = (frame.latest, frame.last_close) else {
            return;
        };
        let y = self.y_price(close);
        if y <= self.plot.origin.y.as_f32() || y >= self.plot.origin.y.as_f32() + self.price_h {
            return;
        }
        let color = palette.direction(latest.open, latest.close);
        self.paint_h_line(window, y, self.plot.origin.x.as_f32(), self.plot_w(), color, true);
    }

    /// 右轴上的最新价价签(方向色底、面板底色字;不裁剪)。
    fn paint_last_price_tag(
        &self,
        frame: &FrameData,
        palette: &Palette,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (Some(_latest), Some(close)) = (frame.latest, frame.last_close) else {
            return;
        };
        let y = self.y_price(close);
        if y <= self.plot.origin.y.as_f32() || y >= self.plot.origin.y.as_f32() + self.price_h {
            return;
        }
        let color = palette.direction(
            frame.latest.expect("checked above").open,
            frame.latest.expect("checked above").close,
        );
        Tag {
            bounds: Bounds::new(
                point(
                    px(self.bounds.origin.x.as_f32() + self.plot_w() + 2.),
                    px(y - AXIS_FONT_SIZE * 0.8),
                ),
                size(px(self.axis_w - 4.), px(AXIS_FONT_SIZE * 1.6)),
            ),
            background: color,
            text_color: palette.background,
            text: scale::format_quote(close),
        }
        .paint(window, cx);
    }

    fn paint_axes(&self, frame: &FrameData, palette: &Palette, window: &mut Window, cx: &mut App) {
        let text_color = palette.muted_text;
        // 价格轴标签(略过贴边刻度,避免压轴)。
        for tick in &self.scale.ticks {
            let y = self.y_price(*tick);
            if y < self.plot.origin.y.as_f32() + AXIS_FONT_SIZE
                || y > self.plot.origin.y.as_f32() + self.price_h - 2.
            {
                continue;
            }
            let label = shape_line(
                scale::format_price(*tick, self.scale.step).into(),
                AXIS_FONT_SIZE,
                text_color,
                window,
            );
            let _ = label.paint(
                point(
                    px(self.bounds.origin.x.as_f32() + self.plot_w() + TEXT_PAD),
                    px(y - AXIS_FONT_SIZE * 0.65),
                ),
                px(AXIS_FONT_SIZE * LINE_HEIGHT),
                TextAlign::Left,
                None,
                window,
                cx,
            );
        }
        // 时间轴标签:日内 "HH:mm",跨日刻度换 "MM-dd";日线 "MM-dd",
        // 新年首刻度换 "yyyy-MM-dd"。
        let mut prev_date: Option<chrono::NaiveDate> = None;
        let mut prev_year: Option<i32> = None;
        let label_y = self.plot.origin.y.as_f32() + self.plot_h() + 4.;
        for (_, abs_ix) in self.time_ticks(frame) {
            let Some(bar) = frame.bar_at(abs_ix) else {
                continue;
            };
            let label = if frame.timeframe == Timeframe::OneDay {
                let year = scale::ny_date(bar.time_ms).year();
                let text = if prev_year.is_some_and(|prev| prev != year) {
                    scale::format_ny_date(bar.time_ms)
                } else {
                    scale::format_ny_md(bar.time_ms)
                };
                prev_year = Some(year);
                text
            } else {
                let date = scale::ny_date(bar.time_ms);
                let text = if prev_date.is_some_and(|prev| prev != date) {
                    scale::format_ny_md(bar.time_ms)
                } else {
                    scale::format_ny_hm(bar.time_ms)
                };
                prev_date = Some(date);
                text
            };
            // clamp 区间可能退化(轴窄于标签):先把上界抬到下界之上。
            let min_x = self.bounds.origin.x.as_f32() + 28.;
            let max_x = (self.bounds.origin.x.as_f32() + self.plot_w() - 28.).max(min_x);
            let x = self.x_center(abs_ix as f32).clamp(min_x, max_x);
            let line = shape_line(label.into(), AXIS_FONT_SIZE, text_color, window);
            let _ = line.paint(
                point(px(x - 30.), px(label_y)),
                px(AXIS_FONT_SIZE * LINE_HEIGHT),
                TextAlign::Center,
                Some(px(60.)),
                window,
                cx,
            );
        }
    }

    /// 十字光标的状态:吸附后的竖线 x、横线 y 与吸附的 bar。
    ///
    /// `cursor` 是元素相对坐标(cell 的存储语义);下面所有几何辅助
    /// (`ix_at`/`price_at`/`x_center`/plot 边界)都是窗口绝对坐标语义
    /// (内部以 `bounds.origin` 为基准),所以入口先转换——面板原点非零时
    /// 不转换就会双减,十字线偏到光标左上方(实测踩过;origin=(0,0) 的
    /// 裸窗口会掩盖此 bug)。
    fn crosshair_state(
        &self,
        frame: &FrameData,
        cursor: Option<Point<Pixels>>,
    ) -> Option<CrosshairState> {
        let cursor = cursor? + self.bounds.origin;
        let x = cursor.x.as_f32();
        let y = cursor.y.as_f32();
        let plot_left = self.plot.origin.x.as_f32();
        let plot_top = self.plot.origin.y.as_f32();
        let plot_right = plot_left + self.plot_w();
        let plot_bottom = plot_top + self.plot_h();
        if x < plot_left || x > plot_right || y < plot_top || y > plot_bottom || frame.total == 0 {
            return None;
        }

        // 竖线吸附到 bar 中心。
        let ix = self.ix_at(x).round().clamp(0., (frame.total - 1) as f32);
        Some(CrosshairState {
            snap_x: self.x_center(ix),
            y,
            bar: frame.bar_at(ix as usize),
        })
    }

    /// 十字线(裁剪在绘图区内)。
    fn paint_crosshair_lines(&self, state: &CrosshairState, palette: &Palette, window: &mut Window) {
        let plot_top = self.plot.origin.y.as_f32();
        let plot_left = self.plot.origin.x.as_f32();
        let cross_color = palette.text.opacity(0.4);
        self.paint_v_line(window, state.snap_x, plot_top, self.plot_h(), cross_color);
        self.paint_h_line(
            window,
            state.y,
            plot_left,
            self.plot_w(),
            cross_color,
            true,
        );
    }

    /// 右轴价签 + 底轴时间签(不裁剪)。
    fn paint_crosshair_tags(
        &self,
        state: &CrosshairState,
        frame: &FrameData,
        palette: &Palette,
        window: &mut Window,
        cx: &mut App,
    ) {
        let plot_left = self.plot.origin.x.as_f32();
        let plot_top = self.plot.origin.y.as_f32();
        let plot_right = plot_left + self.plot_w();
        let plot_bottom = plot_top + self.plot_h();

        // 右轴价签(y 钳制在价格区内;用图例格式保证小数位一致)。
        let tag_y = state.y.clamp(plot_top, plot_top + self.price_h);
        let price = self.price_at(tag_y);
        Tag {
            bounds: Bounds::new(
                point(px(plot_right + 2.), px(tag_y - AXIS_FONT_SIZE * 0.8)),
                size(px(self.axis_w - 4.), px(AXIS_FONT_SIZE * 1.6)),
            ),
            background: palette.secondary,
            text_color: palette.secondary_text,
            text: scale::format_quote(price),
        }
        .paint(window, cx);

        // 底轴时间签(按文本实宽定签宽,钳制在轴内)。
        if let Some(bar) = state.bar {
            let text = if frame.timeframe.is_intraday() {
                scale::format_ny_datetime(bar.time_ms)
            } else {
                scale::format_ny_date(bar.time_ms)
            };
            let line =
                shape_line(text.clone().into(), AXIS_FONT_SIZE, palette.secondary_text, window);
            let tag_w = line.width().as_f32() + 8.;
            let tag_x = (state.snap_x - tag_w * 0.5)
                .clamp(plot_left, (plot_right - tag_w).max(plot_left))
                .max(self.bounds.origin.x.as_f32());
            Tag {
                bounds: Bounds::new(
                    point(px(tag_x), px(plot_bottom + 2.)),
                    size(px(tag_w), px(TIME_AXIS_H - 4.)),
                ),
                background: palette.secondary,
                text_color: palette.secondary_text,
                text,
            }
            .paint_with_line(window, cx, &line);
        }
    }

    fn paint_legend(
        &self,
        frame: &FrameData,
        palette: &Palette,
        symbol: Option<&str>,
        crosshair: Option<SnapshotBar>,
        window: &mut Window,
        cx: &mut App,
    ) {
        // 十字光标吸附的 bar;无光标时显示最新一根(即使已平移出可视区)。
        let Some(bar) = crosshair.or(frame.latest) else {
            return;
        };

        let mut head = match symbol {
            Some(symbol) => format!("{symbol} · {}", frame.timeframe.label()),
            None => frame.timeframe.label().to_string(),
        };
        let time = if frame.timeframe.is_intraday() {
            scale::format_ny_datetime(bar.time_ms)
        } else {
            scale::format_ny_date(bar.time_ms)
        };
        head.push_str("   ");
        head.push_str(&time);
        let head_line = shape_line(head.into(), LEGEND_FONT_SIZE, palette.text, window);
        let origin_x = self.plot.origin.x.as_f32() + TEXT_PAD;
        let origin_y = self.plot.origin.y.as_f32() + TEXT_PAD;
        let _ = head_line.paint(
            point(px(origin_x), px(origin_y)),
            px(LEGEND_FONT_SIZE * LINE_HEIGHT),
            TextAlign::Left,
            None,
            window,
            cx,
        );

        let color = palette.direction(bar.open, bar.close);
        let muted = palette.muted_text;
        let line = shape_runs(
            [
                ("O ".to_string(), muted),
                (scale::format_quote(bar.open), muted),
                ("  H ".to_string(), muted),
                (scale::format_quote(bar.high), muted),
                ("  L ".to_string(), muted),
                (scale::format_quote(bar.low), muted),
                ("  C ".to_string(), muted),
                (scale::format_quote(bar.close), color),
                ("  V ".to_string(), muted),
                (scale::format_volume(bar.volume), muted),
            ],
            LEGEND_FONT_SIZE,
            window,
        );
        let _ = line.paint(
            point(px(origin_x), px(origin_y + LEGEND_FONT_SIZE * LINE_HEIGHT)),
            px(LEGEND_FONT_SIZE * LINE_HEIGHT),
            TextAlign::Left,
            None,
            window,
            cx,
        );
    }

    // ── 基元 ─────────────────────────────────────────────────────────────

    /// 水平线(1px 实心条带)。虚线是分段实心 quad:PaintQuad 的
    /// `BorderStyle::Dashed` 在零高度 bounds 上会被剔除(gpui 的边框
    /// 几何依赖 bounds 面积),实测不可靠,故自绘 dash 节奏。
    fn paint_h_line(&self, window: &mut Window, y: f32, x: f32, w: f32, color: Hsla, dashed: bool) {
        if !dashed {
            window.paint_quad(fill(
                Bounds::new(point(px(x), px(y)), size(px(w), px(1.))),
                color,
            ));
            return;
        }
        let (on, off) = (4., 4.);
        let mut cursor = x;
        let end = x + w;
        while cursor < end {
            let seg_end = (cursor + on).min(end);
            window.paint_quad(fill(
                Bounds::new(point(px(cursor), px(y)), size(px(seg_end - cursor), px(1.))),
                color,
            ));
            cursor = seg_end + off;
        }
    }

    /// 竖线(1px 实心条带;虚线同上自绘)。
    fn paint_v_line(&self, window: &mut Window, x: f32, y: f32, h: f32, color: Hsla) {
        window.paint_quad(fill(
            Bounds::new(point(px(x), px(y)), size(px(1.), px(h))),
            color,
        ));
    }
}

// ── 文本与签 ─────────────────────────────────────────────────────────────

/// 单色文本塑形(窗口当前文本样式的字体)。
fn shape_line(text: SharedString, size: f32, color: Hsla, window: &mut Window) -> ShapedLine {
    let run = TextRun {
        len: text.len(),
        font: window.text_style().font(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window.text_system().shape_line(text, px(size), &[run], None)
}

/// 多色混排文本塑形(OHLCV 图例:标签弱化、收盘按方向着色)。
fn shape_runs(
    parts: impl IntoIterator<Item = (String, Hsla)>,
    size: f32,
    window: &mut Window,
) -> ShapedLine {
    let mut text = String::new();
    let mut runs = Vec::new();
    for (part, color) in parts {
        text.push_str(&part);
        runs.push(TextRun {
            len: part.len(),
            font: window.text_style().font(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        });
    }
    window
        .text_system()
        .shape_line(text.into(), px(size), &runs, None)
}

/// 轴上的圆角小签(最新价/十字光标的价签与时间签)。
struct Tag {
    bounds: Bounds<Pixels>,
    background: Hsla,
    text_color: Hsla,
    text: String,
}

impl Tag {
    fn paint(&self, window: &mut Window, cx: &mut App) {
        let line = shape_line(
            self.text.clone().into(),
            AXIS_FONT_SIZE,
            self.text_color,
            window,
        );
        self.paint_with_line(window, cx, &line);
    }

    fn paint_with_line(&self, window: &mut Window, cx: &mut App, line: &ShapedLine) {
        window.paint_quad(quad(
            self.bounds,
            px(3.),
            self.background,
            Edges::default(),
            transparent_black(),
            BorderStyle::Solid,
        ));
        // 签内文字垂直居中。
        let tag_h = self.bounds.size.height.as_f32();
        let text_h = AXIS_FONT_SIZE * LINE_HEIGHT;
        let _ = line.paint(
            point(
                px(self.bounds.origin.x.as_f32() + 4.),
                px(self.bounds.origin.y.as_f32() + (tag_h - text_h) * 0.5),
            ),
            px(text_h),
            TextAlign::Left,
            None,
            window,
            cx,
        );
    }
}
