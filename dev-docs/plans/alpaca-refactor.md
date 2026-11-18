# AlpacaHub / Alpaca 连接层重构清单(交接文档)

> **✅ 已完成(2026-11 归档)**。清单主体已落地;TradeSDK 迁移后,断线重连/
> 退避/订阅回放进一步下沉到 `tradesdk::alpaca_stream` 的 supervisor,AlpacaHub
> 只镜像 `StreamStatus` 事件。文中引用的 `crates/alpaca-api/*` 路径已不存在
> (SDK 现为 git 依赖)。仅作历史记录,勿按此执行。

> 交接给新会话执行。产出日期:2026-11-09。
> 前置阅读:`.agents/skills/zensidus-gui/SKILL.md`、`.agents/skills/tradesdk/SKILL.md`(原 alpaca-api)。
> 本清单由全 crate 审阅产出,所有 file:line 与框架行为均已核实(gpui-kit 0.8.0 / gpui-base 0.8.0 / gpui-component 0.8.0 / gpui-fast 0.1.10)。

## 背景与现状架构

```
AlpacaHub (gui/src/utils/state/alpaca_hub.rs, Entity 挂 AppState, Global 语义)
├── status: AlpacaStatus { Connected, Connecting, Disconnected, Failed(SharedString) }
├── stream: Option<StockStream>          ← alpaca-api/src/stream/stocks.rs
├── subscribers: HashMap<EntityId, BTreeSet<String>>   ← 按订阅者并集去重
├── connect(): std::thread + Runtime::block_on(StockStream::connect(...)) 一次性握手线程
├── set_symbols(): 差集计算 → runtime.handle().spawn(逐条 subscribe/unsubscribe 发送)
└── auto_connect(): 启动时凭据齐全就连一次,失败不重试

StockStream (alpaca-api)
├── spawn_tasks(): writer 任务(action 泵) + reader 任务(解码 → broadcast::Sender)
├── messages(): 给每个面板一个 broadcast::Receiver(面板自旋过滤)
└── clone_handle(): drop 不断连接(任务持有 writer/read 半)
```

面板侧:Watchlist 面板 pump(`panels/watchlist.rs` refresh_pump)`recv()` 循环,`Lagged` 已处理,`Connected` 事件重挂 pump。

## 已确认的问题(按严重度)

### P0-1 死连接永远"假在线"(最严重)
`spawn_tasks` 的 reader 任务(`stream/mod.rs:103-111`)在 `read.next()` 返回 `None`(服务端关闭/网络断)时**静默退出**,不通知任何人。AlpacaHub 的 `stream: Option<StockStream>` 仍为 `Some`,broadcast sender 还活着 → 后果链:
- 状态点永远绿色 `Connected`,行情静默冻结;
- `connect()` 守卫(`alpaca_hub.rs:120`:`status == Connecting || stream.is_some() → return`)挡住重连;
- 设置页 Connect 按钮 `disabled(connecting || connected)`(`settings/alpaca.rs:85`)禁用;
- Disconnect 虽然 `take()` 有值"看起来能点",但用户不知道要这么做。
另外 stocks.rs **没有处理服务端 `{"T":"ping"}`**(被 `parse_messages` warn 后丢弃);若协议要求 pong,服务端会主动断开 → 与上一条复合成"跑着跑着就断了且不知道"。

### P0-2 握手无超时
`alpaca_hub.rs:151-153`:`std::thread::spawn` 里 `handle.block_on(StockStream::connect(...))`,`connect_async`(stocks.rs:309)无超时。TCP/TLS 黑洞(防火墙 DROP、VPN)→ 永远 `Connecting`;此时 `stream` 是 `None`,`disconnect()` 的 `take()` 无效,状态无法离开 Connecting,**只能重启**。

### P1-3 发送顺序不保证
`set_symbols` 的每条 diff 是独立 `runtime.handle().spawn` 的任务;两次快速 `set_symbols` 的 subscribe/unsubscribe 到达 wire 的顺序不定(都挤同一个 writer mpsc,谁先 spawn 谁先到不保证)→ 服务端订阅状态与 `subscribers` 可能失同步,目前只有 `tracing::warn`。

### P1-4 重连补订失败静默
连接成功后回放并集(`alpaca_hub.rs:163-165, 240-247`)失败只 warn;hub 报 Connected、面板空数据,用户无感知、无重试。

### P2-5 其它(顺手)
- `AlpacaStatus::Failed(SharedString)` 载荷是死代码:`label()` 忽略它,所有构造点传同一 `failed_message()`,真实错误只在 tracing → 状态 tooltip 永远说不清原因。
- `Feed` 改设置后不生效于活动流(旧 feed 继续流,无任何提示;settings/alpaca.rs 只在手动 Connect 时读新值)。
- 两个 tokio Runtime:AppState 一个(`state/mod.rs`,Drop 是阻塞 drop) + hub 一个(`alpaca_hub.rs:137` `Runtime::new().expect`)。失败哲学还不一致( AppState 优雅降级,hub expect)。
- hub `send()` 失败/`set_symbols` 全流程无 MessageLog 反馈。

## 重构目标(验收标准逐条对应)

### R1 连接活性检测 + 自动重连(P0-1)
- reader 任务退出(任何原因)必须**通知**持有方:在 `StockStream` 内部加死亡信号 —— 推荐 `tokio_util::sync::CancellationToken` 或 `oneshot`,reader 循环结束(含 `Err` 分支)时触发;`spawn_tasks` 返回它。
- AlpacaHub 监听信号:触发时自动 `disconnect()`(清理状态)→ 推 `AlpacaStatus::Disconnected` + MessageLog warning("连接断开,正在重连")→ 带**指数退避的自动重连**(建议 1s/2s/4s/...上限 60s;退避期间状态显示 `Connecting`,可手动 Disconnect 取消)。
- 手动 connect 与自动重连共用同一条路径;重连成功后回放并集(R4)并推 Connected。
- **手测验收**:任务管理器断网/服务端模拟断开 → ≤退避间隔内状态翻转 + 消息提示 + 自动恢复;恢复后 Watchlist 行情继续滚动。

### R2 握手超时(P0-2)
- `StockStream::connect` 的 `connect_async` 包 `tokio::time::timeout`(15s,常量放 alpaca-api);超时 → `anyhow::Err`。
- hub 侧握手线程收到 Err → `AlpacaStatus::Failed`(见 R5)→ 允许再次手动 Connect;自动重连路径按退避继续。
- **验收**:防火墙 DROP 场景(可用 `netsh` 模拟或假地址)15s 后状态变 Failed 且可重试。

### R3 发送顺序化(P1-3)
- hub 不再逐条 spawn:所有 wire 操作经 hub 内一个**有序命令队列**(推荐:hub 持 `watch::Receiver`/`mpsc` actor,或直接在 `set_symbols` 内同步算好 diff 后**一条**完整订阅消息发出——Alpaca `subscribe`/`unsubscribe` 支持列表,天然原子)。
- diff 计算保持现有并集语义(`set_symbols` 已正确,别动语义,只动传输)。
- **验收**:快速连续两次 `set_symbols`(可写单测模拟 actor 序)wire 顺序与 diff 一致。

### R4 回放失败可见(P1-4)
- 回放(或 R3 后的单条订阅)失败 → MessageLog warning + 3 次重试(退避);仍失败保持 Connected 但面板侧无数据 —— 此时消息已说明原因。

### R5 状态与 UX
- `AlpacaStatus::Failed(String)` 真正携带原因(截断到 ~120 字符),状态点 tooltip 显示;`label()` 适配。
- `Feed` 变更:设置页保存 feed 时若 stream 在线 → 提示"将重连以应用新数据源"并自动走 disconnect+connect(或明确提示需手动重连,二选一,做进设置页)。
- 停机清理:AppState 的 tokio Runtime drop 改 `shutdown_background`(对齐 hub);评估 hub 复用 AppState 的 runtime(删掉第二个 runtime)。

### R6(可选,时间允许)
- stocks.rs 增加 `{"T":"ping"}` 处理:至少不计入 warn 噪音;若查证协议要求 pong,则 writer 侧自动回 pong(README/文档确认:https://docs.alpaca.markets/docs/real-time-stock-pricing-data-STREAMING)。
- stream 层加 `last_message_at: Instant`(任何消息刷新),hub/状态条未来可显示"数据静默时长"。

## 关键文件

| 文件 | 动什么 |
|---|---|
| `crates/alpaca-api/src/stream/stocks.rs` | connect 超时、死亡信号、ping/pong、(若做)last_message_at |
| `crates/alpaca-api/src/stream/mod.rs` | `spawn_tasks` 返回死亡信号;reader Err 分支也触发 |
| `crates/gui/src/utils/state/alpaca_hub.rs` | 重连状态机(退避)、有序发送、回放重试、Failed 载荷、runtime 归一 |
| `crates/gui/src/utils/state/mod.rs` | runtime shutdown_background;客户端重建见下方"范围外" |
| `crates/gui/src/windows/settings/alpaca.rs` | feed 变更提示/触发重连;Connect 按钮状态适配自动重连(connecting 中允许取消) |
| `crates/gui/src/panels/watchlist.rs` | pump 逻辑基本不动;确认 `Connected` 事件在自动重连后照常触发 |

## 约束(项目规则,违反会被打回)

1. `cargo check -p gui --all-targets && cargo clippy -p gui --all-targets` 零警告、`cargo test -p gui --lib` 全过;改 alpaca-api 则 `cargo clippy -p alpaca-api` 也零警告。
2. `Task` 丢弃即取消:hub 内任何 spawn 出去的任务必须 detach 或存句柄;不要 `let _ =`。
3. `WeakEntity::update -> Result`;`AsyncApp::update(f) -> R`(不包 Result);全局读取用 `ReadGlobal`。
4. UI 手测归用户:完成后给编号手测清单(断网模拟建议:任务管理器禁用网卡 / 改 hosts 指向黑洞 IP)。
5. **不要动** Watchlist 订阅并集语义、`set_symbols` 的 diff 数学(已验证正确)。
6. 凭据读取:走 `settings.alpaca.credentials()`/AppState 解析好的凭据(注意:另一会话正在把密钥迁到 OS keyring,AppState 会提供解析后的凭据接口,不要直接读 settings.json 的明文字段)。

## 范围外(其它会话在做/已做)

- 密钥 keyring 迁移(**已完成**:凭据在 OS 凭据库,settings.json 无明文;UI/客户端从内存值工作);
- settings 写盘防抖/原子写、load_rows 代际令牌(**已完成**);
- **HTTP 客户端改 key 即时生效(已完成)**:`AppState::sync_clients`(`state/mod.rs`)在每次 `update_settings` 时经 `Arc::make_mut` 把内存凭据推给两个 Client(`set_token`/`set_key_id`/`set_secret`);在途请求持旧凭据完成。本重构的凭据读取走 `settings.alpaca.credentials()`/AppState 解析值,勿直读 settings.json;
- AppState tokio Runtime 的 `shutdown_background` Drop 与 hub 复用该 runtime(**已完成**,`state/mod.rs`/`AlpacaHub::new(runtime_handle)`——R5 的 runtime 归一条已落,清单剩活性检测/超时/有序发送等)。
