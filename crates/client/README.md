# ZenSidus GUI

基于 Rust + [GPUI Kit](https://gpui-kit.com) 的行情桌面客户端。

![platform](https://img.shields.io/badge/platform-Windows-blue) ![rust](https://img.shields.io/badge/rust-edition%202024-orange)

## 功能现状

- **窗口管理**:`Main`/`Settings`/`SmallClock` 单例(重复打开自动聚焦已有窗口);
  `Custom`/`ETFLinkCalc` 可多开,实例 id 取"当前未打开的最小 id",复用 id 即恢复旧
  窗口的位置、布局与参数;Main 窗口的 `show` 写死恒为 true,下次启动必在原位置原
  布局恢复;所有窗口的位置/最大化/显示器/`cache` 持久化到 `~/.zensidus/settings.json`,
  启动时按 `show` 标志恢复。关闭主窗口弹出退出对话框:**保存**回写全部窗口几何后退出,
  **不保存**直接退出(布局本来就即时落盘,只丢窗口位置)
- **Dock 面板布局**:主窗口与自定义窗口均由可拖拽/停靠/持久化的 Panels 组成;
  面板布局(`DockAreaState`)直接存进所属窗口的 `cache`,每次布局变更即时落盘,
  崩溃也不丢;`Custom` 窗口只有中间停靠区(无左右 dock);布局版本号
  (`DOCK_VERSION`)不匹配时自动回退到新默认布局。面板自身的状态变化(如图表的
  标的/周期)也会经 `persist_window_dock` 显式落盘
- **面板总表**:12 个面板(自选/个股新闻/市场新闻/单票行情/逐笔成交/个股筛选器/市场筛选器/图表/经济日历/财报日历/IPO 日历/实验),
  `panels::all()` 一张表驱动注册;"Panels" 菜单按族分组为子菜单、族内条目用短标签;
  布局按 `Panel::NAME` 反序列化恢复,未知面板以占位形式保留并往返
- **图表面板**:纯自定义 K 线组件(蜡烛+成交量+十字光标+价格/时间轴),11 档周期
  下拉(值后带单位词),日线/周/月实时走日线推送聚合,分钟周期由分钟推送聚合;
  标的与周期持久化
- **股票筛选器**:Finviz 预设导出为表格;**Table | K 线 Tabs** 切换——K 线视图用
  虚拟滚动列表按需渲染+按需拉取每个 Ticker 的图表(需结果包含 Ticker 列),
  周期可切,静态历史
- **工具窗口**:ETF 联价器(关联参数放对话框,窗口内 ETF 现价 ⇄ 指数价格正推/反推,
  参数与输入按实例记忆);小时钟(网络时间)
- **数据接入**:全部经自建 server(唯一对外 TCP 连接,可选 TLS;TOTP 动态码 +
  会话令牌认证,断线自动重连):RPC = 筛选导出/经济·财报·IPO 日历/翻译/代码搜索/
  历史 Bars/快照;推送 = Quote/Trade/Bar(分钟+日线)/News,多面板同标的共享一条
  订阅(hub 并集聚合)
- **对话框**:代码联想搜索、翻译、筛选预设编辑(含市场筛选器)、通用删除确认、
  连接动态码(6 格 OtpInput,填满即确认);均为 `open_*(window, cx, …)` 入口
- **设置窗口**:通用(语言/主题色)、服务器(连接/地址/TLS)、市场筛选器(交易所规则)、小时钟、日志(按天查看)
- **国际化**:全部文案走 rust-i18n(zh-cn / en-us)
- **主题**:Zen Sidus 设计令牌 → GPUI 主题(唯一允许原始色值的层),settings 可覆盖
- **自绘标题栏**:空白区拖拽、Windows 原生窗口控制按钮、macOS 红绿灯
- **其他**:提示音(独立音频线程)、有界消息日志(状态栏/历史 Popover)、每日滚动日志、
  常驻 tokio 运行时(推送流指令、网络时间)

## 技术栈

| 依赖 | 用途 |
|---|---|
| [gpui-kit](https://gpui-kit.com) | GPUI 桌面框架 + 组件库 + dock 面板系统 + 内置图标 |
| [anysdk](https://github.com/ZZHENJIE/AnySDK) | Alpaca 行情/Finviz 数据的类型化形状(StockMessage 等,git 依赖) |
| [protocol](../protocol/) | 客户端 ⇄ server 线协议(postcard 帧:认证/订阅/推送/RPC) |
| tokio + rustls | 常驻运行时;到 server 的 TLS 连接 |
| rust-embed | 图标/图片/音频资源编译期嵌入(回落框架内置资源) |
| rust-i18n | 国际化(zh-cn / en-us) |
| chrono + chrono-tz | 时间处理(K 线美东时区标签/桶对齐) |
| rodio | 提示音播放 |

## 项目结构

```
src/
├── lib.rs                # i18n 注册 + bootstrap()(目录/日志/设置/locale)+ init_app()
├── main.rs               # bootstrap → init_app → panels::init → windows::init → prompt_connect
├── panels.rs             # Panel trait(NAME/ICON/TITLE_KEY + new(info) + refresh)+ 总表 all()
│                         #   + open_in_center / fresh / refresh_button
├── panels/
│   ├── calendar.rs       # 日历族入口:calendar/{economic,earnings,ipo,common}(共用件)
│   ├── news.rs           # 新闻族入口:news/{stock,market}
│   ├── screener.rs       # 筛选器族入口:screener/{stock(Table|Chart Tabs),market}
│   ├── chart.rs          # 图表面板:K 线 + 周期下拉 + 持久化
│   ├── quote.rs          # 单票行情:Bid/Ask 双表(Price/Size/MPID)
│   ├── trades.rs         # 逐笔成交表(时间列在最后)
│   └── watchlist.rs 等    # 其余面板各自一个文件,全部手写(无宏);数据在 widgets/datatable 的 delegate
├── widgets.rs            # 小元素助手(icon_button)+ 子模块声明
├── widgets/
│   ├── datatable.rs      # 表格 delegate 入口 + Loadable/load_rows/时间格式等公共逻辑
│   ├── datatable/        # 各表 delegate:trades / quote(单侧 Top of Book) /
│   │   │                 #   watchlist / screener/{stock,market} / news/{stock,market} /
│   │   │                 #   calendar/{economic,earnings,ipo}
│   ├── kline/            # K 线图组件(纯自定义):state(KlineState/Timeframe)+
│   │                     #   element(绘制+交互)+ scale(纯函数,单测);mod 含
│   │                     #   load_history(Bars RPC)与周期选择菜单,面板共用
│   ├── dialog.rs         # 跨窗口共用对话框入口:confirm(+ dialog_footer)/ search_symbol /
│   │                     #   translate / screener_preset / market_preset / auth_code(OtpInput)
│   ├── menu.rs           # 标题栏下拉菜单(app / panels / windows)与动作安装器
│   └── picker.rs         # StringPicker:分组 / 预设共用的单选 Combobox 壳
├── windows.rs            # WindowType/WindowState/WindowRegistry + 持久化/启动恢复/退出前批量回写
├── windows/
│   ├── app_window.rs     # AppWindow trait:open_instance(聚焦已开 / 恢复几何 / 根包 Root)
│   │                     #   + persist_dock_layout + close(持久化)+ 自绘标题栏与窗口控制按钮外壳
│   ├── dock_window.rs    # DockWindow trait:dock 创建/恢复/LayoutChanged 即时持久化 + reset_layout
│   ├── main.rs           # 主窗口:菜单栏 + dock(watchlist 左 / chart 中 / stock_news 右)+ 状态栏 + 退出对话框
│   ├── custom.rs         # 自定义窗口:仅中间区、可多开
│   ├── settings.rs       # 设置窗口:通用 / 服务器 / 市场筛选器 / 小时钟 / 日志 五页
│   ├── small_clock.rs    # 小时钟:网络时间(单例,PopUp)
│   └── etf_linkcalc.rs   # ETF 联价器:窗口本体 + 窗口私有的参数对话框(可多开)
├── utils.rs              # app_dir(~/.zensidus)等公共工具 + tr! 宏(t! → SharedString)
└── utils/
    ├── actions.rs        # 全局动作:OpenSettings/OpenDocs、Open* 窗口、OpenPanel { name }
    ├── exchanges.rs      # 美股交易所 MPID ⇄ 简称表(筛选器选择器/表格标题共用)
    ├── settings.rs       # Settings(语言/主题/server/翻译/自选/窗口状态)持久化,
    │                     #   update_settings 是唯一落盘入口
    ├── theme.rs          # Zen Sidus 设计令牌 → GPUI 主题
    ├── server.rs         # ServerClient:到 server 的唯一 TCP 连接(TLS/TOTP/会话令牌/
    │                     #   RPC id 路由/推送广播/时钟估计)
    ├── state.rs          # AppState 全局(设置 + 音频线程 + Hub + tokio runtime)
    ├── state/            # hub.rs(订阅并集 + 重放 + staleness 看门狗)、
    │                     #   stream_dispatch.rs(推送过滤阶段)、audio.rs
    ├── message.rs        # 有界消息日志全局(Info/Success/Warning/Error)
    ├── icons.rs          # AppIcon 图标接口(应用图标唯一来源)
    ├── assets.rs         # rust-embed 资源源(应用优先,回落框架内置)
    └── logging.rs        # 日志(每日滚动文件 + 控制台)
```

## 常见任务

图文教程见 [`dev-docs/`](../../dev-docs/README.md)(添加面板 / 添加窗口 / 约定与陷阱),
下面只列要点。

### 新增面板

1. 复制 `panels/news/market.rs` 的脚手架(数据表面板参考 `panels/watchlist.rs`),手写五件套:
   `impl Panel`(`NAME`/`ICON`/`TITLE_KEY` + `new(window, cx, info)`)、
   `gpui_kit::base::dock::Panel`(`panel_name()` 返回 `Self::NAME`,可覆写 `dump`/`set_active`/`on_removed`)、
   `EventEmitter<PanelEvent>`、`Focusable`、`gpui_kit::component::dock::Panel`、`Render`。
2. `TITLE_KEY` 对应的文案加进两份 locale。
3. 在 `panels::all()` 加一行——注册与打开动作自动跟上;再在
   `widgets/menu/panels.rs` 的菜单里补一行(族内条目用短标签)。

`NAME` 是持久化布局中的标识,**一旦发布不可更改**。需要拉远程数据的面板把
delegate 放 `widgets/datatable/`(日历/新闻/筛选器族的进对应族子目录并在族根声明)、
实现 `Loadable`,`refresh` 里调 `load_rows`。面板状态(非布局)变更后要落盘时调
`windows::persist_window_dock(window.window_handle(), cx)`(**必须 `cx.defer`**)。

### 新增窗口

1. 在 `windows/` 下新建目录,实现 `AppWindow`(`new`/`window_type`/
   `default_window_type` + `render_content`,需要标题栏内容时覆写
   `render_titlebar_left`),`new` 里调用 `hook_should_close`。
2. 内容是面板布局的再实现 `DockWindow`(`dock_area`/`build_default_layout`,
   center-only 窗口覆写 `center_only`)。
3. `windows.rs` 加 `WindowType` 变体,补齐各 `match`(标题、dock 名、默认尺寸、
   窗口种类、`init` 恢复分支——编译器会逐个报缺)。
4. `utils/actions.rs` 加 `OpenX` 动作,`widgets/menu/windows.rs` 加菜单项与 `on_action`。

单例即 `default_window_type` 返回固定变体;多开的(如 Custom / ETFLinkCalc)覆写
`open`,用 `allocate_instance_id` 分配最小空闲 id 后 `open_instance`。窗口一律经
`open_instance` 打开——窗口根在那里包成 gpui-kit `Root`,否则对话框会直接杀进程。

### 打开/关闭窗口

```rust,ignore
MainWindow::open(cx);         // 单例:已打开则聚焦
CustomWindow::open(cx);       // 多开:分配最小空闲 id,复用旧布局
ETFLinkCalcWindow::open(cx);  // 多开:同上,连参数一起恢复
SettingsWindow::open(cx);
```

关闭统一走 `AppWindow::close`:几何/显示器/cache 写回 `WindowState` 并保存,
多实例条目保留(`show=false`),下次同 id 打开时恢复。主窗口例外:关闭弹出退出
对话框,"保存"调 `persist_open_windows` 回写所有窗口几何后退出。

### 新增对话框

跨窗口共用的放 `widgets/dialog/x.rs`,写 `open_x_dialog(window, cx, …)` 并在
`widgets/dialog.rs` 导出;只服务一个窗口的放该窗口目录下(如 `windows/etf_linkcalc/params_dialog.rs`)。
输入实体在 `open_dialog` 之前创建,脚注用 `dialog_footer`,Enter 确认订阅
`InputEvent::PressEnter`(固定长度数字码用 `OtpInput`,填满即 `OtpEvent::Complete`),
宽度显式 `dialog.width(px(N))`。

### 添加图标

1. 将 SVG 放入 `assets/icons/`,统一 **lucide 线稿**:viewBox 24、`fill="none"`、
   `stroke="currentColor"`(跟随主题前景色)、`stroke-width="2"`、圆角
   (`stroke-linecap/linejoin="round"`);斜线图标在标题栏小尺寸显细,允许光学补偿
   (如 `window-close.svg` 是 2.3);无需写死 width/height,保留 viewBox 即可
2. 在 `utils/icons.rs` 的 `AppIcon` 中登记变体与路径

文件名与内置 Lucide 图标同名时自动覆盖(如 `window-close.svg`);内置回退集只有
约 106 个图标,用前查 `default-icons.txt`。

### 多语言文案

在 `locales/zh-cn.yml` 与 `locales/en-us.yml` 中同步添加 key,代码里
`tr!("app.xxx")`(得到 `SharedString`)或 `t!("app.xxx")`;占位符写 `%{name}`。
只改 YAML 不触发重编译,新 key 不生效先 `touch` 任一 `.rs`。

## 设计约定

- 界面颜色一律通过 `cx.theme()` 语义色读取,**禁止硬编码色值**;
  品牌色/语义色调整只改 `utils/theme.rs`
- 窗口管理(打开/关闭/持久化/单例)只发生在 `windows.rs` 与
  `app_window.rs`,窗口实现不碰 `settings.json`;设置落盘唯一入口
  `settings::update_settings`
- 面板 = UI 壳 + 订阅/生命周期,数据与"设置变了我的选择还在吗"的回退逻辑
  下沉到 `widgets/datatable/` 的 delegate
- 布局状态属于窗口(`WindowState.cache`),不要另建持久化文件
- 操作已打开的窗口一律 `cx.defer`(窗口自己的回调里 `handle.update` 自己会失败)
- 数据访问只经 server 连接(RPC/推送),客户端不直接触 Alpaca/Finviz 凭据

更多框架机制见 `.agents/skills/gpui-kit/`;本项目窗口/面板架构与实测陷阱
(窗口重入、DOCK_VERSION、serde 兼容、自绘图表的坐标/裁剪/虚拟列表等)见
`.agents/skills/zensidus-client/`。
