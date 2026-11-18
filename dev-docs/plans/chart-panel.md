# Chart 面板(蜡烛图)实现任务书

> **由用户自行实现**,本文档是实现前的地形侦察与决策清单(2026-11-17 整理)。
> 所有 API 均已核实到具体文件/行号;决策点列出待定项,实现时按需勾选。

## 现状

- `crates/client/src/panels/chart.rs`(53 行)是占位壳:`DataEntity = ()`,
  NAME=`ChartPanel`、ICON=Chart、TITLE_KEY=`app.panels.chart`,已在
  `panels::all()` 与菜单注册——**只需填实现,不动注册**。

## 可复用的渲染资产(gpui 栈)

gpui-component 0.9.1 **自带蜡烛图组件**(`src/chart/candlestick_chart.rs`),
底层是 gpui-base 0.9.1 的 plot 机器(`src/plot/`:axis / grid / hover /
scale / path_cache / shape)。

**组件已有的能力**(直接可用):
- `CandlestickChart<T, X, Y>::new(data)` + `.x()/.open()/.high()/.low()/.close()`
  访问器取数
- `.bullish()/.bearish()` 牛熊色(接主题 `chart_bullish`/`chart_bearish`)
- `.tooltip_title()/.tooltip_value()/.tooltip_content()` 悬停信息
- `.x_axis(bool)`、`.grid(bool)`、`.interactive(bool)`、appear 动画
- `.tick_margin()/.body_width_ratio()/.max_band_width()` 蜡烛形态
- 数据变化后需要重建组件实例(模式同 TableState 列组快照)

**缺口**(即"功能不完善"的具体清单,交易级图表需要自行补的):
- 无成交量副图
- candlestick 未接 crosshair(plot 机器有,area/line 已用,candle 没用上)
- 无平移/缩放(滚轮翻历史、拖拽视窗)——x 轴是分 band 的类别轴
- 无实时最后一根 K 线的流内更新(组件设计为静态数据一次性传入)
- 无右侧价格轴读数、最新价线、均线叠加
- 无周期切换(组件不关心数据来源,周期切换 = 换数据重建)

**两条实现路线**(实现时二选一或混合):
1. **组件路线**:用 `CandlestickChart` 画主图,数据换挡时重建实例;缺的功能
   (成交量副图等)用叠加元素自补。上手快,交互天花板受组件限制。
2. **自绘路线**:直接用 gpui-base plot 的 element/scale/shape 自己拼(area/line
   的 crosshair 实现是现成参考,`plot/element.rs` 是绘图入口)。自由度最高,
   工作量大。

## 数据源(AnySDK,已核实)

- **REST 拉历史**:`anysdk::alpaca_sdk::bars::Params`
  (`bars.rs:44`:symbols / timeframe / start / end / limit(1..=10000)/
  page_token / sort / adjustment / feed)+ `Client::bars` → `BarsPage`
  (`page_token` 可翻页,v2 API)。timeframe 如 `1Min`/`1Hour`/`1Day`。
- **流内实时**:`StockMessage::MinuteBar` / `UpdatedBar` / `DailyBar`
  (`BarMessage`:o/h/l/c/v/t,`stocks.rs:158`)—— MinuteBar/UpdatedBar
  正好用于流内更新最后一根分钟 K。
- 组合建议:打开面板 → REST 拉近期历史;订阅该标的(trades 恒订,
  minute bar 也会随之到达)→ 流内 bar 更新/追加最后一根。

## 决策点(实现前先定)

- [ ] 渲染路线:组件 vs 自绘
- [ ] MVP 范围:建议先 `REST 日线/分钟线 + CandlestickChart 静态渲染 +
      选标的(搜索对话框)+ 周期切换`,实时更新/缩放平移做二期
- [ ] 成交量副图要不要进 MVP
- [ ] symbol 选择交互:搜索对话框(与 Quote/Trades 一致)即可
- [ ] 持久化:`dump` 存 `{symbol, timeframe}`(info json,同 Quote/Trades)
- [ ] 订阅语义:`SymbolWish { symbols, quotes: false }`(bar 随 trades 走)
- [ ] 主题:bullish/bearish 接 `chart_bullish`/`chart_bearish`

## 交互缺口清单(自绘/增强阶段逐项做)

- [ ] 十字光标 + 价格/时间读数(plot hover 机器已有,candle 接上)
- [ ] 滚轮横向翻历史(REST 翻页加载更早数据,page_token)
- [ ] 拖拽平移视窗
- [ ] 最新价线 + 实时最后一根 K 更新(MinuteBar/UpdatedBar)
- [ ] 成交量副图
- [ ] 均线叠加(MA5/MA20 之类,纯计算)

## 验收(MVP 后)

- 菜单打开 Chart → 搜索选标的 → 显示蜡烛图,周期可切
- 面板隐藏/关闭/重启后 symbol 与周期还原(dump/new)
- 隐藏面板时暂停订阅;关闭退订;断线重连后数据自动恢复
- clippy 零警告 / tests 全过
