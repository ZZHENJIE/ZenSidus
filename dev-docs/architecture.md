# GUI 架构速览

这页回答"程序是怎么跑起来的、东西都放在哪、谁依赖谁"。读完它再看两篇教程会顺很多。

## 目录地图

```
crates/client/src/
├── lib.rs               # i18n! 声明 + bootstrap()(目录/日志/settings/locale)
│                        # + init_app()(gpui_kit::init / 主题 / MessageLog / http / AppState)
├── main.rs              # bootstrap → init_app → panels::init → windows::init → RelayHub::auto_connect
├── panels.rs            # Panel trait + PanelSpec 总表 all() + open_in_center / fresh / refresh_button
├── panels/              # 每个面板一个文件:UI 壳 + 订阅/生命周期;
│                        # calendar / news / screener 三族各一个模块根 + 子目录
├── widgets/datatable/   # 表格 delegate(面板的数据侧),按族分目录同 panels/
│                        # (calendar/{economic,earnings,ipo}、news/stock、screener/stock、watchlist)
│                        # + Loadable / load_rows / loading_view / empty_view
├── widgets/dialog/      # 对话框:confirm(通用确认 + dialog_footer)/ translate / search_symbol / screener_preset
├── widgets/picker.rs    # StringPicker:单选 Combobox 壳(Watchlist 分组 / Screener 预设共用)
├── widgets/menu/        # 标题栏下拉菜单(app / panels / windows)及其动作安装器
├── windows.rs           # WindowType / WindowState / WindowRegistry / 持久化 / 启动恢复
├── windows/app_window.rs# AppWindow trait:open_instance(Root 包装)/ close / 自绘标题栏壳
├── windows/dock_window.rs # DockWindow trait:dock 创建 / 恢复 / LayoutChanged 即时落盘
├── windows/{main,settings,custom,small_clock,etf_linkcalc}.rs  # 五种窗口本体;main/ 含退出对话框
│                        # 与状态栏,settings/ 五页,etf_linkcalc/ 含窗口私有的参数对话框
└── utils.rs             # app_dir + tr! 宏;子模块:settings / theme / state(AppState + relay_hub
                         # + audio + tokio runtime)/ relay(RelayClient)/ message / icons / assets /
                         # logging / actions
```

数据文件在 `~/.zensidus/`:`settings.json`(**全部**持久化状态:主题、语言、
凭据、watchlist 分组、筛选预设、每个窗口的几何和 dock 布局 cache)和
`logs/YYYY-MM-DD.log`(每日滚动,排查问题先看它)。

## 启动顺序(`main.rs`)

```rust
let (settings, _log_guard) = bootstrap()?;          // 1. 目录、日志、读 settings、set_locale
app.run(move |cx| {
    let window_state = settings.window_state.clone();
    init_app(settings, cx);                          // 2. gpui_kit::init、主题、MessageLog、http client、AppState 全局
    panels::init(cx);                                // 3. 把所有面板的构造器注册进 PanelRegistry
    windows::init(cx, &window_state);                // 4. 按持久化记录恢复窗口(主窗口最先)
    utils::state::RelayHub::auto_connect(cx);        // 5. relay 配置齐全就连一次 relay-server
});
```

顺序是有讲究的:窗口打开时会往 `AppState` 写几何,所以 `AppState` 必须先于
`windows::init`;窗口恢复布局时要按名字构建面板,所以 `panels::init` 也必须先于它;
`auto_connect` 放最后是为了让状态栏能收到连接结果的消息。

要单独截某个窗口的渲染时,可以临时在 `crates/client/examples/` 写一个薄壳:同一套
`bootstrap` + `init_app`,然后直接 `cx.open_window` 构造窗口并包 gpui-kit `Root`
(不走 `open()`,就不会进注册表、不会写用户的 `settings.json`),截完删掉——仓库
不保留 `examples/`。

## 四个全局(Global)

| 全局 | 在哪 | 装什么 | 怎么通知 UI |
|---|---|---|---|
| `AppState` | `utils/state.rs` | `settings`(所有持久化状态)、`relay_hub`(Entity)、音效通道 | **唯一写入口** `settings::update_settings(cx, \|s\| …)`:它用 `global_mut` 改完再保存文件,`global_mut` 会触发所有 `observe_global::<AppState>` 观察者——任何窗口/面板想"设置变了就刷新"只需订阅这个 |
| `MessageLog` | `utils/message.rs` | 有界消息环(200 条) | `message::{error,warning,info,success}` 推送即通知观察者;状态栏左侧显示最新一条 |
| `WindowRegistry` | `windows.rs` | 已打开窗口 `HashMap<WindowType, OpenWindow>` | 内部使用:单例去重、多实例分配 id |
| `PanelRegistry` | gpui-kit | 面板名 → 构造闭包 | 布局恢复和"Panels"菜单都通过它按名建面板 |

`message::error(message, detail, cx)` 有两个文本参数,**都要 i18n**:`message` 是
用户看见的,`detail` 进日志(用 `app.log.*` 键,日志跟随 UI 语言;上游错误值
`{err:#}` 作 `%{err}` 参数保持原文)。写"请求失败"当 message、把真实原因塞 detail
会让用户看不到原因——缺凭据这类情况要把"没配置,去设置页"直接写进 message。

## 面板系统一句话版

一个面板 = 一个实现了 `crate::panels::Panel`(本项目:`NAME` / `ICON` / `TITLE_KEY` /
`new(info)` / `refresh`)+ gpui-kit 两个 dock trait + `Render` 的类型。所有面板列在
`panels::all()` 这张总表里,**注册与打开动作由它驱动**("Panels" 菜单按族分组为
子菜单、族内短标签,见 `widgets/menu/panels.rs`)。面板的数据
(表格行、选中项)放在 `widgets/datatable/` 的 delegate 里,面板本身只是 UI 壳。

→ 完整步骤见 [添加新面板](guides/add-panel.md)。

## 窗口系统一句话版

`WindowType` 是窗口身份:`Main` / `Settings` / `SmallClock` 是单例,`Custom(u32)` 带
实例号可以开多个。每种窗口实现 `AppWindow`;内容是面板布局的再实现 `DockWindow`。
打开**必须**经过 `AppWindow::open_instance`:它负责"已开就聚焦"、恢复记忆的几何和
cache,以及把窗口根包成 gpui-kit 的 `Root`(没有 Root,第一次弹对话框进程就直接退出)。

→ 完整步骤见 [添加新窗口](guides/add-window.md)。

## 数据从哪来

客户端唯一的对外连接是 `utils/relay.rs` 的 **RelayClient**:一条到 relay-server
的 TCP(`protocol` 帧格式,postcard),自带自动重连(指数退避)与心跳。
协议契约在 `crates/protocol`,server 侧对接见 `crates/server/README.md`。

| 数据 | 路径 |
|---|---|
| 实时行情(trades / daily bars / quotes) | `RelayHub`(应用级唯一连接的订阅中枢)→ 每个面板自己拿一个 broadcast 接收端按 symbol 过滤。面板用 `hub.set_symbols(自己的 EntityId, SymbolWish)` 表达意愿,hub 只把**并集**的增减以 Subscribe/Unsubscribe 帧发给 relay;推送帧在 RelayClient 里映射回 anysdk 的 `StockMessage` 形状,`stream_dispatch` 级联因此与直连时代完全一致 |
| 全市场新闻 | MarketNews 面板:RPC 回填最近 50 条(按文章 id merge,不用 load_rows)+ relay 的 NEWS 推送广播实时插入(server 对所有客户端全局推新闻);泵随面板生命周期常驻,切 tab 不丢数据 |
| 新闻 / 历史bars / 快照 | RPC:`relay.rpc_checked(RpcRequestPayload::News/Bars/Snapshot)`(凭据在服务端;bars 拉取过程有 `RestProgress` 进度帧,经 `relay.progress()` 广播) |
| Finviz 筛选 / 经济·财报日历 | RPC:`ScreenerExport` / `MarketScreenerExport` / `EconomicCalendar` / `EarningsCalendar`(settings 里的 `Parameter` 经 `panels::screener_parameter_dto` 转 DTO;日历行已是毫秒时间戳的协议 DTO,server 负责 US-Eastern 解析与周窗口) |
| 代码搜索 / 翻译 / IPO 日历 | RPC:`Search` / `Translate` / `IpoCalendar`(futunn / DeepSeek / iposcoop 在 server 本地实现) |
| 网络时间 | relay 的 `Welcome.server_time_ms` + 心跳 RTT:`relay.clock()` 给出 `(offset, rtt)`,小时钟窗口消费 |

远程表格统一走 `datatable::load_rows`(闭包拿 `RelayClient` 发 RPC),见面板教程。
