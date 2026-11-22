# 在 client 的 widgets 中实现纯自定义 K 线图组件

## Context

`panels/chart.rs` 目前是占位面板(主窗口/自定义窗口的默认中央面板)。gpui-component 自带的 `CandlestickChart` 是分类(band)坐标轴的静态图:无平移缩放、无实时推送更新、无成交量副图、无十字光标/价格轴,不满足看盘需求,所以按用户要求**纯自定义**实现。

已确认的范围:**交易级交互**(蜡烛+成交量+右侧价格轴+底部时间轴+十字光标+悬停 OHLCV+滚轮缩放+拖拽平移+跟随最新+最新价虚线)、**多周期+全部实时**(1Min/5Min/15Min/1Hour/1Day;日线实时走现有 DAILY 推送,分钟周期由客户端聚合 minute-bar 推送,需给 hub 订阅意愿扩展 `bars` 维度,协议 `Channels::BARS` 与服务端均已支持,零协议改动)、时间轴用**美东时区**(client 新增 chrono-tz 依赖,server 已在用 0.10)。

## 新增 widget:`crates/client/src/widgets/kline/`

数据来自 `RpcRequestPayload::Bars`(协议已有,server 已实现,client 尚无调用方);实时推送经 hub 的 `stock_messages()` 广播,`Frame::Bar` 已由 `utils/server.rs:511` 映射为 `StockMessage::{MinuteBar,UpdatedBar,DailyBar}`(anysdk 形状)。

```
widgets/kline/
├── mod.rs      # pub use + 模块说明(参照 widgets.rs 现有风格)
├── scale.rs    # 纯函数几何/格式化:价格↔y、index↔x、nice-number 刻度、
│               #   分钟桶 floor、美东时间轴标签(chrono-tz America/New_York)——可单测
├── state.rs    # KlineState(Entity,面板持有):
│               #   symbol/timeframe/bars: Vec<Bar>/viewport(end_ix+span,f32 支持平滑)/
│               #   follow_latest/loading/empty;原始分钟 bar 暂存 BTreeMap<i64, RawBar>
│               #   (仅盘中会话,增量重算所在桶,正确处理 UpdatedBar 整桶替换);
│               #   set_history / apply_live_bar / zoom_at / pan_by / pin_latest
└── element.rs  # Kline(RenderOnce 值持有 Entity<KlineState> 引用,Input/InputState 同款
                #   有状态组件模式)+ 自定义 Element 三阶段:request_layout 占满父容器、
                #   prepaint 建 hitbox、paint 用 paint_quad/文本系统绘制:
                #   网格、蜡烛(涨跌色 cx.theme().chart_bullish/chart_bearish,语义 token)、
                #   成交量副图(底部 ~20%)、右价格轴、底时间轴、最新价虚线+价签;
                #   交互:MouseMove→十字光标(元素局部 Cell + cx.notify(view),
                #   照抄 gpui-base plot/element.rs 的 PlotElement 模式,不逐像素刷实体)、
                #   悬停图例 OHLCV、左键拖拽平移、滚轮缩放(光标锚点)、
                #   双击回到最新(follow_latest 重新钉住)
```

要点:
- 十字光标/图例是纯呈现,不进 KlineState;viewport/缩放是领域状态,进实体。
- 每根蜡烛有稳定 index;可见范围外不绘制(数百根量级,无需虚拟化)。
- 不引入裸色值,全部 `cx.theme()` 语义 token;坐标像素为绘制几何,属文档允许的例外。
- 历史加载:日线 500 根、1Min 1000 根(其余 500);加载中保留旧图(渐进刷新);MVP 不做更早历史翻页(`start/end` 参数留作日后)。

## Hub 扩展:`utils/state/hub.rs`

- `SymbolWish` 加 `bars: bool`(分钟 bar 意愿)。
- 镜像 `trades_union` 加 `bars_union`;`set_symbols`/`replay_subscriptions` 对 bars 增减差集发 `Channels::BARS` 订阅/退订(server hub 按位 OR 合并,与 TRADES|DAILY 并存无冲突)。
- 更新全部 `SymbolWish` 结构体字面量(编译器定位):`panels/trades.rs`、`panels/quote.rs`、`panels/watchlist.rs`、`windows/etf_linkcalc.rs` 等补 `bars: false`。
- `stream_dispatch::StreamSpec` 无需改:`trades=true` 时 stage 已转发 Minute/Updated/Daily(119-121 行)。

## ChartPanel 重写:`panels/chart.rs`

照 trades.rs 脚手架完整实现五件套:
- `type DataEntity = KlineState`;`new` 读持久化 info(symbol+timeframe)。
- toolbar:搜索按钮(`open_search_symbol_dialog`,同 trades 模式)+ 周期切换(`Button` 加 `.selected()`,Selectable 已确认)+ `refresh_button::<Self>`。
- `refresh(entity, cx)`:经 `AppState::server(cx)` 拉 Bars RPC(离线静默跳过)+ `reload_on_connect` 补载。
- pump:`spawn_filter_stage` + `apply_batch` 按 timeframe 过滤:1Day 吃 `DailyBar`,分钟周期吃 `MinuteBar/UpdatedBar`(喂给 KlineState 聚合)。
- `set_active` 隐藏退订/激活补订 + 停/启 pump;`on_removed`/`on_release` 清 wish(双保险);`dump` 写 symbol+timeframe。
- 空态/加载态:面板内 `datatable::empty_view`/`loading_view` 复用;错误进消息中心。

## 其他

- `crates/client/Cargo.toml`:+`chrono-tz = "0.10"`。
- `widgets.rs`:+`pub mod kline;`。
- locale:`app.chart.*` 节(en-us/zh-cn 同构;改 YAML 后 touch 任一 .rs 保证重编译)。
- `panels/chart.rs` 顶部 `use crate::widgets::kline::…`。

## 测试与验证

1. 单测(scale.rs + state 聚合):nice 刻度、桶 floor(美东日界)、RawBar 聚合合并/替换、viewport 缩放锚点数学。
2. `cargo check -p client --all-targets && cargo clippy -p client --all-targets`(保持零警告)&& `cargo test -p client --lib`。
3. 渲染截图:临时 `crates/client/examples/kline.rs`(bootstrap + init_app + 合成 bars 开窗包 Root,不走 open(),PrintWindow 截图),截完即删。
4. 用户手测清单(编号):搜索切标的、切周期、盘中实时刷新(日线/分钟)、缩放平移、双击回最新、重启后 symbol/周期还原、隐藏 tab 后退订重显补订。

预计规模:widget 约 900-1200 行 + 面板/接线约 400 行。