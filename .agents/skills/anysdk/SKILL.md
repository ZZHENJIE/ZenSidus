---
name: anysdk
description: 使用 AnySDK（git 依赖，Alpaca 行情 + Finviz Elite SDK）时必读。涵盖泛型 Client 的构造（执行器由使用方实现，本项目在 relay-server 的 crates/server/src/executor.rs）、alpaca_sdk 的 HTTP 端点全表与命名约定、alpaca_stream 的 supervisor 自愈流模型（StreamStatus/订阅回放/fail-fast）、finviz_sdk 的 CSV 导出与 verbatim 字符串原则、以及本项目 server 侧上游对接与实测陷阱。当任务涉及 anysdk、Alpaca 数据接入、Finviz 数据接入、行情流或日历数据时使用。
---

# anysdk：Alpaca + Finviz 数据 SDK

定位：Alpaca / Finviz 的行情数据 SDK,**独立项目**
[AnySDK](https://github.com/ZZHENJIE/AnySDK),以 git 依赖引入(App 根
`Cargo.toml`,features = ["full"])。本仓库**没有**
SDK 源码——旧 `crates/alpaca-api` / `crates/finviz-api` 已删除;SDK 的修改
去 AnySDK 仓库做。

| 模块 | 说明 |
|---|---|
| `anysdk::alpaca_sdk` | Alpaca 市场 HTTP/REST(data-only,无交易):bars / quotes / trades / auctions / snapshots / meta / news;`Feed` 枚举在此定义 |
| `anysdk::alpaca_stream` | Alpaca WebSocket 自愈流(supervisor 模型):`StockStream` / `NewsStream` / `StockMessage` / `NewsMessage` / `StreamStatus`;`Feed` 从 alpaca_sdk 重导出 |
| `anysdk::finviz_sdk` | Finviz Elite CSV 导出:screener(`Parameter`/`ScreenerType`/`EXPORT_COLUMNS`)+ calendar(economic / earnings);**全部字段保留导出原文** |
| `anysdk::utils` | 核心架构:`HttpExecutor` / `Method` / `HttpResponse` / 泛型 `Client` |

## Client 构造(本项目集成点:relay-server)

SDK 的 `Client<T, E>` 是泛型的:T 是 HTTP 客户端类型,E 是使用方实现的
`HttpExecutor<T>`。**2026-11 relay 化后,本仓库里对接 anysdk 的只有
`crates/server`(relay-server)**——客户端不再直连 Alpaca/Finviz,只持
relay-server 的一条 TCP。server 的执行器与客户端别名在
`crates/server/src/executor.rs`(reqwest 直用):

| 类型 | 位置 | 说明 |
|---|---|---|
| `server::executor::ReqwestExecutor` | `crates/server/src/executor.rs` | `HttpExecutor<reqwest::Client>` 实现,server 唯一 reqwest 落点 |
| `server::executor::AlpacaClient` | 同上 | `anysdk::alpaca_sdk::Client<reqwest::Client, ReqwestExecutor>` 别名 |
| `server::executor::FinvizClient` | 同上 | `anysdk::finviz_sdk::Client<…>` 别名 |
| 上游 WS | `crates/server/src/state.rs` | `StockStream`/`NewsStream` 各一条,推送帧映射在 `server/src/mapping.rs` |
| 日历行加工 | `crates/server/src/finviz_data.rs` | 日历行时间戳换算(US-Eastern)、周窗口参数(原 gateway_sdk 逻辑移植) |

```rust
// server main.rs:实例化一次,Arc<AppState> 共享
let alpaca = AlpacaClient::new(http.clone(), key_id, secret, ReqwestExecutor);
let finviz = FinvizClient::new(http.clone(), &finviz_token, ReqwestExecutor);
```

客户端侧消费的是 protocol 帧(见 `crates/protocol`),不直接触碰
anysdk——只引用它的消息类型(`StockMessage` 等)做形状映射。

- 客户端无 `Clone`:改凭据走 **重建整个客户端**(`AppState::sync_clients`),
  in-flight 请求持有旧 `Arc` 自然收尾
- key/secret 用标准头 `APCA-API-KEY-ID` / `APCA-API-SECRET-KEY`(SDK 内部处理)

## alpaca_sdk 命名约定(全 SDK 统一)

| 后缀 | 含义 | 响应 |
|---|---|---|
| `<域>(&Params)` | multi 单页(可能只含第一个 symbol) | `XxxPage`(map + `next_page_token`) |
| `<域>_all(&Params)` | 自动跟踪翻页拉完 | `HashMap<symbol, Vec<T>>` |
| `<域>_single(symbol, &SingleParams)` | 单标的单页 | 带 `symbol` 字段的 `Vec<T>` |
| `<域>_single_all(symbol, &SingleParams)` | 单标的拉完 | `Vec<T>` |
| `latest_<域>(&LatestParams)` / `latest_<域去掉 s>(symbol)` | 最新值(无分页) | multi = map;single = 带 `symbol` |

域方法全表:`bars` / `quotes` / `trades` / `auctions` 各 4 个 + latest 2 个;
`news` / `news_all`(HTTP 新闻,无 single 变体);`snapshots` / `snapshot`;
`meta::condition_codes` / `meta::exchange_codes`。

```rust
let page = client.bars(&anysdk::alpaca_sdk::bars::Params {
    symbols: vec!["AAPL".into()],
    timeframe: anysdk::alpaca_sdk::bars::timeframe::DAY_1.into(),  // 仅 bars 有
    ..Default::default()
}).await?;
let all = client.bars_all(&params).await?;                            // 自动翻页

let page = client.news(&anysdk::alpaca_sdk::news::Params {
    symbols: vec!["AAPL".into()],
    limit: Some(50),
    ..Default::default()
}).await?;
```

## alpaca_stream:supervisor 自愈流

连接生命周期**全部在 SDK 内部**,应用侧不写重连:

- `StockStream::connect(feed, key_id, secret)` **首连同步 fail-fast**:整个握手
  (TCP/TLS + auth)有 `alpaca_stream::CONNECT_TIMEOUT`(15s)硬超时,失败即
  `Err`;之后断线由 supervisor 后台自愈——指数退避(1s 起步封顶 60s)无限重连,
  重连成功自动**回放订阅**并清零退避
- `subscribe(&StockSubscription)` / `unsubscribe(&StockSubscription)` 增量对称,
  socket 保持;**断线期间只合并进期望状态,重连后回放**,不会失败
- `Clone` 廉价且共享同一连接,**最后一个句柄 drop = 流关闭**;显式关闭用
  `close()`(幂等)。没有 `clone_handle()` / `death_token()`——旧 API 已删除
- `status()` 返回 `StreamStatus` broadcast:`Connected` / `Reconnecting { attempt,
  delay }` / `Failed { message }`(认证失败等**永久错误**,supervisor 已停)/
  `Closed`(显式关闭)。GUI 侧消费它驱动状态灯,不自己数退避
- `messages()` 返回 `StockMessage` broadcast;服务端帧是 JSON 数组
  (`[{"T":"t",…},…]`),SDK 已按 `#[serde(tag = "T")]` 解码为强类型,未知 `T`
  跳过不断流;`{"T":"ping"}` 解码为 `StockMessage::Ping {}`
- `StockSubscription` 字段(camelCase 序列化):`trades` / `quotes` / `bars` /
  `updated_bars` / `daily_bars` / `statuses` / `lulds` / `imbalances`;订阅
  `trades` 自动启用 corrections 与 cancel/errors

## finviz_sdk:CSV 导出与 verbatim 原则

- 全部走 Finviz Elite 的导出端点(`elite.finviz.com/export/…`,认证 token 拼
  在 `?auth=`),按表头名解析 CSV 为强类型;非 2xx 直接报错(带响应体)
- screener:`Parameter { screener_type, order_by, signal, filters, columns }`
  (有 `Default`,序列化兼容旧设置的 `parameter` 字段名)、`ScreenerType`、
  `EXPORT_COLUMNS: &[(u16, &str)]`;客户端方法 `screener()` / `market_screener()`
- calendar:`economic_calendar(Parameter { from, to })`(字符串日期,SDK 不做
  周对齐)、`earnings_calendar(from: &str)`(`YYYY-MM-DD`,**不再是 `NaiveDate`**)
- **verbatim 原则**:SDK 保留导出单元格原文——时间字段是字符串、无时区换算、
  无 "All Day" 归一。美东时间戳换算与经济日历的周窗口在**本项目
  `gateway_sdk::finviz`** 完成(`economic_rows` / `earnings_rows` /
  `economic_week_parameter{,_with_start}`,有单测),GUI 消费的是
  `EconomicRow` / `EarningsRow`(带 `timestamp: i64`)

## ⚠️ 实测陷阱(违反即出 bug)

1. **严禁在 GPUI executor 上 await 阻塞/定时**:HTTP 走 executor 桥接没问题;
   流内部用 tokio task + broadcast,GUI 侧只 `recv()`,天然安全。首连握手在
   AlpacaHub 里放独立线程 `block_on`,不占 UI 线程
2. **`Feed` 枚举全端点共用**(HTTP `feed` 参数 + WS 路径),但各端点只接受子集:
   历史 bars/quotes/trades 无 `delayed_sip`/`overnight`;**WS 无 `Otc`**
   (`Feed::path()` 返回 `None`,connect 会明确报错)。不支持的值由服务端拒绝
3. **休市/周末时流上只有 subscription echo,没有任何行情**——这不是坏了,
   开盘后自然恢复;新闻 24 小时有,但几秒到几分钟一条属于正常
4. **multi snapshots 响应是裸 map**(无包装字段),single 是
   `{symbol, currency, …snapshot 字段}`(serde flatten)
5. **字段名全是单字母/缩写**(`t/o/h/l/c/v/n/vw`、`S/x/p/s/i/c/z`、`ax/ap/as/…`),
   SDK 模型内已 `#[serde(rename)]`;snapshot 是 camelCase(`dailyBar`/`latestQuote`…)
6. **Alpaca 某些端点把小数编码成 JSON 字符串**:遇新端点解码报
   "expected f64, found string" 时在 **AnySDK 仓库**加 lenient 反序列化
   (本仓库改不了 SDK)
7. **rustls 加密提供器**:alpaca_stream 的 tokio-tungstenite + rustls 已在
   SDK 内确保 provider(ring,进程级一次),GUI 进程里首次 connect 安全
8. **News HTTP 免费套餐延迟 15 分钟**(`start` 缺省时服务端只回 15 分钟前至今),
   限额 200 req/min;新闻到达频率分钟级,30s 轮询足够
9. **流调试**:`RUST_LOG=warn` 可看到被跳过的未知消息(需先初始化
   tracing-subscriber);真实验证用 `Feed::Iex` + 真实 key 手动跑

## 测试与验证

```bash
# SDK 自身测试在 AnySDK 仓库里跑
# 本项目侧:
cargo test -p gateway_sdk    # gateway_sdk::finviz 的时间戳换算 / 周窗口测试
cargo test -p client        # settings 往返(含 Feed serde 兼容)等
```

设置兼容性红线:`Feed` 与 screener `Parameter` 的 serde 格式改动会破坏用户的
`settings.json`(详见 zensidus-app skill 的设置持久化节),改前先跑
`cargo test -p client`。
