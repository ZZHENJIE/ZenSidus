# ZenSidus

基于 Rust 的美股行情桌面客户端(client)+ 自建行情中转服务(relay-server)。

![platform](https://img.shields.io/badge/platform-Windows-blue) ![rust](https://img.shields.io/badge/rust-edition%202024-orange)

## 结构

```
crates/
├── client/           GPUI Kit 桌面客户端(窗口/面板系统、组件、主题、i18n、消息中心)
├── protocol/   client ↔ relay-server 线协议(TCP + postcard 帧,唯一契约)
└── server/           relay-server:自建行情中转服务(VPS 常驻,见 crates/server/README.md)
```

上游数据 SDK 在独立项目 [AnySDK](https://github.com/ZZHENJIE/AnySDK)
里,server 以 git 依赖 `anysdk` 引入(features = ["full"])对接 Alpaca
(股票/新闻 WS + REST)与 Finviz Elite;搜索/翻译/IPO 由 server 本地实现。

各 crate 详见各自 `README.md`。开发者文档(docsify 站,含"添加面板 / 添加窗口"
教程)在 [`dev-docs/`](dev-docs/README.md),本地预览:`cd dev-docs && python -m http.server`。

## 快速开始

```bash
cargo run -p server -- crates/server/server.toml   # 先起服务(cp server.toml.example 填凭据)
cargo run -p client                                       # 再起客户端
```

客户端首次运行会在 `~/.zensidus/settings.json` 生成默认配置。在 **设置 → Relay** 中
填入服务器地址与 Token(TLS 视部署而定),点击"连接"即可(修改自动保存);配置齐全时
下次启动会**自动连接**。连接状态见主窗口底部状态条。客户端唯一的对外连接就是这条
relay TCP——行情、新闻与全部查询都经由它。

## 功能

- **Watchlist**:分组管理标的,实时最新价/涨跌幅(多实例可看不同分组,相同
  标的共享一条订阅);Ctrl+点击行删除标的,标题栏搜索按钮添加标的
- **Stock News**:按 symbol 展示个股资讯(标题/来源/更新时间),点击行翻译标题,
  Ctrl+点击行打开原文;选择与加载状态按面板实例持久化
- **Market News**:全市场新闻流(标题/相关标的/来源/更新时间),RPC 回填最近
  50 条,relay 推送实时插入新文章;点击行翻译标题,Ctrl+点击行打开原文
- **IPO 日历**:即将上市与近期上市公司列表(数据源 iposcoop,server 抓取)
- **经济日历**:Finviz Elite 经济事件表(当前周或日历所选周,时间本地化)
- **搜索**:全局面板内代码联想对话框(键盘 ↑↓/Enter 导航,确认即接入对应功能)
- **翻译**:对话框内中英互译(来源语言/目标语言选择持久化)
- **消息中心**:状态栏左侧实时滚动最新消息,点击查看全部历史(支持清空)
- **设置窗口**:通用(界面语言/主题配色,含行情涨跌色,即时生效)、Relay
  (服务器地址/Token/TLS/连接)、市场筛选器、小时钟、日志(按天查看)
- **ETF 联价器**:独立小窗口,在对话框里设好指数/ETF 的 Symbol、ETF 倍数与多空方向、
  当日开盘价与指数/多空偏移值;窗口内显示两个 Symbol 的实时 Bid/Ask,以及 ETF 两个
  报价档位各自的破点价格(按方向加多空偏移);可多开,每个窗口记住自己的参数与位置

## 架构一览

```
┌────────────────────────────────────────────────────────┐
│  client (GPUI Kit)                                     │
│  ┌──────────────┐        ┌──────────────────────────┐  │
│  │ AppState     │        │ RelayHub(订阅聚合中枢)     │  │
│  │ Global 状态   │        │ 按订阅者并集去重/重连重放    │  │
│  └──────┬───────┘        └────────────┬─────────────┘  │
│         │ RelayClient(RPC + 推送广播)   │ broadcast 分发   │
│  ┌──────▼──────────┐                   │                │
│  │ panels/widgets  │◄──────────────────┘                │
│  └─────────────────┘                                    │
└───────────────────────┬────────────────────────────────┘
                        │ 一条 TCP(postcard 帧,TLS 可选)
┌───────────────────────▼────────────────────────────────┐
│  relay-server(VPS)                                     │
│  上游:AnySDK → Alpaca 股票/新闻 WS + REST、Finviz Elite   │
│  本地:futunn 搜索 · DeepSeek 翻译 · iposcoop IPO         │
│  凭据(Alpaca/Finviz/DeepSeek)只存在服务端配置文件          │
└────────────────────────────────────────────────────────┘
```

- **protocol**:线上唯一契约(`[u32 LE 长度][postcard Frame]`):
  Hello/Welcome 认证对时、Ping/Pong 心跳、Subscribe/Unsubscribe 订阅协商
  (symbol 名,服务端 intern)、Quote/Trade/Bar/News 推送、RPC 请求-响应
  (bars 带翻页进度帧)
- **RelayHub**(client):应用级唯一连接的聚合层;多面板订阅同一 symbol 自动
  去重,断线由 RelayClient 按指数退避(1s→60s 封顶)自动重连、hub 重放订阅;
  连接状态/成败消息直达消息中心
- **relay-server**:per-client 意愿 → 上游并集(引用计数);推送帧对 client
  的 `stream_dispatch` 级联保持 anysdk 消息形状,UI 侧零感知

## AI 协作

本项目为 AI 编码助手维护了 skill(`.agents/skills/`),涉及对应领域时**必读**:

| Skill | 覆盖 |
|---|---|
| `anysdk` | Alpaca/Finviz SDK 用法、端点/流规范、实测坑位(server 侧上游对接) |
| `gpui-kit` | GPUI Kit 框架的组件/GPUI 机制权威指南 |
| `gpui-kit-design-guides` | 界面设计规范 |
| `zensidus-app` | **本项目 client 代码**:窗口/面板/对话框系统、RelayHub、设置持久化、实测陷阱 |

## 开发

```bash
cargo build --workspace      # 全量构建
cargo run -p client             # 运行桌面客户端
cargo clippy --workspace     # 静态检查
```

要求:Rust 1.85+(edition 2024),Windows(已验证);relay-server 另见
[crates/server/README.md](crates/server/README.md)(Linux/systemd/Nginx TLS)。

## License

MIT
