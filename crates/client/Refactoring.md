# ZenSidus Client 重构方案

> 状态:提案(2026-11-23)。本文基于当前代码逐面板盘点,诊断"HTTP 时代演进到
> RPC 后的架构包袱",给出目标架构、可复用组件清单与渐进迁移路线。
> 配套背景阅读:`.agents/skills/zensidus-client/`(面板架构与实测陷阱)、
> `.agents/skills/zensidus-server/`(线协议与 server 端约定)。

---

## 1. 背景与诊断:HTTP → RPC 的演进包袱

项目最初的数据获取是 **HTTP 直连**(gateway_sdk:搜索/翻译/IPO/日历经数据网关;
行情走 AlpakaHub 直连 WebSocket)。server 化之后,这些通道全部收敛为**一条 TCP
连接上的 RPC + 推送**,但架构形态没有跟着收敛,遗留了以下包袱:

### 1.1 RPC 变体 = 面板一对一

`RpcRequestPayload` 的每个变体几乎对应一个面板的一次请求
(`News`→StockNews/MarketNews、`EconomicCalendar`→经济日历、
`EarningsCalendar`→财报日历、`IpoCalendar`→IPO、`ScreenerExport`→个股筛选器、
`MarketScreenerExport`→市场筛选器、`Bars`→图表/筛选器图表、`Search`→搜索对话框、
`Translate`→翻译对话框、`Snapshot`→ETF 联价器)。**每加一个功能 = protocol 加
变体 + server 加 handler + client 加调用点**,三处联改。

### 1.2 数据形状耦合上游:两套加工风格并存

- **Calendar 族已经 server 加工**:`EconomicItem`/`EarningsItem` 是强类型 DTO
  (`Option<f64>` 等),client 直接显示;
- **Screener 族还是上游原始形状**:`ScreenerExport { headers, rows: 原始字符串 }`,
  client 端 delegate 自己解析数值(`millions`/`dollars`/`signed_pct`,见
  `datatable/calendar/earnings.rs`)。

同一份 Finviz 数据,两种加工位置;HTTP 时代的"client 解析导出文件"习惯被原样
搬进了 RPC。

### 1.3 每个面板重复"拉取编排"三件套

generation 防乱序、离线静默跳过、错误进消息中心 —— 这套逻辑在三处各写一遍:

- 表格:`datatable::load_rows`(统一了,✓);
- K 线:`kline::load_history`(统一了,✓);
- **MarketNews 的 `backfill`**(不用 load_rows:按 id merge 防丢推送)、
  **MarketScreener 的 fetch**、**筛选器图表的 ensure_charts** —— 各自手写。

### 1.4 推送与拉取双通道在面板手工接线

每个"活数据"面板都要自己接:`SymbolWish` 订阅意愿 → `spawn_filter_stage` 过滤泵
→ `cx.spawn` 循环 → batch 处理 → `set_active` 启停 → `on_release`/`on_removed`
双保险退订 → `Connected` 重连补拉。**约 80 行/面板的生命周期样板**,且 pump 命名、
channel 容量、Lagged 处理逐面板微差。

### 1.5 面板直接依赖全局与协议

面板 `use` 里直接出现 `protocol::{RpcRequestPayload, …}`、`AppState`、`hub`、
`settings`——UI 状态与数据编排杂糅在同一个 impl 里,无法单测、无法复用到
非面板场景(例如把 K 线卡放进将来的看板)。

### 1.6 数据语义不统一

- K 线:Chart 面板**实时聚合**,筛选器 Chart 视图**静态历史**;
- MarketScreener 的交易所/最小量规则在 **client 端对全量 quote 流过滤**
  (`spawn_filter_stage` + `apply_quote(min_size)`)——规则命中与否只有客户端
  知道,上游流量全部白拉;
- Watchlist 的"最新价"来自 Trade 推送、Quote 面板来自 Quote 推送——两个
  "价格"通道没有统一语义层。

---

## 2. 现状盘点:12 个面板逐个详述

> 路径前缀 `crates/client/src/`。"控件位置"按 dock 面板从左到右/上到下描述。
> delegate 全部在 `widgets/datatable/` 同名文件。

### 2.1 WatchlistPanel(`panels/watchlist.rs`,501 行)— 自选行情

| 项 | 现状 |
|---|---|
| 控件 | 整幅表格;toolbar:搜索按钮(`open_search_symbol_dialog`→`add_symbol` 写 settings);title_suffix:200px 组选择 StringPicker(footer:Plus 新建组 / Trash 删组+确认框);行 Ctrl+点击删除标的(delegate `render_tr` 内) |
| 数据 | 无 RPC;推送 `Trade`(最新价/时间)+ `DailyBar`(开盘)——**订阅 TRADES\|DAILY** |
| 触发 | `new` 建组→refresh_pump;hub Connected→重建 pump;observe_global(settings 变更→refresh+picker 同步);`set_active` 启停订阅与 pump |
| 状态 | `picker/table/active/subscriber/spec/pump`;dump 存 `group` |
| delegate | `datatable/watchlist.rs`:5 列(代码 72 / 最新价 76 / 涨跌 76 / 涨跌幅 80 / 时间 84),数字右对齐,涨跌三色;`quotes: BTreeMap<符号, Quote>` 按组保留历史;`reload_from_settings` 跟随设置 |
| 耦合 | hub 意愿 `trades:true`;`on_release`/`on_removed` 退订;settings.watchlists 为共享源 |

### 2.2 StockNewsPanel(`panels/news/stock.rs`,173 行)— 个股新闻

| 项 | 现状 |
|---|---|
| 控件 | 整幅表格;toolbar:搜索按钮 + 刷新按钮;tab 标题显示已选 symbol |
| 数据 | RPC `News{symbols:[symbol], limit:50}`;经 `panels/news.rs::server_rpc` 守卫 + `load_rows` |
| 触发 | `new`(有持久化 symbol 即拉)、`reload_on_connect` 补拉、refresh |
| 状态 | `table`;dump 存 `symbol` |
| delegate | `datatable/news/stock.rs`:3 列(标题 480 / 来源 88 / 更新时间 88);行点击 = 翻译对话框,Ctrl+点击 = 开原文 URL;实现 `Loadable` |
| 耦合 | `observe_global_in` 仅重翻译表头 |

### 2.3 MarketNewsPanel(`panels/news/market.rs`,223 行)— 市场新闻

| 项 | 现状 |
|---|---|
| 控件 | 整幅表格;toolbar:刷新按钮 |
| 数据 | 历史 = RPC `News{symbols:[], limit:50}`(**自写 backfill**:按 id `merge_articles` 去重防丢推送,不用 load_rows);实时 = `news_messages()` 推送泵常驻(Lagged 仅 warn) |
| 触发 | `new` attach + hub Connected→backfill;refresh |
| 状态 | `table/pump`;**无 dump** |
| delegate | `datatable/news/market.rs`:4 列(标题 560 / 相关标的 110 / 来源 88 / 时间 88);行点击同 2.2 |
| 耦合 | observe hub(服务端全局新闻推送,无符号订阅) |

### 2.4 QuotePanel(`panels/quote.rs`,354 行)— 单票行情

| 项 | 现状 |
|---|---|
| 控件 | **两个三列表格并排**:左 Bid、右 Ask,列 = 价格/数量/MPID;toolbar:搜索按钮 |
| 数据 | 推送 `Quote`(`Side::take`:price>0 才有效;断线/切标的清空回破折号);**订阅 QUOTES** |
| 触发 | hub Connected→pump;`set_active` 启停;`select_symbol` 切换清空 |
| 状态 | `bid_table/ask_table(QuoteSideDelegate)/symbol/active/subscriber/spec/pump`;dump 存 `symbol` |
| delegate | `datatable/quote.rs`:`QuoteSideDelegate`(Bid/Ask 两实例,价格按方向着色:Bid 涨色 / Ask 跌色);单行表,空值破折号 |
| 耦合 | hub 意愿 `quotes:true` |

### 2.5 TradesPanel(`panels/trades.rs`,307 行)— 逐笔成交

| 项 | 现状 |
|---|---|
| 控件 | 整幅表格,4 列(价格 76 / 数量 72 / MPID 76 / 时间 96,右对齐,时间列最后);toolbar:搜索按钮 |
| 数据 | 推送 `Trade`,**命中日志**:`MAX_TRADES=500` 上限,最旧滚出;订阅 TRADES |
| 触发 | hub Connected→pump;`set_active` 启停 |
| 状态 | `table/active/subscriber/spec/pump`;dump 存 `symbol` |
| delegate | `datatable/trades.rs`:实现 `fit_column` 自适应列宽;空态三态提示(未选/未连接/等待成交) |
| 耦合 | hub 意愿 `trades:true` |

### 2.6 StockScreenerPanel(`panels/screener/stock.rs`,652 行)— 个股筛选器

| 项 | 现状 |
|---|---|
| 控件 | 内容顶部 **TabBar(表格 \| K 线,segmented)**,Chart 模式下旁挂周期下拉(`kline::timeframe_menu`);Table 模式 = 整幅表格;Chart 模式 = `v_virtual_list` 纵向列表,每项 260px(ticker 名 + K 线卡) |
| 数据 | RPC `ScreenerExport{parameter}` → 原始 CSV 行;Chart 视图:可见范围 `defer` 触发 `ensure_charts` → 懒建 `KlineState::new(tf)` + `set_symbol(ticker)` + `kline::load_history`(静态历史,无实时);**Ticker 列门槛**:header 无 Ticker → Chart tab 不渲染并回落 Table |
| 触发 | `new`(持久化 preset 即拉)、`reload_on_connect`、observe_global(settings 同步)→ fetch、observe table → `sync_chart_set`(结果集变化清空图表) |
| 状态 | `picker/table/view_mode/chart_timeframe/charts: BTreeMap<ticker, Entity<KlineState>>`;dump 存 `preset` |
| delegate | `datatable/screener/stock.rs`:动态列(headers→Column,换列须 `TableState::refresh`);`ticker_column()/tickers()` 提取 |
| 耦合 | settings.finviz.screener_parameter;hub 无自身订阅(图表走 RPC 静态历史) |

### 2.7 MarketScreenerPanel(`panels/screener/market.rs`,783 行)— 市场筛选器

| 项 | 现状 |
|---|---|
| 控件 | 空 preset → 空态提示;否则**水平滚动多列**:每个交易所一竖列(160px,简称标签 + 两列表格 代码/挂单量);toolbar:刷新按钮;title_suffix:200px preset StringPicker(footer:Plus/Pencil/Trash 三个 PickerAction) |
| 数据 | RPC `MarketScreenerExport`(固定 No.+Ticker 两列)→ ticker 池(`pool_limit()` 截断);实时 = **Quote 推送 + client 端规则过滤**(`apply_quote`:bid/ask 交易所 ∈ 用户选集 且 数量 ≥ min_size 才进对应交易所表的命中日志,`MAX_HITS=400`) |
| 触发 | `new` resync+pump、hub Connected→重置连接态+pump+fetch、settings 变更→resync(diff)、refresh、`set_active` |
| 状态 | `picker/tables: Vec<Entity<TableState>>/exchanges/preset/loaded 快照/pool/active/subscriber/spec/pump/fetch_generation/pending_resync`;dump 存 `preset` |
| delegate | `datatable/screener/market.rs`:`ExchangeTableDelegate` 每交易所一个实例(2 列;命中日志新→旧);render_empty 三态 |
| 耦合 | hub 意愿 `quotes:true`(订阅整个池的 quote,**规则过滤在客户端**);settings.market_screener.presets + finviz preset 引用 |

### 2.8 ChartPanel(`panels/chart.rs`,442 行)— 图表(K 线)

| 项 | 现状 |
|---|---|
| 控件 | 整幅 K 线(蜡烛+成交量+十字光标+最新价线+时间/价格轴);title_suffix:**周期下拉**(`kline::timeframe_menu`);toolbar:搜索按钮 |
| 数据 | RPC `Bars{symbols, timeframe, limit}`(server 端 desc+截断,升序回);实时:`DailyBar`(日/周/月/年桶聚合)/`MinuteBar+UpdatedBar`(分钟桶聚合)——**订阅随周期切换**(分钟周期多 `bars` 意愿) |
| 触发 | `new`(持久化 symbol+timeframe 即拉)、`reload_on_connect`、select_symbol/select_timeframe |
| 状态 | `kline: Entity<KlineState>`;dump 存 `symbol`+`timeframe` |
| delegate | 无表;`widgets/kline/`(state/element/scale,详见该模块) |
| 耦合 | hub 意愿 `trades:true` + (分钟周期时) `bars:true` |

### 2.9 EconomicCalendarPanel(`panels/calendar/economic.rs`,199 行)— 经济日历

| 项 | 现状 |
|---|---|
| 控件 | 整幅表格;toolbar:刷新按钮;title_suffix:**周选择 DatePicker**(禁用周一外所有日) |
| 数据 | RPC `EconomicCalendar{date: 周一}` → **强类型** `EconomicItem`;行 tint_upcoming(未来行淡染);事件列 = Button(按 impact 着色),点击 → 翻译对话框 |
| 触发 | `new`(持久化周或本周)、DatePicker Change、`reload_on_connect` |
| 状态 | `table/date_picker`;dump 存 `date` |
| delegate | `datatable/calendar/economic.rs`:6 列(时间/事件/对象/实际/预期/前值) |
| 耦合 | observe_global 仅重翻译 |

### 2.10 EarningsCalendarPanel(`panels/calendar/earnings.rs`,200 行)— 财报日历

| 项 | 现状 |
|---|---|
| 控件 | 整幅表格;toolbar:刷新按钮 + title_suffix:**日选择 DatePicker**;**普通表格**(2026-11 起:仅日期列 `MM-DD`,去掉按时到达的行背景色) |
| 数据 | RPC `EarningsCalendar{date}` → 强类型 `EarningsItem`;单元格着色保留(实际值青色、涨跌百分比红绿——数值语义,非时间语义) |
| 触发 | `new`(持久化日期)、DatePicker Change、`reload_on_connect` |
| 状态 | `table/date_picker`;dump 存 `date` |
| delegate | `datatable/calendar/earnings.rs`:11 列(日期/代码/公司/市值/EPS 预估·实际·超出/营收预估·实际·超出/1 日反应),数值解析在 client(`millions/dollars/signed_pct`) |
| 耦合 | observe_global 仅重翻译 |

### 2.11 IPOCalendarPanel(`panels/calendar/ipo.rs`,101 行)— IPO 日历

| 项 | 现状 |
|---|---|
| 控件 | 整幅表格;toolbar:刷新按钮 |
| 数据 | RPC `IpoCalendar`(iposcoop 服务端爬取),`load_rows` |
| 触发 | `new` 立即拉、`reload_on_connect` |
| 状态 | `table`;无 dump |
| delegate | `datatable/calendar/ipo.rs`:7 列纯文本(公司/代码/承销商/发行股数/价格区间/预期日期) |
| 耦合 | observe_global 仅重翻译 |

### 2.12 ExperimentPanel(`panels/experiment.rs`,53 行)— 实验占位

空壳(标题栏 + 占位文本),`all()` 里保留用于布局往返。

---

## 3. 目标架构

### 3.1 分层原则

```
┌────────────────────────────────────────────────────┐
│ ui/panels    面板 = 组件组装 + UI 状态              │  不再直接 import
│              (不碰 protocol/hub/settings 内部)      │  protocol/hub
├────────────────────────────────────────────────────┤
│ ui/components 复用组件(见 3.3 清单)                │
├────────────────────────────────────────────────────┤
│ data/        数据层(新增):唯一数据出口            │  面板唯一依赖
│  ├ api.rs      类型化查询:每个 RPC 一个 pub fn     │
│  │             (generation 防乱序/离线静默/错误上报  │
│  │              统一在这层实现)                      │
│  ├ stream.rs   订阅:每面板一个 StreamHandle(意愿    │
│  │             收发/重连重放/退订封 inside)          │
│  └ format.rs   数值/时间格式化(millions/dollars/    │
│                signed_pct/format_ts_local/format_size)
├────────────────────────────────────────────────────┤
│ server_client(现 utils/server.rs)+ hub(现 state/hub)│  只被 data 层引用
└────────────────────────────────────────────────────┘
```

规则(重构后由 clippy/评审守):

1. `panels/`、`ui/components/` **禁止出现** `protocol::`、`ServerClient`、
   `AppState::hub`——一切数据经 `data/` 的类型化 API;
2. `data/` 层无 UI 类型(不进 gpui 面板状态),错误统一返回
   `Result<T, DataError>` 并自动进消息中心(可关);
3. 每个面板的"拉取编排"(loading/generation/离线跳过)收敛到 data 层的
   一个泛型原语:`DataRequest<T>`(代数 + 状态回调),替代三处手写。

### 3.2 目录结构(目标)

```
client/src/
├── lib.rs / main.rs            # 不变
├── panels/                     # 面板(逐步迁移;每迁一个,import 换 data 层)
│   ├── watchlist.rs …          # 结构不变,数据调用换 data::
├── data/                       # ★ 新增
│   ├── mod.rs                  # DataError / DataRequest<T> / retry+offline 策略
│   ├── api/                    # 类型化查询(每 RPC 域一个文件)
│   │   ├── bars.rs             #   history(symbol, tf, limit) -> Vec<Bar>
│   │   ├── news.rs             #   for_symbols(symbols, limit)
│   │   ├── calendar.rs         #   economic(week) / earnings(day) / ipo()
│   │   ├── screener.rs         #   export(parameter) / market_export(parameter)
│   │   ├── snapshot.rs         #   for_symbols(symbols)
│   │   └── text.rs             #   search(keyword, lang) / translate(from,to,text)
│   ├── stream.rs               # 订阅封装(封 hub.set_symbols + pump + 退订)
│   └── format.rs               # 数值/时间格式化(从 delegates 抽出)
├── ui/
│   ├── components/             # 复用组件(3.3)
│   └── (panels 保留原位或整体挪入 ui/,二选一,倾向保留 panels/ 减少扰动)
└── widgets/                    # 通用件(DataTable/kline/dialog/picker/menu)不变
```

### 3.3 可复用组件清单(从现有代码提取)

全部组件 = **GPUI Kit 组件的组合**;唯一例外是 `Kline`(自绘,见 §3.5-A)。

| # | 组件 | 来源(现状) | 复用场景 |
|---|---|---|---|
| 1 | `DataTable<T>` + `Loadable` + `load_rows` | widgets/datatable.rs(已有) | 全部表格面板 |
| 2 | `DataPage<D>`(**新提取**) | 把 generation/is_loading/empty/error 三态收进一个状态结构,配 `error_view` 补齐 loading/empty | 表格/图表/新闻统一三态 |
| 3 | `Kline` + `KlineState` + `wheel_zoom`/`with_id`(**全项目唯一自绘**) | widgets/kline(已有) | Chart 面板、筛选器图表 |
| 4 | `KlineCard`(**新提取**,GPUI Kit 组合) | 筛选器 Chart 视图的内联卡(ticker 名 + Kline + 懒加载句柄 + Spinner) | 筛选器图表、将来的看板/自选弹窗 |
| 5 | `timeframe_menu(current, on_pick, menu)` + `timeframe_item_label` | widgets/kline.rs(已泛化) | Chart 面板、筛选器图表 |
| 6 | `SymbolSearch` | `open_search_symbol_dialog`(已有;与 StringPicker 双形态并存) | 自选/逐笔/行情/新闻 |
| 7 | `pinned_date_picker`(周/日两种禁用策略) | calendar/common.rs(已有) | 经济/财报日历 |
| 8 | `persist_window_dock(handle, cx)` | windows.rs(已有) | 任何面板状态落盘 |
| 9 | `spawn_filter_stage`(推送过滤) | utils/state/stream_dispatch.rs(已有) | watchlist/quote/trades/market |
| 10 | `loading_view` / `empty_view` / `error_view`(**补**) | datatable.rs(前两个已有) | 全部数据面板 |
| 11 | `news_row` 点击行为(翻译/开原文) | datatable/news.rs(已有) | 两个新闻面板 |
| 12 | `format_size/format_quote/millions/dollars/signed_pct/format_ts_local/format_date_local` | 分散在 delegates → 收进 data/format.rs | 全部展示位 |

### 3.4 server 侧配套(与 client 重构同步)

1. **ScreenerExport 强类型下沉**:server 端按 preset 的列定义把 CSV 行解析成
   `Vec<Record>`(数值/日期/枚举列在 server 定型),`ScreenerExport` 载荷改为
   `headers + Vec<Vec<Cell>>`(`Cell::{Text, Number, Percent, Money, Date}`);
   client 删除 `millions/dollars/signed_pct` 等解析(排序/筛选也能用真实类型);
2. **MarketScreener 规则下沉(可选,二期)**:订阅请求带上
   `{exchanges, min_size}` 规则,server 订阅侧过滤后只下发命中——客户端流量
   从"全池 quote"降到"命中行";client 端 `apply_quote` 过滤退役;
3. **统一 K 线订阅语义(可选,二期)**:`BarsSubscribe{symbol, timeframe}` 服务端
   聚合推送,替代 client 端分钟聚合(把 state.rs 的桶聚合搬去 server)——代价是
   server 常驻每个活跃标的的聚合状态,视使用情况决定。

### 3.5 两条硬性工程约束

**A. 布局:少用 flex 嵌套;控件一律 GPUI Kit 组件,K 线是唯一自绘**

taffy 每个节点都是一次布局计算,flex 链越深、节点越多,每帧成本线性上涨;
而且深层 flex 的对齐语义(stretch/center/auto)正是历史上多处视觉 bug 的来源。
规则:

1. **控件一律用 GPUI Kit 的组件与容器**(div/flex + Button/TabBar/DataTable/
   Spinner/…),**不写新的自绘 Element**——`Kline` 是全项目唯一的自绘组件
   (蜡烛/量柱/轴/十字光标必须逐像素控制,自绘是它的实现需要,不是通用手段);
2. 面板 UI 树的 **flex 容器嵌套 ≤ 3 层**;超过就说明该区域应封装成一个
   GPUI Kit 组件组合(一个函数/结构返回组装好的元素,内部封装布局细节,
   使用点只出现一次);
3. **固定尺寸优先于弹性**:卡片高 260px、列宽、卡片宽这些已知的尺寸直接
   给定值,让 flex 只处理"剩余空间怎么分"这一件事;
4. 大列表滚动用 `v_virtual_list`(GPUI Kit 组件)承载,条目 = GPUI Kit
   组件组合;不为性能再造自绘列表;
5. 现有反例(重构时处理):Quote 面板逐行 pill、面板标题栏多层 h_flex 嵌套;
   K 线卡在GPUI Kit 组件之上组合(不改成自绘)。

**B. 模块组织:禁止 mod.rs**

统一 Rust 2018 风格:`foo.rs`(模块文档 + 子模块声明 + 对外入口)+
`foo/`(子模块实现)。已执行:`kline/mod.rs` → `kline.rs` + `kline/`。
新目录一律照此;评审遇 `mod.rs` 直接打回。

**B. 模块组织:禁止 mod.rs**

统一 Rust 2018 风格:`foo.rs`(模块文档 + 子模块声明 + 对外入口)+
`foo/`(子模块实现)。已执行:`kline/mod.rs` → `kline.rs` + `kline/`。
新目录一律照此;评审遇 `mod.rs` 直接打回。

---

## 4. 逐面板迁移要点(渐进,每步行为零变化)

| 面板 | 迁移动作 | 依赖阶段 |
|---|---|---|
| Watchlist | 订阅改 `data/stream.rs` 句柄;着色/时间格式换 `data::format` | 1 |
| StockNews | `News` RPC 换 `data::api::news::for_symbols`;行点击行为提取 `news_row` 组件 | 1 |
| MarketNews | backfill+推送合并为 `data/stream.rs` 的 `NewsFeed`(merge 语义 inside) | 1 |
| Quote | 双表保留;订阅换 stream 句柄;`Side::take` 规则进 data 层 | 1 |
| Trades | 同 Quote(单表) | 1 |
| StockScreener | Tabs/虚拟列表保留;导出换强类型(阶段 2 后删 client 解析);`ensure_charts` 换 `KlineCard` + data 层 `bars::history` | 1→2→3 |
| MarketScreener | 先迁 data 层;规则下沉为二期(需 server 订阅侧过滤) | 1→(2)→4 |
| Chart | `load_history` 已共享;订阅换 stream 句柄 | 1→4 |
| 经济/财报日历 | 已强类型,仅换 data 层调用 | 1 |
| IPO | 同上 | 1 |

---

## 5. 风险与守则

1. **行为零变化起步**:阶段 1 纯搬运(调用点换 API),不改任何渲染/交互;
   每阶段 `cargo test -p client --lib` + 手测清单(连接→各面板数据→重启恢复);
2. **测试锚定**:server 的 rpc 行为有测试(Bars desc/limit、Calendar 加工);
   client 迁移时为 `data/` 层补同等单测(离线静默、代数丢弃、格式化);
3. **i18n**:key 迁移必须两份 locale 同步,`critical_keys_translate` 守卫会报;
   改 YAML 后 touch 任一 `.rs`;
4. **DOCK_VERSION**:面板 dump 结构不变则不 bump;改 `PanelInfo` 结构才需要;
5. **serde 兼容**:`Settings` 全结构级 `#[serde(default)]`,新增字段安全,
   改类型/枚举名不安全(整个文件回退默认);
6. **图形陷阱清单**(必读,均已踩过):见 `.agents/skills/zensidus-client/`
   的 kline 条目——虚拟列表条目宽度退化、clamp 区间反转、滚轮消费、
   多实例元素 id、坐标语义、Dashed 边框剔除、面板 update 内禁止同步 dump;
7. **布局/模块两条硬约束**(§3.5)从迁移第一笔提交开始执行:新代码 flex
   嵌套超 3 层或出现 `mod.rs` 直接打回,不欠新债。

---

## 6. 不做(明确排除)

- 不引入外部状态管理/响应式框架(保持 gpui 实体模型);
- 不做 big-bang 目录大挪移(渐进迁移,每步可编译可回退);
- 不改线协议帧格式(postcard 按位置编码的字段序),只增改 DTO 层;
- 不在 client 引入本地数据库/文件缓存(`settings.json` 之外的持久化暂不需要)。
