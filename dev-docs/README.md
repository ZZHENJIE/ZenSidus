# ZenSidus 开发文档

基于 Rust / GPUI Kit 的 Alpaca 美股行情桌面客户端。这份文档面向**改代码的人**：
讲清楚 app crate 的结构、两条最常走的扩展路径（加面板、加窗口），以及那些
已经踩过、不想再踩一次的坑。

![platform](https://img.shields.io/badge/platform-Windows-blue) ![rust](https://img.shields.io/badge/rust-edition%202024-orange)

## 从哪里开始

| 你想做的事 | 看这里 |
|---|---|
| 先搞懂整体是怎么跑起来的 | [GUI 架构速览](architecture.md) |
| 在 Dock 里加一个新的面板（tab） | [添加新面板](guides/add-panel.md) |
| 加一个独立窗口，或者可以开多个的窗口 | [添加新窗口（含多实例）](guides/add-window.md) |
| 文案 / 设置 / 消息 / 图标 / 异步怎么写才对 | [约定与陷阱](guides/conventions.md) |

> 两篇教程里的示例代码都在当前工程里**实际编译验证过**（加进去编译、零警告，再撤掉），可以照抄。

## 仓库结构

```
crates/
├── gateway_sdk/       数据网关层:第三方数据源 → 类型化 Rust 函数
│                  (自有服务端点 search/translate/ipo/timestamp;Finviz 日历行加工)
└── app/           GPUI Kit 桌面客户端(窗口/面板系统、组件、主题、i18n、消息中心)—— 本文档的主角
```

Alpaca / Finviz 的原始 SDK 在独立项目
[AnySDK](https://github.com/ZZHENJIE/AnySDK),以 git 依赖 `anysdk` 引入
(features = ["full"])。各 crate 的 API 细节见各自
目录下的 `README.md`。

## 快速开始

```bash
cargo run -p client
```

首次运行会在 `~/.zensidus/settings.json` 生成默认配置。在 **设置 → Alpaca** 中
填入 API Key / Secret 并选择行情源(IEX 免费可用),点击"连接"即可(修改自动
保存);凭据齐全时下次启动会**自动连接**。连接状态见主窗口底部状态条。

日常开发命令:

```bash
cargo check  -p app --all-targets   # 连 examples 一起检查
cargo clippy -p app --all-targets   # 当前零警告,请保持
cargo test   -p client --lib           # 设置兼容性 / locale 一致性测试
cargo test   -p gateway_sdk             # gateway 数据加工(finviz 日历行换算等)测试
```

要求:Rust 1.85+(edition 2024),Windows(已验证)。

## 架构一览

```
┌────────────────────────────────────────────────────────┐
│  app (GPUI Kit)                                        │
│  ┌──────────────┐        ┌──────────────────────────┐  │
│  │ AppState     │        │ AlpacaHub(WS 共享订阅中枢)│  │
│  │ Global 状态   │        │ 按订阅者并集去重/断线补发   │  │
│  └──────┬───────┘        └────────────┬─────────────┘  │
│         │ executor::AlpacaClient 等三客户端│ broadcast 分发  │
│  ┌──────▼──────────┐                   │                │
│  │ panels/widgets  │◄──────────────────┘                │
│  └──────┬──────────┘                                    │
└─────────┼───────────────────────────────────────────────┘
          │ HTTP(WebSocket → alpaca_stream 内部 tokio)      │ HTTP
┌─────────▼─────────┐  ┌───────────────────────────────────┐
│ Alpaca 官方接口     │  │ gateway_sdk 聚合的第三方数据源           │
│ 行情/新闻/日历      │  │ futunn 代码搜索 · zensidus 翻译/日历│
└───────────────────┘  └───────────────────────────────────┘
```

- **anysdk**(AnySDK):Alpaca 行情 SDK(HTTP + WebSocket)与 Finviz Elite
  SDK;WebSocket 读写与断线重连全部在其内部完成,GUI 侧零感知
- **AlpacaHub**(app):应用级唯一推送流;多面板订阅同一 symbol 自动去重,断线
  由流内置 supervisor 按指数退避(1s→60s 封顶)自动重连并补发订阅,hub 只镜像
  其 StreamStatus,连接状态/成败消息直达消息中心
- **gateway_sdk**:自有服务(富途搜索 / zensidus 翻译·IPO·网络时间)→ 类型化
  Client 方法,纯 SDK 不绑定 HTTP 客户端;执行器(`ReqwestExecutor`)在 app 的
  `utils/executor.rs` 实现,与 anysdk 的两个客户端共用;Finviz 日历的
  verbatim 导出在 `gateway_sdk::finviz` 加工成带时间戳的行

## 给 AI 编码助手的说明

本项目为 AI 助手维护了 skill(`.agents/skills/`)。它们和这份文档互补:文档讲
"怎么做",skill 记录的是更细碎的 API 事实和实测陷阱。涉及对应领域时**必读**:

| Skill | 覆盖 |
|---|---|
| `anysdk` | Alpaca/Finviz SDK 用法、端点/流规范、实测坑位(认证头/推送信封/语言枚举/supervisor 流) |
| `gpui-kit` | GPUI Kit 框架的组件/GPUI 机制权威指南 |
| `gpui-kit-design-guides` | 界面设计规范 |
| `zensidus-app` | **本项目 app 代码**:窗口/面板/对话框系统、AlpacaHub、设置持久化、实测陷阱 |

## License

MIT
