# ZenSidus App — 功能清单与数据层重构设计

> 状态:**已实施**(2026-11,commit P0a–P3:crates/app → crates/client、
> 新增 crates/relay-protocol 与 crates/server、全部面板迁移到 relay TCP/postcard;
> 与实现有出入处以代码为准——`Channels` 在草图 4 通道外拆出 `DAILY`,
> 推送载荷结构化,`RestProgress` 为 D2 新增帧)。
> 目标:数据获取全面迁移到自建服务器,**全程 TCP + postcard 二进制协议,彻底移除 HTTP/WS 依赖**。
> 本文档分三部分:① 当前功能与数据获取点全量清单(逐面板详述);② 新协议与新架构设计;③ 迁移路线。

---

## Part 1 — 功能与数据获取点全量清单

### 1.0 通用机制(所有面板共享)

**面板系统**
- 每个面板实现 `Panel` trait:`NAME`(持久化主键)/`ICON`/`TITLE_KEY`/`new(window, cx, info)`(info = 上次 `dump` 的持久化载荷)/`refresh`(刷新按钮触发)。
- 标题栏结构:`[icon + 标题] [toolbar_buttons] [title_suffix(下拉框等,固定 200px)]`。
- **toolbar_buttons**:面板自定义的图标按钮组(`icon_button(id, icon, tooltip, disabled, on_click)`)。
- **title_suffix + 管理下拉框**(`StringPicker::managed_element`):单选下拉 + 底部按钮行,可选**添加/编辑/删除**三按钮(添加恒可用;编辑/删除在未选中时置灰;点击回调统一收到当前选中值)。Watchlist/股票筛选器/市场筛选器三处共用。
- **生命周期**:面板切走(非激活 tab)→ 暂停流订阅 + 停消息泵;切回/流重连 → 恢复;面板关闭 → 退订 + `dump` 持久化;应用重启 → 按持久化布局恢复全部面板与选中状态。
- **消息泵**(所有实时面板):后台过滤级联(`stream_dispatch::filter_stage`)按面板 `StreamSpec`(symbols/trades/quotes/min_size/exchanges)过滤全量广播流 → 有界通道 → 主线程前景泵逐批提交。过载丢批不积压。stderr 打印 `[lag] panel: in~/fwd~/oldest_max/dropped` 诊断行。

**用户交互(全面板共用)**
- 搜索标的对话框(`open_search_symbol_dialog`):搜索框 + 结果列表,选中回调;被 Watchlist(添加)、Quote(切换跟随)、Trades(切换 tape)、Stock News(切换标的)复用。
- 确认对话框(`open_confirm_dialog`):删除类操作的统一确认。
- 列交互:表格列宽双击自适应(`fit_column` 量宽)、可排序;表头不可移动。

---

### 1.1 Watchlist(自选)— `WatchlistPanel`

**功能**:按分组管理自选标的,实时显示最新价/涨跌/最后成交时间。

**工具栏(toolbar_buttons)**
- 🔍 `watchlist-add-symbol`:打开搜索对话框;选中标的 → 加入当前分组(写入 settings `watchlists`),表格与订阅随 settings 观察者刷新。

**标题栏下拉框(title_suffix)**
- 分组选择器(当前分组的标的集合)。底部按钮:
  - ➕ 新建分组:输入名称对话框 → 创建并切换到新分组。
  - 🗑 删除分组:确认对话框("删除分组 X 及其 N 只标的")→ 删除分组;选中回落到第一个剩余分组。

**显示布局**:表格 5 列 — `Symbol`(代码)| `Last`(最新价)| `Change`(涨跌)| `Change %`(涨跌幅)| `Updated`(最后成交时间,HH:MM:SS)。行按分组内 settings 顺序。

**用户交互**
- **Ctrl + 点击行**:从分组中删除该标的(写入 settings;普通单击无操作)。
- 切换分组(下拉框):表格与订阅随分组切换。

**数据获取**
- 现状:WS — `SymbolWish { symbols: 分组标的, trades: true, quotes: false }`;Last ← trades 最新成交,Updated ← 成交时间戳(本地格式化);Change 基于 dailyBar 的当日开盘价。
- 重构后:TCP `Subscribe{channels: TRADES|BARS}` + `Trade`/`Bar` 推送帧;逻辑不变。

**空态**:分组无标的 → "暂无自选代码,在上方搜索添加"。

**持久化**:`dump → {"group": 选中分组名}`;分组数据本身在 settings.watchlists。

**订阅语义**:`quotes: false`(只要逐笔+日线);隐藏面板退订、关闭退订、重连自动回放订阅。

---

### 1.2 Quote(单票行情)— `QuotePanel`

**功能**:单个标的的实时最优买卖盘(top of book)。

**工具栏**
- 🔍 `quote-pick-symbol`:搜索对话框 → 切换跟随标的(清空当前报价,重新订阅)。

**标题栏下拉框**:无。

**显示布局**(芯片行,与 ETF 联价器同款 flat chip):
- 表头行:`[标的 chip][Price][Size][Exchange]` 列标签。
- **Bid 行**:价格(红)/ 数量(千分位)/ 交易所码(单字母)。
- **Ask 行**:价格(绿)/ 数量 / 交易所码。
- **快照语义**:始终显示当前最优报价;单边报价合法(缺失侧 `—`);断线清空显示(不显示过期报价)。

**数据获取**
- 现状:WS — `SymbolWish { symbols: [标的], trades: false, quotes: true }`。
- 重构后:TCP `Subscribe{channels: QUOTES}` + `Quote` 推送帧。

**持久化**:`dump → {"symbol": 标的}`。

**空态**:未选标的 → "从工具栏搜索选择标的"(居中提示)。

---

### 1.3 Trades(逐笔成交)— `TradesPanel`

**功能**:单个标的的实时逐笔成交记录(time & sales),**记忆模式记录簿**。

**工具栏**
- 🔍 `trades-pick-symbol`:搜索对话框 → 切换 tape 标的(清空旧记录,重新开始记录)。

**标题栏下拉框**:无。

**显示布局**:表格 4 列 — `Time`(HH:MM:SS.mmm)| `Price`(2 位小数)| `Size`(数量)| `Exchange`(交易所码)。**新成交插最上面**;上限 **500 笔**,更早的从底部移除;断线保留已记录内容;切换标的清空重录。

**数据获取**
- 现状:WS — `SymbolWish { symbols: [标的], trades: true, quotes: false }`(trades 通道恒订语义)。
- 重构后:TCP `Subscribe{channels: TRADES}` + `Trade` 推送帧。

**持久化**:`dump → {"symbol": 标的}`(tape 本身不持久化,重启后从零记录)。

**空态三态**:未选标的 / 流未连接 / 已连接但暂无成交。

**订阅语义**:隐藏退订、关闭退订、重连回放(与 Watchlist 同款生命周期)。

---

### 1.4 Stock News(个股新闻)— `StockNewsPanel`

**功能**:单个标的的新闻页(最新 50 条)。

**工具栏**
- 🔍 `stock-news-pick-symbol`:搜索对话框 → 切换资讯标的并重新拉取。

**标题栏下拉框**:无。

**显示布局**:表格 3 列 — `Headline`(标题)| `Source`(来源)| `Updated`(时间)。**点击行**:在系统默认浏览器打开新闻原文。

**数据获取**
- 现状:**HTTP** — Alpaca REST `news`(symbols=[标的], limit=50),经 `alpaca_client`。
- 重构后:TCP RPC `RpcRequestPayload::News` → `RpcResponsePayload::News`。

**持久化**:`dump → {"symbol": 标的}`;重启自动重新拉取。

**空态**:未选标的 / 拉取失败(消息中心提示)/ 无凭据提示。

---

### 1.5 Market News(市场新闻)— `MarketNewsPanel`

**功能**:全市场新闻流,推送式(非轮询)。

**工具栏**:无。

**标题栏下拉框**:无。

**显示布局**:表格 4 列 — `Headline` | `Symbols`(相关标的列表)| `Source` | `Updated`。**点击行**:浏览器打开原文。

**数据获取**
- 现状:**WS** — 独立 `NewsStream`(Alpaca news WS,`subscribe_all` 全市场);面板隐藏时断开、激活时重连(新闻流低频,省资源)。
- 重构后:TCP `Subscribe{channels: NEWS}`(并入主连接,常驻)+ `News` 推送帧。

**持久化**:无(全市场流,无选择状态)。

**空态**:连接前 → "正在等待新闻流推送";断连 → 提示。

---

### 1.6 Stock Screener(股票筛选器)— `StockScreenerPanel`

**功能**:按 Finviz 预设(信号/筛选/排序/导出列)拉取筛选结果表格。

**工具栏**
- ↻ `screener-refresh`:重新拉取当前预设的导出数据。

**标题栏下拉框(title_suffix + 共享 footer)**
- 预设选择器(列出 settings.finviz.screener_parameter 全部预设;选中即加载该预设数据)。底部按钮:
  - ➕ 新建预设:打开预设表单对话框(空表单):名称 / 类型(Stocks|ETF Radio)/ Signal / Order / Filters / Columns(多选 chips,从 Finviz 156 个导出列中选)。确认 → 写入 settings。
  - ✏️ 编辑预设:同一表单,按选中预设预填;确认按 upsert(支持改名,旧键删除)。
  - 🗑 删除预设:确认对话框 → 删除;选中回落第一个剩余预设。

**显示布局**:表格**动态列**(列 = Finviz 导出 headers,由预设的 columns 参数决定;如 Ticker/Volume/Price)。行 = 筛选结果。

**数据获取**
- 现状:**HTTP** — Finviz Elite `screener` CSV 导出(经 `finviz_client`,token 在 keyring)。
- 重构后:TCP RPC `ScreenerExport{parameter}` → `ScreenerExport{headers, rows}`(Finviz token 移到服务端,客户端不再持有)。

**持久化**:`dump → {"preset": 选中预设名}`;预设定义在 settings.finviz.screener_parameter。

**空态**:未选预设 → "从工具栏选择预设";无 API key → 提示去设置;预设无数据 → "预设没有返回数据"。

**订阅语义**:无流订阅(纯请求-响应)。

---

### 1.7 Market Screener(市场筛选器)— `MarketScreenerPanel`

**功能**:两轮筛选。**第一轮**:Finviz 预设导出股票池(上限 = settings.pool_limit,默认 200,0 = 不限;超出截断并提示)。**第二轮**:订阅池内全部标的的实时报价,按预设规则(交易所 + 最小挂单量)实时判定,**每个交易所一张表**,命中事件追加为记录行。

**工具栏**
- ↻ `market-refresh`:重新拉取股票池(按当前 pool_limit 截断)。

**标题栏下拉框(title_suffix + 共享 footer)**
- 市场预设选择器。底部按钮:
  - ➕ 新建市场预设:表单对话框(宽 480px):**Preset title**(标题,工具栏显示用)/ **Finviz preset**(单选下拉,首轮筛选)/ **Exchanges**(chips 多选,18 项交易所码表,如 K=EDGX、P=NYSE Arca)/ **Minimum size**(数字,0 = 不限)。确认 → upsert(settings.market_screener.presets)。
  - ✏️ 编辑市场预设:预填选中预设;确认 upsert(支持改名)。
  - 🗑 删除预设:确认对话框;选中回落第一个剩余预设。

**显示布局**:交易所列表展开为 **N 张并排表**(每张固定 160px 宽 = Symbol 72 + Size 84,容器横向滚动),表头小标题 = 交易所名(如 "EDGX Equities (K)")。每表 2 列:`Symbol` | `Size`。
- **记忆模式(记录簿)**:池内标的一条报价的 ask 或 bid 挂在该交易所且量 ≥ min_size → 立即在该表追加一行(两侧同时命中取较大 size);行不会消失;同一代码可重复出现;新行插最上;每表上限 **400 条**;断线保留记录;切预设(交易所集合变化)或关面板才清空。
- 规则编辑只影响**之后**到达的报价。

**数据获取**
- 现状:**HTTP**(Finviz `market_screener` 圈池)+ **WS**(quotes 通道,`SymbolWish { trades: false, quotes: true }`)。
- 重构后:TCP RPC `MarketScreenerExport` 圈池 + TCP `Subscribe{channels: QUOTES}` + `Quote` 推送帧(命中判定在客户端 stream_dispatch 级联,逻辑不变)。

**持久化**:`dump → {"preset": 选中预设名}`;预设定义在 settings.market_screener.presets;pool_limit 在 settings.market_screener.pool_limit。

**空态三态**:未连接 → "Not connected" / 池子为空 → "Empty pool" / 已连接无命中 → "No hits yet"。

**订阅语义**:隐藏退订、关闭退订、重连回放;pool 上限在设置 → 市场筛选器页可调(0 = 不限)。

---

### 1.8 Economic Calendar(经济日历)— `EconomicCalendarPanel`

**功能**:指定周的宏观经济事件表。

**工具栏**:无(周选择在标题栏,见下)。

**标题栏**:**DatePicker(周选择器)** — 选任意日期,自动吸附到所在周的周一;标题栏旁显示周区间(如 "11/23/2026 Week of")。

**显示布局**:表格 6 列 — `Time`(时间)| `Event`(事件名,彩色胶囊按影响级别着色)| `For`(对象)| `Actual`(实际值)| `Expected`(预期)| `Prior`(前值)。未来事件行有浅色底纹突出。

**数据获取**
- 现状:**HTTP** — `gateway_client.economic_calendar(week_start)`(自建网关 → Finviz 经济日历加工)。
- 重构后:TCP RPC `EconomicCalendar{week_start}` → 日历 DTO。

**持久化**:`dump → {"date": 所选周一}`(重启恢复到该周)。

**空态**:拉取失败 → 消息中心提示。

---

### 1.9 Earnings Calendar(财报日历)— `EarningsCalendarPanel`

**功能**:指定交易日(周内单日)的财报表。

**工具栏**:无。

**标题栏**:**DatePicker(日选择器)** — 周末禁选(财报按交易日报告);显示所选日(如 "11/19/2026 Thursday")。

**显示布局**:表格 7 列 — `Company`(公司)| `Symbol` | `Managers`(承销商)| `Shares (Millions)` | `Price Low` | `Price High` | `Expected Date`。

**数据获取**
- 现状:**HTTP** — `gateway_client.earnings_calendar(date)`。
- 重构后:TCP RPC `EarningsCalendar{date}`。

**持久化**:`dump → {"date": 所选日}`。

---

### 1.10 IPO Calendar(IPO 日历)— `IPOCalendarPanel`

**功能**:近期 IPO 列表。

**工具栏**:无。**标题栏**:无选择器(固定近期窗口)。

**显示布局**:表格 7 列 — `Company` | `Symbol` | `Managers` | `Shares (Millions)` | `Price Low` | `Price High` | `Expected Date`。

**数据获取**
- 现状:**HTTP** — `gateway_client.ipo()`。
- 重构后:TCP RPC `IpoCalendar`。

**持久化**:无(启动自动加载)。

---

### 1.11 Chart(图表)— `ChartPanel` — **用户自行实现**

占位壳(53 行)。计划:K 线蜡烛图。侦察与设计见 `dev-docs/plans/chart-panel.md`(gpui-component 有 CandlestickChart 组件但功能不全;数据源 AnySDK bars REST + 流内 bar)。重构后数据源:TCP RPC `Bars{timeframe,…}`(历史)+ 流内 `Bar` 推送帧(实时最后一根)。

---

### 1.12 Experiment(实验性)— `ExperimentPanel`

框架试验田占位(组件/API 验证用),保持空壳是设计意图,不参与重构。

---

### 1.13 窗口(非面板)

#### ETF LinkCalc(ETF 联价器,多实例窗口)

**功能**:杠杆 ETF 与其指数代理的联价器——由当日开盘价锚定,实时换算任一 ETF 报价水平对应的指数价格,并给出 bid/ask 两侧的"破点价"。

**标题栏按钮**
- ⚙ `etf-params`:参数对话框(宽自适应):Index Symbol / ETF Symbol(文本)/ ETF 倍数(数字)/ 方向(Long/Short Radio)/ 当日指数开盘价 / 当日 ETF 开盘价(数字,自动锚定后当天内不被覆盖)/ 指数偏移值 / 多空偏移值(均可负)。确认即写缓存并重新订阅;换 symbol 时自动重拉快照锚定开盘价。

**显示布局**
- Row1:`[指数 Symbol chip][Bid 芯片红][Ask 芯片绿]`
- Row2:`[ETF Symbol chip][Bid][Ask]`
- Row3:`[×倍数 chip][方向 chip][破点 ×4]` —— 破点 = 隐含指数 ± 偏移;**两侧同式**:`计算值 + 指数偏移 + 多空差值`;列按 quote 侧对齐(bid 派生对在 bid 下方)。
- 开盘价**自动锚定**:窗口打开/换 symbol 时 HTTP snapshot 取当日日线 open(仅接受今天的 bar,防昨日锚污染);流内 DailyBar 持续校准;日期门控保证手动修改当天不被覆盖、跨日自动失效。
- 锚点未就绪时破点显示 `—`(说明文案已移入文档素材,不上屏)。

**数据获取**
- 现状:**WS**(quotes 通道,`trades:false/quotes:true`)+ **HTTP**(snapshot 取日线锚点)。
- 重构后:TCP `Subscribe{channels: QUOTES}` + `Quote` 帧;TCP RPC `Snapshot` 取锚。

**持久化**:多实例(`WindowType::ETFLinkCalc(id)`);每实例缓存 params + 开盘价锚定日期,窗口关闭/退出即写。

#### Small Clock(小时钟,单实例 PopUp 窗口)

**功能**:屏幕角落的时钟 + 经济事件提醒(声音提示临近事件)。

**数据获取**
- 现状:**HTTP** — `gateway_client.timestamp()` 网络对时(本地钟偏差校正);经济日历数据(与 Economic Calendar 同源)。
- 重构后:`Welcome.server_time_ms` + 心跳 RTT 对时(免费);日历数据走 RPC。

**持久化**:settings.small_clock(alert_minutes 提前分钟数 / sound 声音开关 / min_impact 最低提醒级别);设置页可调。

#### 主窗口(Main)

- Dock 布局:左侧 dock(Watchlist 等侧栏面板)/ 顶部 center tab 组 / 右侧 dock(经济日历等);布局持久化(dock 事件即时落盘)。
- 标题栏:App / Panels / Windows 三个菜单(Panels 菜单按族分组列出全部面板入口);右下 Alpaca 连接状态点(绿=连接,黄=连接中,红=失败);左下最新消息 + 点击弹消息历史。
- 退出:X → 退出对话框(取消 / 不保存 / 保存)——保存时批量回写窗口几何。

---

### 1.14 对话框(全局复用)

| 对话框 | 触发点 | 输入 → 输出 | 数据获取 |
|---|---|---|---|
| Search Symbol | Watchlist/Quote/Trades/StockNews 工具栏 | 关键词 → 标的列表(单选) | **HTTP** `gateway_client.search` → 重构后 RPC `Search` |
| Translate | 标题栏菜单 | 文本 + 源/目标语言下拉 → 译文 | **HTTP** `gateway_client.translate` → RPC `Translate` |
| Confirm(通用) | 所有删除类操作 | 标题 + 说明 → 确认/取消 | 无网络 |
| Screener Preset | 股票筛选器 footer | 名称/类型/Signal/Order/Filters/Columns 多选 → Finviz Parameter | 无网络(确认后才发请求) |
| Market Preset | 市场筛选器 footer | 标题/Finviz 预设单选/交易所多选/最小量 → MarketPresetDraft | 无网络 |
| ETF Params | 联价器 ⚙ | 2 symbol/倍数/方向/双开盘价/双偏移 → LinkParams | 无网络 |

---

### 1.15 全局数据服务(现状架构 → 重构映射)

| 现状组件 | 职责 | 重构后 |
|---|---|---|
| `AlpacaHub`(单条股票 WS + 订阅聚合 + staleness 看门狗) | `SymbolWish{symbols,trades,quotes}` 按消费者聚合 union → 上游订阅;引用计数;广播分发;15s 数据滞后自动重连 | 语义平移:hub 改为向 relay 发 `Subscribe/Unsubscribe` 帧;聚合/引用计数/看门狗逻辑保留(看门狗数据源换成 TCP 帧) |
| `stream_dispatch`(后台过滤级联) | worker 线程 drain 广播流 → StreamSpec 过滤 → 有界通道 → 主线程 | **不变**(数据源从 broadcast 换成 relay 客户端解帧出的同一 broadcast) |
| `AlpacaClient`(REST:news/snapshots/bars) | 面板 HTTP 请求 | RPC 帧(News/Snapshot/Bars) |
| `FinvizClient`(HTTP CSV) | 筛选器导出 | RPC `ScreenerExport`(token 下沉服务端) |
| `GatewayClient`(HTTP:search/translate/timestamp/日历×3) | 自建网关端点 | RPC Search/Translate/日历;timestamp 由 Welcome/心跳对时替代;**gateway_sdk crate 退役** |
| `NewsStream`(独立新闻 WS) | 市场新闻面板 | `Subscribe{channels: NEWS}` 并入主连接;独立 NewsStream 退役 |
| 凭据(keyring) | Alpaca key/secret + Finviz token | **迁到 relay-server**(VPS 配置文件);客户端只持 relay token;Alpaca/Finviz 的 keyring 项退役 |

**重构后的出站连接**:唯一一条到 relay-server 的 TCP(postcard 帧,TLS 可选)。任何面板新增数据需求 = 协议加一种帧,不再引入新的协议栈。

---

## Part 2 — 新架构:自建服务器 + TCP + postcard

### 2.1 角色划分

```
┌─ VPS(自建,JP 大阪 500M)──────────────────────┐
│ relay-server(新 crate)                        │
│  ├─ 上游适配层:                                │
│  │   Alpaca 股票 WS(1 条)   ← anysdk          │
│  │   Alpaca 新闻 WS(1 条)   ← anysdk          │
│  │   Alpaca REST(news/bars/snapshot)← anysdk  │
│  │   Finviz Elite HTTP       ← finviz_sdk      │
│  │   网关功能(search/translate/日历/对时)本地  │
│  ├─ 订阅聚合:per-client 意愿 → 上游并集(引用计数) │
│  ├─ 下游:TCP listener(postcard 帧,TLS 可选)  │
│  └─ 客户端认证:Hello.token 预共享              │
└──────────────┬─────────────────────────────────┘
               │ 一条 TCP 长连接 / 客户端(postcard 帧,全双工)
┌─ 本机 ZenSidus ────────────────────────────────┐
│ relay-client(utils/relay.rs,新)               │
│  ├─ 连接管理:自动重连/心跳/订阅重放(现有 hub 语义平移)│
│  ├─ 帧解出 → broadcast → stream_dispatch 级联不变 │
│  └─ RPC 请求-响应复用同一 TCP(id 关联)          │
└────────────────────────────────────────────────┘
```

### 2.2 线协议(wire protocol)

**传输**:一条 TCP 长连接,全双工。**帧格式**:`[u32 LE 长度][postcard 编码的 Frame]`。

```rust
#[derive(Serialize, Deserialize)]
pub enum Frame {
    // ── 会话 ──
    Hello { token: String, app_version: String },   // 客户端首帧;token = 服务端预共享密钥
    Welcome { server_time_ms: i64 },                // 服务端确认(顺带对时,替代 timestamp 端点)

    // ── 心跳 ──
    Ping,                                           // 客户端 → 服务端,10s 间隔
    Pong,                                           // 服务端 → 客户端(流活性证据)

    // ── 订阅协商(替代 Alpaca subscribe/subscription 帧)──
    Subscribe   { id: u32, symbols: Vec<String>, channels: Channels },  // Channels bitflags: TRADES|QUOTES|BARS|NEWS
    Unsubscribe { id: u32, symbols: Vec<String>, channels: Channels },
    Subscribed  { id: u32 },                        // 服务端确认

    // ── 行情推送(服务端 → 客户端)──
    Quote { symbol: String, bid_exchange: String, bid_price: f64, bid_size: i64,
            ask_exchange: String, ask_price: f64, ask_size: i64, time_ms: i64 },
    Trade { symbol: String, id: i64, exchange: String, price: f64, size: i64,
            conditions: Vec<String>, time_ms: i64 },
    Bar   { symbol: String, kind: BarKind, open: f64, high: f64, low: f64,
            close: f64, volume: i64, time_ms: i64 },    // kind: Minute/Updated/Daily
    News  { id: i64, symbols: Vec<String>, headline: String, source: String,
            url: String, time_ms: i64 },

    // ── RPC(请求-响应,复用同一条 TCP;id 关联)──
    RpcRequest  { id: u32, payload: RpcRequestPayload },
    RpcResponse { id: u32, result: Result<RpcResponsePayload, String> },
}

pub enum RpcRequestPayload {
    News { symbols: Vec<String>, limit: u32 },
    Bars { symbols: Vec<String>, timeframe: String, start: Option<String>,
           end: Option<String>, limit: u32 },
    Snapshot { symbols: Vec<String> },
    ScreenerExport { parameter: ScreenerParameterDto },        // Finviz 代理(token 在服务端)
    MarketScreenerExport { parameter: ScreenerParameterDto },
    Search { keyword: String, lang: String },
    Translate { from: String, to: String, text: String },
    EconomicCalendar { date: String },
    EarningsCalendar { date: String },
    IpoCalendar {},
}
// RpcResponsePayload:对应 DTO(News(Vec<NewsItem>) / Bars / Snapshot /
// ScreenerExport{headers, rows} / Search / Translate / Calendars…)
```

**设计要点**
- **DTO 全 owned**(postcard 无生命周期);时间统一 `i64 ms`(epoch UTC);交易所码原样字符串
- **RPC 与推送共用一条 TCP**,响应按 id 路由、与推送顺序无关;洪峰时 RPC 可能排在推送后(一期接受)
- **postcard** varint 编码:quote 帧 ≈ 80-120B,比 JSON 省 30-40% 带宽
- **背压**:客户端接收缓冲满时,服务端对**推送**执行按 symbol coalescing(保最新丢中间),**RPC 帧永不丢**

### 2.3 客户端新架构(app 侧)

```rust
// 新模块 utils/relay.rs:替代 executor 三客户端 + 两条 WS 的唯一落点
pub struct RelayClient {
    // 连接状态机:Connecting/Connected/Reconnecting(自动重连 + 订阅重放)
    // RPC 表:HashMap<u32, oneshot::Sender>(按 id 路由响应)
    // 推送出口:tokio broadcast(→ stream_dispatch 级联原样复用)
}
```

对接映射:

| 现状 | 改造后 |
|---|---|
| `AlpacaHub`(WS 订阅聚合 + staleness 看门狗) | 语义平移:`SymbolWish` → `Subscribe/Unsubscribe` 帧;union/引用计数/看门狗保留在客户端 hub(数据源换 TCP 帧) |
| `AlpacaClient`(REST) | `relay.rpc(RpcRequestPayload::News/Snapshot/Bars…)` |
| `FinvizClient`(HTTP) | RPC `ScreenerExport`(Finviz token 下沉服务端) |
| `GatewayClient`(HTTP ×6 端点) | RPC Search/Translate/日历×3;timestamp 由 `Welcome.server_time_ms` + 心跳 RTT 替代;**gateway_sdk crate 退役** |
| `NewsStream`(独立 WS) | `Subscribe{channels: NEWS}` 并入主连接,常驻;NewsStream 退役 |

**不变**:面板 UI、`stream_dispatch` 级联、`SharedSpec` 语义、消息中心、i18n、持久化。

### 2.4 服务端(relay-server)

- 技术:Rust + tokio;上游 anysdk(alpaca_stream / alpaca_sdk / finviz_sdk)直接复用;下游 tokio + postcard 自写 frame codec
- 认证:`Hello.token` 与服务端配置比对,失败断开;个人自用,无账号体系
- 订阅聚合:与客户端 hub 同语义——per-client 意愿表 → 上游并集(引用计数);客户端断开撤销其意愿
- 服务端规则过滤(二期可选):market-screener 规则评估前移到服务端省下行带宽;一期全量透传
- 可观测:连接数、上游重连、每客户端速率、RPC 延迟日志
- 部署:JP 大阪 VPS(500M 带宽 / 500GB 流量),systemd 常驻;先确认流量计费方向(见 Part 1 市场筛选器的带宽账:150 只 ≈ 140GB/月入站)

### 2.5 安全

- 传输加密:**rustls TLS + 自签证书固定**(推荐,不依赖 Clash 配置);或复用现有 SSH/VLESS 隧道(协议层不加密)
- 认证:Hello.token 预共享;TLS 防重放

---

## Part 3 — 迁移路线(建议的实施顺序)

### 阶段 0:协议 crate(半天)
`relay-protocol`(或并入 AnySDK):Frame/DTO + postcard derive + 编解码往返单测。

### 阶段 1:relay-server 骨架 + 股票流(1-2 天)
TCP listener + Hello/Welcome + 心跳 + 上游股票 WS 接入 + Quote/Trade/Bar 推送 + 订阅协商。
**验收**:客户端经 relay 收 SPY quote,`alpaca:lag_test` 式对照——预期 VPS 路线延迟 150-250ms(vs 现状 ~950ms)。

### 阶段 2:RPC 通道 + 端点迁移(1-2 天)
RPC 帧 + news/screener/search/translate/日历/ipo 逐个迁移;executor 三客户端退役。
**验收**:对照 1.1-1.14 表逐项过,功能与迁移前一致。

### 阶段 3:News WS 并入 + 收尾(半天)
NewsStream 退役(订阅并入 relay);timestamp 端点退役(Welcome/心跳对时)。
**验收**:应用运行时**零 HTTPS/WSS 出站连接**(网络面板确认)。

### 阶段 4(二期,可选)
服务端规则过滤(省下行带宽)、bars 历史(Chart 数据源)、多客户端共享、慢客户端 coalescing 细化。

---

## 风险与开放问题

1. **RPC 与推送共享 TCP 的队头阻塞**:推送洪峰时 RPC 延迟上升(同队列)。一期接受;体感明显则拆第二条 RPC 专用连接(协议不变,多一条会话)
2. **时钟偏差**:延迟读数依赖两端 NTP;`Welcome.server_time_ms` + 心跳 RTT 可估算并补偿
3. **Finviz token 下沉服务端**:客户端 `settings.finviz.api_key` 退役(字段保留兼容,不再使用)
4. **断线语义**:重连后服务端按客户端重放的订阅向上游重新订阅;重连期间的行情数据天然缺失(与现状一致)
5. **postcard 版本兼容**:Frame 加字段用 `#[serde(default)]`;客户端/服务端版本独立升级需协议版本号(一期单用户手工同步)
6. **服务器可用性**:relay-server 挂了 = 全部数据不可用(比现状多一个单点)。缓解:systemd 自动拉起 + 客户端无限重连 + (应急)客户端保留直连 Alpaca 的代码路径作降级开关
