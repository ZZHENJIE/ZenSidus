---
name: zensidus-client
description: 'Architecture, conventions and hard-won pitfalls of the ZenSidus client crate (crates/client): window management (AppWindow/DockWindow, singleton vs multi-instance, WindowRegistry, persistence, gpui-kit Root wrapper, the main-window quit dialog), the dock panel system (hand-written Panel trait with DataEntity/refresh, no macro), widgets/dialog + widgets/datatable conventions, Hub shared server subscriptions + ServerClient RPC/push, status bar/message center, settings persistence, i18n/theme. Use when touching any file under crates/client — adding windows or panels or dialogs, changing window open/close/persistence behavior, wiring server data into panels, or debugging "window not found", dialogs crashing the app, layouts not restoring, or settings being reset. For the server crate see the zensidus-server skill.'
---

# ZenSidus Client (crates/client)

项目级技能:client crate 的架构决定与**实测陷阱**。GPUI/gpui-kit 框架机制
(element/entity/context/async/组件选型)见 `gpui-kit` 与 `gpui-kit-design-guides`
技能,本文件不重复;但**本 fork + 本项目已踩过**的 API 事实在下方速查节。
server crate(协议/部署/上游对接)见 `zensidus-server` 技能;上游 SDK
(Alpaca/Finviz)细节见 `anysdk` 技能。

## 项目地图

```
crates/client/src/
├── lib.rs               # i18n! + bootstrap()(目录/日志/settings/locale)+ init_app()
│                        # (gpui_kit::init/theme/MessageLog/http/AppState);main.rs 只是
│                        # 它们的薄壳(examples/ 已删除,临时截图载体用完即删,见验证清单)
├── main.rs              # bootstrap → init_app → panels::init → windows::init →
│                        # prompt_connect(lib.rs:主窗口已恢复且地址已配置时自动弹
│                        # 动态码对话框;无启动自动连接)
├── panels.rs            # Panel trait(const NAME/ICON/TITLE_KEY + new(info) + refresh
│                        # 静态钩子 + default_title)+ PanelSpec 总表 all()(驱动注册与
│                        # 菜单)+ open_in_center(按名走 PanelRegistry)+ fresh::<P>
│                        # + refresh_button;宏已删除,面板全部手写
├── panels/*.rs          # 面板本体(12 个;quote 单票行情=Bid/Ask 双表、trades 逐笔
│                        # 成交表(时间列在最后)、chart=K 线面板(kline 组件 +
│                        # 周期下拉 + 持久化)、screener/stock=Table|Chart Tabs
│                        # (Chart 为虚拟滚动 K 线列表);experiment 仍为占位):
│                        # 手写五件套脚手架;数据在 DataEntity delegate。
│                        # MarketNews = HTTP 回填 + 面板自持 NewsStream
│                        # (subscribe "*",泵随面板常驻)
├── widgets/dialog/      # 跨窗口共用的对话框:open_*(window, cx, …) 入口(confirm 通用
│                        # 确认 + dialog_footer / translate / search_symbol / screener_preset /
│                        # auth_code 连接动态码,状态条 Popover 与设置页共用)
├── widgets/kline.rs     # K 线图组件(纯自定义;chart 面板 + 筛选器 Chart 视图
│                        # 共用;**全项目唯一自绘 Element**——其余 UI 一律 GPUI
│                        # Kit 组件组合,不得新增自绘):state(KlineState 实体:
│                        # bars/viewport/分钟桶聚合/日线聚合(周/月/年))、
│                        # element(自定义 Element 三阶段绘制+指针交互;
│                        # wheel_zoom 开关、with_id 多实例)、
│                        # scale(纯几何/刻度/美东标签,单测)、kline.rs 公共入口:
│                        # load_history(Bars RPC,离线静默/代数防乱序)+
│                        # timeframe_menu(扁平菜单,值后带单位词,单位切换处
│                        # 分隔线)+ timeframe_item_label。周期 11 档:分
│                        # 1/2/3/5/10/15/30、时 1/2/4、日/周/月(Alpaca 串透传)。
│                        # 历史走 Bars RPC,实时走 DailyBar(日/周/月/年桶聚合)
│                        # /MinuteBar 聚合(分钟周期,SymbolWish.bars 意愿 →
│                        # Channels::BARS)。
│                        # ⚠ PaintQuad 的 BorderStyle::Dashed 在零高度/零宽度 bounds
│                        # 上被剔除(边框几何依赖 bounds 面积)——虚线要自绘分段
│                        # 实心 quad;Element::paint 里的 window.current_view()
│                        # 合法,但鼠标回调(事件分发期)里调用会 panic,须在 paint
│                        # 时捕获 view id 带入闭包。
│                        # ⚠ 自绘图表的指针判定不要用 Hitbox::is_hovered:dock/悬浮
│                        # 层场景下遮挡感知 hit-test 会瞬时报 false,把交互冻结在
│                        # 旧位置(实测);用元素自身的 bounds.contains(事件坐标)
│                        # 判定,事件驱动+cx.notify(view)。
│                        # ⚠ 坐标语义要一刀切:交互 cell 若存"元素相对坐标"
│                        # (e.position − bounds.origin),消费它的几何函数
│                        # (ix_at/price_at 等)就必须以元素相对为基准;若几何
│                        # 以窗口绝对为基准(内部减 bounds.origin),入口先加回
│                        # origin。origin=(0,0) 的裸窗口会掩盖混用 bug,验证
│                        # 必须用 origin≠0 的嵌套场景。
│                        # ⚠ 自动化验证时用户的真实鼠标会与 SetCursorPos 竞争,
│                        # 截图落点可能根本不是合成位置——以探针日志为准。
│                        # ⚠ v_virtual_list:条目宽度按"测量条目 0"推断,卡片不带
│                        # 显式 w_full 会退化成内容宽 → 里面的 K 线 plot_w=0 →
│                        # paint_axes 的 clamp 区间反转 panic(min>max)。K 线元素
│                        # 对退化尺寸(plot_w/h < 60)必须只画底色提前返回,
│                        # clamp 的上界一律 .max(下界) 兜底。
├── widgets/picker.rs    # StringPicker:watchlist 分组 / screener preset 共用的单选 Combobox 壳
├── widgets/datatable/   # TableDelegate 实现,按族分目录(calendar/{economic,earnings,ipo}、
│                        # news/{stock,market}、screener/stock、watchlist)= 面板的
│                        # DataEntity;Loadable + load_rows 统一"拉数据→loading→写回/
│                        # 报错"(market 是例外:merge 语义,见其面板注释);
│                        # measure_text 量宽给 fit_column
├── widgets/menu/        # 标题栏下拉菜单(app / panels / windows)及其动作安装器
├── windows.rs           # WindowType/WindowState/WindowRegistry + 持久化 + 启动恢复
│                        # + persist_open_windows(退出前批量回写几何,不 dump cache)
├── windows/app_window.rs# AppWindow trait:open_instance 里窗口根包 gpui-kit Root;
│                        # render_titlebar_left/right 钩子(右侧可放窗口级按钮)
├── windows/dock_window.rs # DockWindow trait:dock 创建/恢复/LayoutChanged 即时落盘
├── windows/main.rs      # 主窗口本体 + main/status_bar.rs(左:最新消息+历史 Popover;右:「服务器」label+连接色点,点击弹连接 Popover)
│                        # + main/close_event.rs(X → 退出对话框:取消 / 不保存 / 保存)
├── windows/settings.rs  # 设置窗口(text_setting 助手)+ settings/ 五页
│                        # general/server/market_screener/small_clock/logs
├── windows/custom.rs    # 自定义 dock 窗口(多开,center-only)
├── windows/small_clock.rs # 小时钟(单例,PopUp;网络时间走 AppState 的 tokio runtime)
├── windows/etf_linkcalc.rs # ETF 联价器(多开):symbols/当日开盘价/倍数/两偏移公式 +
│                        # 测试 + 窗口本体(实时 bid/ask 两行 + 四破点价,quotes 订阅,
│                        # 设置按钮在标题栏 render_titlebar_right);参数确认即写
│                        # cache(见"open/close 生命周期")
│                        # + etf_linkcalc/params_dialog.rs(窗口私有的参数对话框,不放 widgets/dialog)
└── utils.rs             # app_dir + tr! 宏;utils/ 子模块:
                         # server(ServerClient:唯一对外 TCP 连接 + RPC)/
                         # settings(LANGUAGES/update_settings/Server 配置)/
                         # theme / state(AppState + audio + hub +
                         # tokio_runtime)/
                         # message(MessageLog)/ icons / assets(rust-embed)/
                         # logging / actions(OpenPanel{name} + Open* 窗口动作)
```

数据层分工(2026-11 server 化后):**客户端唯一的对外连接是 `utils/server.rs` 的
`ServerClient`**——一条到 server 的 TCP(`protocol` 帧格式,postcard),
自带自动重连/心跳/RPC id 路由,推送帧映射回 anysdk 的 `StockMessage` 形状
(`stream_dispatch` 级联不变)。hub 在 `utils/state/hub.rs`(`Hub`:
`SymbolWish` 并集聚合 + 每次 `Connected` 重放订阅 + staleness 看门狗)。面板数据
一律走 RPC:`server.rpc_checked(RpcRequestPayload::…)`,远程表格经
`datatable::load_rows`(闭包拿 `ServerClient`)。旧 `utils/executor.rs` 三客户端、
独立 `NewsStream`、`gateway_sdk` 已全部退役;Alpaca/Finviz/DeepSeek 凭据在
**server 侧** `server.toml`,客户端只持一次性动态码(连接对话框输认证器上的 6 位码,
30 秒过期;密钥 totp_secret 只在服务端配置,keyring 已于 2026-11 移除)。
server 侧架构与部署见 `zensidus-server` 技能;上游 SDK 细节见 `anysdk` 技能。

数据文件(`~/.zensidus/`):`settings.json`(全部状态:主题/语言/server 地址/
translate/watchlists/window_state 含各窗口 cache;动态码不落盘)、`logs/`
(每日滚动,排查先看)。

## 窗口管理模型(改动前必读)

### 窗口根必须包 gpui-kit Root(杀进程级陷阱)

`open_instance`(app_window.rs)里 `cx.open_window` 的闭包返回
`cx.new(|cx| gpui_kit::component::Root::new(view, window, cx))`。
`WindowExt::open_dialog`/`push_notification` 经 `window.root::<Root>()` 找
overlay 插件;**裸视图根会在点击时 `expect(ROOT_MISSING)` 直接杀死进程**
(无窗口无 panic 窗口,只有 stderr)。激活/持久化路径忽略视图参数,包 Root
无副作用。新窗口一律走 `open_instance`,不要绕过。

### 类型与单例

- `WindowType`:`Main`/`Settings`/`SmallClock` 单例;`Custom(u32)`/`ETFLinkCalc(u32)`
  多开(Settings 是 `WindowKind::Dialog`,SmallClock 是 `PopUp`)
- 已开窗口登记在 `WindowRegistry`(Global,`HashMap<WindowType, OpenWindow>`,
  `OpenWindow { handle: AnyWindowHandle, view: AnyWeakView }`)
- 两个匹配语义,**职责不同,不可混用**:
  - `find_open_window` → **精确 key**(`registry.0.get(&window_type)`)。
    判断"这个具体窗口"是否已开。若改成 `same_kind` 模糊匹配,多实例会互相
    顶掉(开 Custom #2 被当成 #1 已开)
  - `allocate_instance_id` → **家族匹配**(`same_kind`,同变体任意 id)。
    枚举该类已开实例,分配最小空闲 id(复用 id = 恢复旧位置与布局)。
    若改成精确相等,已开 id 列表永远为空 → 永远分配 1
- `Main.always_show() == true`:`show` 写死恒 true(关掉也恢复);
  `update_window_state` 里有硬规则兜底

### open/close 生命周期

- `open()` → `open_instance(window_type, cx)`:registry 精确查 → 已开则
  **defer 激活**(见陷阱 1)→ 返回;未开 → 从 settings 的 WindowState 取
  bounds/maximized/display_id/cache(bounds 为零则用 `default_bounds` 居中
  默认且不最大化)→ 组装 `WindowBounds::Windowed/Maximized` →
  `cx.open_window`(根包 Root)→ 注册 registry + `update_window_state(show=true)`
- `close()`(经 `hook_should_close` 挂在 `on_window_should_close`)→
  `dump_cache` + `mark_window_closed`:`persisted_geometry`(见陷阱 8)
  回写 bounds/maximized/display_id、`show = always_show()`、从 registry
  注销、保存 settings
- **退出路径不 dump 各窗口的 cache**:`persist_open_windows` 只回写几何。
  dock 布局靠 LayoutChanged 即时落盘所以无感;窗口有随用随变的状态
  (如 ETF 联价器参数)就必须在**变更时**经 `update_window_state` 写
  cache —— 只在 close 时 dump,退出应用不关窗口就丢(ETF 联价器
  2026-11-15 修过)。close 路径的 dump_cache 保留(幂等写同值)
- **退出不经过 close,也没有托盘**:唯一退出路径是主窗口的 X →
  `main/close_event.rs` 的退出对话框(取消 / 不保存=直接 `quit` / 保存=
  `persist_open_windows` 批量回写几何再 `quit`;主窗口不挂 `hook_should_close`)。
  **"保存"必须 `cx.defer(|cx| { persist_open_windows(cx); cx.quit() })`**:按钮
  点击处理器运行在主窗口自己的 update 内,直接调会让主窗口自己的
  `handle.update` 失败(陷阱 1),结果恰好只有主窗口位置不记——2026-11-02 实测
  修过。布局 cache 平时每次 `DockEvent::LayoutChanged` 已即时落盘,所以
  "不保存"实际只丢窗口几何

### 陷阱(全部实测踩过)

1. **窗口 update 重入 → 假 "window not found" → 误开新窗口**。
   `update_window_erased` 执行时会把窗口从 arena 槽 `Option::take` 出来;
   在**同一窗口自己的回调里**(如面板按钮 on_click)对它 `handle.update`
   会拿 None → Err。日志特征:每次操作伴随 `window not found` +
   `无效的窗口句柄` 三连(`~/.zensidus/logs`)。
   修法:**激活/操作已开窗口一律 `cx.defer(move |cx| handle.update(...))`**;
   存活判断用 `AnyWeakView::upgrade()`(只读,不碰 arena),绝不用
   `update` 的成败当探针
2. **判活与匹配语义**(见上,`same_kind` 家族 vs 精确 key 曾互串出两个 bug)
3. **gpui 本 fork 的返回值与惯例不同**:
   - `AsyncApp::update(f) -> R`(不包 Result;失败 panic/日志)
   - `WeakEntity::update -> Result<R>`;`App::open_window -> anyhow::Result`
   - `App::defer(f)` 存在且是 Effect;`cx.spawn` 是 `boxed_local`(不要求 Send)
   - 全局读取没有 `Global::global(cx)`,是 `ReadGlobal` trait
     (`use gpui_kit::ReadGlobal as _`);`Context::observe_global` 闭包**两参** `|_, cx|`
   - **`Task` 丢弃即取消**(platform_scheduler.rs:311 明文):`let _ = cx.spawn(...)`
     语句结束任务即被取消。一律 `cx.spawn(...).detach();`,且不要留 `let _ =`
     前缀(detach 返回 `()`,触发 clippy `let_unit_value`)
   - `gen` 是 edition 2024 保留关键字,不能当字段/变量名
   - `rust_i18n::locale()` 返回 `impl Deref<Target=str>`,`.as_str()` 走到
     str 的不稳定方法;用 `let locale = …; Lang::from(&*locale)`
4. **Settings 反序列化是全有或全无**:旧 settings.json 解析失败 →
   `Settings::load` Err → **整个配置回退默认值**(主题/语言/Key 全丢)。
   现在 `Settings` 及所有嵌套结构都是**结构级 `#[serde(default)]`**(缺字段
   取 `Default`,有测试守着),新增字段/新增嵌套结构不用再单独标;但**改
   已有字段的类型或枚举变体名**仍会让整个文件解析失败——要么加
   `#[serde(alias)]`/自定义 deserialize 兼容,要么接受回退。另外**应用运行时
   该文件被锁**,外部改不了,且退出时会被内存数据覆盖回去
5. **改默认 dock 布局必须 bump 版本**:`dock_window::DOCK_VERSION`(全局)
   或某窗口覆写 `dock_version()`(如 Custom = 3)。恢复时校验版本,不符才
   回退默认布局;不 bump 的话用户存量 cache 永远盖住新默认
6. **资源一律走嵌入 Assets**(`crate::utils::Assets::get("icons/x.svg")` →
   `EmbeddedFile.data`,回退 gpui-kit 内置集),禁止 cwd 相对路径。
   **内置回退集只有 106 个图标**(`default-icons.txt`):`IconName::FolderPlus`
   这类常量存在但 SVG 不在集里 → 按钮渲染成空白。用前查
   default-icons.txt,或把 SVG 放进 app 的 assets/icons(trash.svg 即自带)。
   音频用 `Decoder::try_from(Cursor::new(bytes))`,注意 channel 的泛型要显式标注
7. **gpui 根元素必须 `size_full()`**:窗口 root(`Render::render` 的返回)或
   `render_window` 的 v_flex 若没有 `size_full()`,高度塌缩为内容高度,窗口
   其余部分是**未绘制的清屏色(白)**,看起来像"内容区白屏/不完整"。
   特征:标题栏/tab 条正常,下面一片纯白且无任何文字。容器中间包一层裸
   `div()` 同理(要 `div().size_full()` 或改用 v_flex)。实测案例:加菜单栏时
   外层套 `on_actions(div(), cx)`,内层 v_flex 的 size_full 以它为基准归零,
   整个 dock 内容区变白。另注意 `size_full/w_full/h_full` 是 **`Styled` trait**
   的方法——trait 不在作用域时报 E0599("no method size_full"),极易误判为
   "这个类型没有该方法"。验证白屏真伪:PrintWindow(PW_RENDERFULLCONTENT)
   直接截窗口本体,CopyFromScreen 截全屏受 z-order 干扰
8. **保存窗口几何必须用 `window.window_bounds()`,禁止 `window.bounds()`**。
   `bounds()` 报告的是 WM_MOVE 的客户区原点;重开路径(`adjusted_window_rect`)
   把保存的客户区向外扩"不可见边框"且边框被**对半切分**——而 Windows 实际
   上边框是上 0 / 下 ≈7px,不对称 → 每轮 关闭→重启 Y 上移 3~4px,窗口
   逐步"往上爬"(宽度方向左右对称,所以 X 不漂)。
   修法:`windows::persisted_geometry(window)` 取 `window_bounds()`
   (平台基于 GetWindowPlacement,与恢复路径同一套数学,误差互消),
   拆出 `(bounds, maximized)`;恢复时组回 `WindowBounds::Windowed/Maximized`。
   `WindowState.maximized`(serde default)顺带持久化最大化状态。
   注意:窗口存于副屏负坐标区且贴近屏缘时,平台 `check_bounds` 会判
   "不在显示器内"而重新贴边定位——这是 gpui 的跨屏保护,与漂移无关
9. **合成点击自动化的坑(已放弃,UI 测试归用户)**:`GetWindowTextW` 对
   GPUI 窗口只读首字符('Z');主窗口几何在副屏负坐标区时 `SetCursorPos`
   静默失败、点击全部落空。**UI 上能看到的测试由用户手动做,自动化只做
   启动 + 截图验证渲染,交互测试给编号步骤清单**

## 面板系统(宏已删除,全部手写)

- **`Panel::NAME` 是持久化布局的主键,发布后不可改**;未知 name 会以
  占位面板形式往返不丢。`panel_name()` 必须返回 `Self::NAME`(注册与持久化
  用的是同一个常量,两边不一致布局就恢复不出来)
- **面板总表 `panels::all()`**(`[PanelSpec; N]`,由各类型的 `spec()` 组成):
  `panels::init` 按它注册,打开面板走**单一动作** `actions::OpenPanel { name }` →
  `open_in_center(area, name, …)` → `PanelRegistry::build_panel`(与布局恢复
  同一条路)。"Panels" 菜单在 `widgets/menu/panels.rs` 按族(news/screener/
  calendar)分组为子菜单,族内条目用短标签(省族词,如 "Economic"),引用
  面板类型常量。**新增面板 = 实现 + `all()` 加一行 + 菜单补一行**,不碰 actions
- 每个面板手写五件套(参考 `panels/watchlist.rs` 最完整):
  1. `impl Panel`(crate trait):`type DataEntity`(面板的数据实体,纯占位
     面板用 `()`)、`const NAME/ICON/TITLE_KEY`、`new(window, cx, info)`
     (**new/restore 已合并**,info 是持久化 payload)
  2. `impl gpui_kit::base::dock::Panel`:`panel_name() { Self::NAME }`;可覆盖
     `dump`(写 `PanelInfo::Panel(json!(…))`,`PanelState::new(Self::NAME)`)、
     `set_active(active)`(tab 切换:隐藏退订/激活补订的标准挂点)、
     `on_removed`(面板移除清理)
  3. `impl EventEmitter<PanelEvent>`
  4. `impl gpui_kit::Focusable`
  5. `impl gpui_kit::component::dock::Panel`:`title` 静态标签直接
     `Self::default_title()`(ICON + TITLE_KEY),动态标签(StockNews/StockScreener)
     用 `panel_title(Self::ICON, label)`;可覆盖 `toolbar_buttons`(刷新按钮用
     `refresh_button::<Self>(id, table.downgrade())`)/`dropdown_menu`/
     `title_suffix`/`tab_name`
  6. `impl Render`
- **`refresh(entity: WeakEntity<Self::DataEntity>, cx)` 是静态钩子**,语义 =
  "把数据从源重载进 DataEntity";`new` 构建时调用一次 = 启动自动加载;
  数据变化(搜索确认/设置变更)也复用它。需要额外上下文(如 symbol)就
  下沉进 delegate——refresh 只拿得到 DataEntity 的弱引用。
  **启动时 server 未连接 = 静默跳过**(不发请求、不进消息中心),表格面板
  在 `new` 里再接一行 `crate::panels::reload_on_connect::<Self>(table.downgrade(), cx)`,
  连接建立(含重连)后自动补一次加载
- **远程表格统一走 `datatable::load_rows(table, error_title, |http| async {…}, cx)`**:
  delegate 实现 `Loadable`(`type Row` + `loading_mut` + `rows_mut`),helper
  负责置 loading → spawn → 写回 rows 或 clear+`message::error`;
  **离线时 helper 静默 return**(不置 loading、不报错)。`render_loading`/
  `render_empty` 用 `datatable::loading_view()` / `empty_view(text, cx)`
- **列头本地化(语言切换即时生效)两个条件缺一不可**:`column()` **实时 `t!` 生成** Column(Watchlist/IPO/StockNews/Economic 模式;把 `t!` 快照进 `columns` 字段则切换后仍是旧串)+ 面板注册 `observe_global_in::<AppState>`(数据不依赖 settings 的面板 observer 只 `cx.notify()`)。Screener 表头来自 CSV 数据本身,语言无关
- **动态列表格**(如 StockScreener 的 CSV 导出):headers → Column,行存原始字符串;**换列后必须 `TableState::refresh(cx)`**(列组是构造时快照,见陷阱);双击列宽适配走 `fit_column` → `datatable::measure_text`
- **数据下沉**:TableDelegate 等数据实现放 `widgets/datatable/`,目录与
  `panels/` 同构(族根只做声明 + re-export):`calendar/{economic,earnings,ipo}.rs`、
  `news/{stock,market}.rs`、`screener/stock.rs`,独立面板平铺(`watchlist.rs`)。
  外部一律用 `crate::widgets::datatable::XDelegate`。面板 = UI 壳 + 订阅/生命周期;
  delegate 持有 group/symbols/quotes 这类数据,面板只发指令。**"设置变了→我的选择还在吗"
  的回退逻辑也在 delegate**(`WatchlistDelegate::reload_from_settings` /
  `StockScreenerDelegate::sync_with_settings` 返回是否变化);面板观察者的顺序固定
  为:**先让 delegate 同步,再按 delegate 的结果设 picker**(反过来 picker 会
  停在空白——已踩过)
- 生命周期清理双保险:`on_removed` + `cx.on_release`(都要幂等)
- 布局操作:`panels::open_in_center(area, name, …)`(往 center 加 tab);
  默认布局用 `panels::fresh::<P>(window, cx)` 建实例;
  `DockWindow::reset_layout`(重建默认并落盘)
- **面板状态变更要主动落盘**:dock 布局只在 `DockEvent::LayoutChanged` 时
  dump,面板自身的状态变化(如图表的标的/周期)**不触发该事件**,退出路径
  又只回写几何——状态变更处必须显式调
  `windows::persist_window_dock(window.window_handle(), cx)`(registry 里每个
  窗口带 persist 闭包 → `AppWindow::persist_dock_layout`,dock 窗口转发
  `DockWindow::persist_layout` 整树重 dump)。**调用点必须 `cx.defer`**:
  面板在自己的 update 里同步触发,dock dump 逐个 update 面板实体会撞上
  "already being updated" 重入崩溃(实测)
- **面板内的弹出选择器**(周期/开关类):`toolbar_buttons` 的 `Vec<Button>`
  契约装不下弹出锚定元素——用 `title_suffix` 钩子承载(任意 IntoElement,
  `Button::dropdown_menu(...)` 包一层即可),它渲染在标题栏工具区左侧
- **同屏多个自绘元素必须各自显式元素 id**(`Kline::with_id`):默认 id 相同
  时元素局部状态(光标/拖拽)跨实例互串
- **嵌入可滚动列表的自绘元素关掉滚轮消费**(`Kline::wheel_zoom(false)`):
  滚轮必须让位给列表滚动;列表本身要 `track_scroll(&handle)` + `flex_1().min_h_0()`
  才会响应滚轮并限制视口高度
- **股票筛选器 Chart 视图**(panels/screener/stock.rs):`v_virtual_list`
  纵向列表,每项 260px(ticker 名 + K 线卡);回调只收到可见范围——渲染
  按需,拉取也按需(`defer_in` 出帧后 `ensure_charts`:懒建
  `KlineState::new(tf)` + `set_symbol(ticker)` + `kline::load_history`)。
  卡片必须 `w_full`(虚拟列表条目宽度按"测量条目 0"推断,无显式宽度退化成
  内容宽 → plot_w=0 → clamp panic,见 kline 条目);结果 header 无 Ticker 列
  → Chart tab 不渲染并回落 Table;结果集/周期变化 → 清空全部图表状态重建;
  bars 为空的卡显示 Spinner;离线创建的卡在连接建立后自动重拉(hub 订阅)

## 全局数据与消息

- **设置落盘唯一入口**:`settings::update_settings(cx, |s| …)`(utils/settings.rs)
  —— `global_mut::<AppState>` 自动通知所有 `observe_global::<AppState>` 窗口
  (全窗口刷新零接线)+ **防抖落盘**(500ms 合并写,2026-11-09 起;真正写盘
  在 `flush_settings`,temp+rename 原子写;**settings.json 永无密钥**——
  server 端密钥只在服务端配置,客户端每次连接输 6 位动态码,keyring 已于 2026-11 移除)。
  **新增退出路径必须调
  `flush_settings`**(close_event 两按钮、`persist_open_windows` 结尾已接)。
  失败进消息中心;窗口几何/布局的 `update_window_state` 也走它。
  **Settings 及所有嵌套结构都是结构级 `#[serde(default)]`**,缺字段的旧文件
  照常加载(有测试 `partial_settings_file_loads` 守着),新增字段不用再单独标
- **文案**:`tr!("key")` / `tr!("key", name = v)` = `t!` 转 `SharedString`
  (2026-11-09 起统一:**全 crate 只用 `tr!`,不要 `use rust_i18n::t`**——
  返回类型直接是文本槽位要的 `SharedString`,免 `.to_string()/.into()` 拼接)
  (gpui-kit 所有文字槽位的类型);需要 `String` 做拼接时才用 `t!(..).to_string()`
- **凭据守卫**:RPC 消费方的在线守卫是 `AppState::server(cx) -> Result<ServerClient>`
  (server 未连接即 Err;`panels::news::server_rpc` 把它压成 `None`——离线
  静默跳过,2026-11 起离线 RPC 一律不进消息中心);`ServerStatus::{label(),
  reason(), color(cx)}` 是状态条与设置页共用的展示映射
- **MessageLog**(utils/message.rs,Global):`message::{error,warning,info,success}`
  推送即通知 `observe_global::<MessageLog>` 观察者(状态栏/主窗口零接线);
  状态栏左侧显示 `last()`,点击弹 `latest_first()` 历史 Popover。
  **Hub 连接成败会自动推 success/error 消息**
- **Hub**(utils/state/hub.rs,Entity 挂在 AppState;2026-11 起替代
  旧 AlpacaHub 直连 Alpaca WS):
  - 应用级唯一 server 连接(ServerClient,utils/server.rs);`set_symbols(EntityId,
    SymbolWish)` 按订阅者记录意愿(symbols + trades/quotes/bars 三档),**只把并集的增减
    发给 server**(trades 意愿 → TRADES|DAILY,quotes 意愿 → QUOTES,bars 意愿 →
    BARS=分钟/更新 bar;筛选器图表视图在分钟周期时置 bars)——两个面板
    同看 NVDA 只订一次
  - **订阅重放在 hub**:ServerClient 自己自动重连(指数退避)但不保存订阅意愿,
    hub 监听其 `status()` 广播,每次 `Connected`(首连与重连)重发两个并集;
    `Failed(reason)`(动态码被拒 AuthFail 等永久错误)→ Failed + teardown
  - 分发不经 hub:面板各自 `hub.stock_messages()` 拿 broadcast 接收端(已是
    anysdk `StockMessage` 形状,由 ServerClient 从协议帧映射),`cx.spawn` 循环
    recv 按自己的 symbols 过滤;RPC 走 `server.rpc_checked(...)`
  - **staleness 看门狗保留**:数据流在流而最新交易所时间戳落后本地钟 >15s →
    `server.force_reconnect()` 换新路由(订阅经 Connected 重放)
  - 连接入口 = **启动自动弹出**(prompt_connect,地址已配置时)/ 状态条色点 Popover / 设置页按钮 → **动态码对话框**
    (`widgets/dialog/auth_code.rs`,`hub.connect(code)`)——**6 格 OtpInput**
    (3+3 分组,只收数字;填满第 6 格经 `OtpEvent::Complete` 自动确认,
    Confirm 只在填满时提交)——码不落盘、每次连接输
    认证器上的 6 位码。**认证成功后 Welcome 下发会话令牌**,ServerClient actor
    在内存里持有它:之后的断线重连自动带令牌(`HelloAuth::Session`)免输码,
    服务端轮换下发新令牌。令牌失效(服务端重启 / 30 分钟 TTL / 被轮换)时服务端
    回 AuthFail → Failed 终态,重试无意义——重新点击连接输新码建链。
  - tokio Runtime 全 app 只有一个(AppState 持有,Drop 时 shutdown_background;
    构建失败则 hub 报 Failed 不 panic),hub 只存 Handle
- **状态栏**(windows/main/status_bar.rs):左 = 最新消息(级别色点 + 截断文本 +
  muted 时间;点击 Popover 历史 + Clear;无消息完全空白);右 = server 连接状态
  **「服务器」label + 色点**(label 复用 `app.settings.server`;状态文字在
  tooltip——曾按"色点 only"约定做过,2026-11-20 用户明确要求恢复 label),
  **点击弹连接 Popover**(状态行 + TCP 延迟 = 心跳 RTT,无样本显示 "—";服务器
  地址 = settings.server.addr;连接/断开按钮与设置页同语义 `hub.connect/disconnect`;
  latency 值随窗口重绘刷新,无独立定时器)

## 对话框(widgets/dialog)

- 入口函数模式:`open_*(window: &mut Window, cx: &mut App, …)`,从任意
  on_click 触发;`widgets/dialog.rs` 统一 `pub use` 导出
- **删除类确认一律 `open_confirm_dialog(window, cx, title, message, on_confirm)`**
  (watchlist 删组 / screener 删 preset 面板与设置页三处共用);自定义对话框的
  取消/确认按钮对用 `dialog_footer(id_prefix, on_confirm)`
- **状态实体在 open 之前创建**(dialog builder 是 `Fn` 会重跑;内层 `move`
  闭包用克隆的实体,不能把 builder 捕获的实体 move 进去)
- **异步结果必须强制重绘**(2026-11-15 实测,translate/search 已接):对话框
  内容构建器在**每次窗口重绘**时重跑,而对话框私有的裸状态实体
  (TranslateState/SearchSymbolState)**没有任何观察者** —— 响应落地后的
  `cx.notify()` 打在无人监听的实体上,界面纹丝不动;首屏结果"看得到"纯粹是
  hover/键盘交互在驱动重绘。修法:open 时捕获 `window.window_handle()`,
  异步写回后 `window_handle.update(cx, |_, window, _| window.refresh())`。
  加载态同理:fetch 开始把结果置回 None(Spinner)并同帧强制重绘,否则
  转圈也不可见
- **输入框焦点**:initial_focus `Cell<bool>` guard,builder 首跑
  `input.read(cx).focus_handle(cx).focus(window, cx)`,重跑不抢
- **Enter 确认**:Input 的 key context 绑了 enter/up/down,外壳 `on_key_down`
  对 enter 不可靠——**订阅 `InputEvent::PressEnter`**(input 必发);关对话框
  用 open 时捕获的 `window.window_handle()`(`AnyWindowHandle`,Copy):
  `window_handle.update(cx, |_, window, cx| window.close_dialog(cx))`
- **Dialog 宽度显式 `dialog.width(px(N))`**:不设时内容比它宽会静默裁切
- 翻译对话框(translate.rs):footer = from/to 语言下拉 + 刷新按钮;语言存
  `settings.translate`(rust-i18n locale 字符串,无枚举);切语言落盘但不
  自动重发,刷新才重发
- 搜索对话框(search_symbol.rs):Input Group(前置搜索图标 + 后置刷新按钮
  `.loading(searching)`);300ms 防抖 + generation 代数防乱序;**组内按钮
  on_click 要回焦输入框**(`input.read(cx).focus_handle(cx).focus(window, cx)`)
  否则抢焦点断键盘

## 常用配方

**新增面板**:复制 `panels/quote.rs` 脚手架(或 watchlist.rs 的完整版),
改 `NAME/ICON/TITLE_KEY`(TITLE_KEY 对应的 locale 两份都加);需要持久化 →
`dump` 写 `PanelInfo::Panel(json!())`、`new` 读 info;需要数据表 → delegate
放 `widgets/datatable/`(日历/新闻/筛选器族的进对应族子目录并在族根声明,
独立面板平铺)、实现 `Loadable`、`type DataEntity = TableState<D>`、
`refresh` 里调 `load_rows`;需要隐藏退订 → 覆写 `set_active`;最后在
`panels::all()` 加一行——注册与打开动作自动跟上;菜单在
`widgets/menu/panels.rs` 补一行(族内条目用 `app.menu.item.*` 短标签)。

**新增对话框**:跨窗口共用的放 `widgets/dialog/x.rs`,写 `open_x_dialog(window, cx, …)`,
`widgets/dialog.rs` `pub use` 导出;**只服务一个窗口的放该窗口目录下**(如
`windows/etf_linkcalc/params_dialog.rs`,`pub(super)`)——widgets 不反向依赖
windows。两种都是:状态实体先行 + initial_focus guard;脚注用 `dialog_footer`;
回调需要改输入框(`set_value`/`set_placeholder`)时签名带 `&mut Window`;
**异步结果写裸状态实体的,完成时必须经 `window_handle` 强制重绘**(见对话框节
的陷阱)。

**新增窗口**:windows/ 下实现 `AppWindow`(new 里调 `hook_should_close`;
内容是面板布局再实现 `DockWindow`,center-only 覆写 `center_only`)→
`windows.rs` 加 `WindowType` 变体,并补齐 same_kind / instance_id / sort_key /
title / dock_name / default_bounds / window_options / init 的 match 分支
(编译器会逐个报缺)。单例=`default_window_type` 返回固定变体;
多开=覆写 `open` 用 `allocate_instance_id` 后 `open_instance`。
**必须走 `open_instance`**(Root 包装在里面)。打开入口 = `actions.rs` 加
`OpenX` 动作 + `widgets/menu/windows.rs` 加菜单项与 `on_action`。

**测试入口**:自动化测试在 `cargo test -p client --lib`(settings 往返、i18n 一致
性)与 `cargo test -p server`(finviz 行换算/周窗口);对话框/搜索等交互目前
手动验证,正式入口以后做在标题栏菜单。

**i18n**:key 加进 `locales/en-us.yml` 和 `zh-cn.yml` 各一份,两份文件分节
结构完全同构(全局/菜单/窗口/panels 标题/面板文案/calendar 族/设置页/通用组件/
消息通知/log),新 key 放对应功能节;代码 `t!("app.xxx")`;带参数
`t!("key", name = value)`,文案里占位符是 **`%{name}`**
(写成 `{name}` 不会报错但永远不替换);tracing 日志文本走 `app.log.*`
(上游错误值作 `%{err}` 参数保持原文);删功能时同步清 key,别留孤儿。
翻译语言枚举用
语言值即 rust-i18n locale 字符串(`en-us`/`zh-cn`)。
**locale 名 = 文件名 stem,rust-i18n 精确匹配不折叠大小写**:`set_locale` 传了
不存在的名字(如 `en-US` vs 文件 `en-us.yml`)不报错,**全部 `t!` 返回裸 key**。
全项目统一**小写**(`en-us`/`zh-cn`),语言列表唯一来源是
`settings::LANGUAGES`(默认值、设置页下拉框都从它取),
`settings.rs` 的 `languages_match_locale_files` 测试守住文件名与常量一致。
另外 rust-i18n 4.2.4 的宏不向 cargo 登记 YAML 文件:**只改 YAML 不改 .rs 不会
重编译**,新 key 不生效先 `touch` 任一 .rs 或 `cargo clean -p client`。

**图标**:SVG 放 `assets/icons/`,统一 **lucide 线稿**:viewBox 24、
`fill="none"`、`stroke="currentColor"`、`stroke-width="2"`、圆角
(`stroke-linecap/linejoin="round"`);斜线图标在标题栏小尺寸光栅化显细,
允许光学补偿(`window-close` 是 2.3,文件头有注释);品牌 logo
(`zen-sidus.svg/.ico`)多色,除外。`utils/icons.rs`
的 `AppIcon` 登记变体;gpui-kit 内置图标先查 `default-icons.txt`(106 个)。

## 验证清单

1. `cargo check -p client --all-targets && cargo clippy -p client --all-targets &&
   cargo test -p client --lib`(当前 **clippy 零警告**,不要新增)
2. **UI 可见测试由用户手动做,自动化只做启动 + PrintWindow 截图验证渲染;
   交互链路给编号步骤清单**(合成点击已证明不可靠:窗口枚举只读首字符、
   副屏负坐标点击落空)。渲染截图载体:写**临时** `crates/client/examples/x.rs`
   (`bootstrap` + `init_app` 后直接 `cx.open_window` 构造窗口并包 Root,**不走
   `open()`**,这样不进 registry、不写用户 settings.json),按 PID 枚举窗口
   PrintWindow,截完**删掉**——仓库不保留 examples/
3. 手测点:开/关/重启后窗口位置与布局还原(主窗口走 X → 保存);单例重复
   打开 = 聚焦;Custom/ETFLinkCalc 多开与 id 复用;杀进程后布局仍在;面板
   隐藏/重显的订阅退订/补订
4. 排查 `~/.zensidus/logs/<日期>.log`;`window not found` 三连 = 陷阱 1;
   配置回退默认 = 陷阱 4
5. 改窗口/面板 serde 格式 → 检查 `~/.zensidus/settings.json` 兼容性
   (新增字段必须 `#[serde(default)]`)
