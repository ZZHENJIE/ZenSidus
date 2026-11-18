# 添加新面板（Panel）

面板是 Dock 里的一个 tab。本页从最小占位面板起步，逐步加上远程数据表格、状态持久化、
设置联动，每一步都对应仓库里一个现成的面板作为参照。

> 第 1 节的 `SamplePanel` 已在当前工程里实际编译验证过（零警告）。

## 0. 先理解：一个面板由什么组成

一个面板类型要同时实现三组 trait，各管一摊：

| Trait | 来自 | 管什么 |
|---|---|---|
| `crate::panels::Panel` | 本项目 `panels.rs` | **身份与构造**：`NAME`（持久化主键）、`ICON`、`TITLE_KEY`、`new(window, cx, info)`、`refresh` 静态钩子、`default_title()`、`spec()` |
| `gpui_kit::base::dock::Panel` | gpui-kit | **dock 的持久化与生命周期**：`panel_name()`、`dump()`、`set_active()`、`on_removed()` |
| `gpui_kit::component::dock::Panel` | gpui-kit | **外观**：`title()`、`toolbar_buttons()`、`dropdown_menu()`… |

再加三个常规 trait：`Render`（画内容）、`gpui_kit::Focusable`（焦点句柄）、
`EventEmitter<PanelEvent>`（空实现，dock 需要）。

所有面板登记在 `panels::all()` 这张总表里。**注册进 `PanelRegistry` 和打开面板的动作
由总表驱动**；"Panels" 菜单在 `widgets/menu/panels.rs` 里按族分组为子菜单（族内条目
用短标签、省族词）——新增面板需要实现它，在总表加一行，再在菜单里补一行。

文件位置约定（`panels/` 与 `widgets/datatable/` 同构：日历/新闻/筛选器族各一个
模块根 + 族子目录，族根只做声明与 re-export）：

- 面板本体：族内 `crates/client/src/panels/calendar/<name>.rs` 等，独立面板
  `crates/client/src/panels/<name>.rs`（UI 壳 + 订阅/生命周期）；族根与总表各登记一行
- 面板数据（表格 delegate 等）：族内 `crates/client/src/widgets/datatable/<族>/<name>.rs`，
  独立面板 `crates/client/src/widgets/datatable/<name>.rs`

## 1. 最小面板（占位）

### 1.1 新建 `crates/client/src/panels/sample.rs`

```rust
//! Sample panel — the minimal five-piece scaffold from the dev docs.

use gpui_kit::{
    App, Context, EventEmitter, FocusHandle, IntoElement, ParentElement as _, Render, Window,
    base::dock::PanelEvent, div,
};

use crate::panels::Panel;
use crate::utils::AppIcon;

pub struct SamplePanel {
    focus_handle: FocusHandle,
}

impl Panel for SamplePanel {
    type DataEntity = ();
    const NAME: &'static str = "SamplePanel";
    const ICON: AppIcon = AppIcon::Experiment;
    const TITLE_KEY: &'static str = "app.panels.sample";

    fn new(_window: &mut Window, cx: &mut Context<Self>, _info: &serde_json::Value) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
        }
    }
}

impl gpui_kit::base::dock::Panel for SamplePanel {
    fn panel_name(&self) -> &'static str {
        Self::NAME
    }
}

impl EventEmitter<PanelEvent> for SamplePanel {}

impl gpui_kit::Focusable for SamplePanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl gpui_kit::component::dock::Panel for SamplePanel {
    fn title(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        Self::default_title()
    }
}

impl Render for SamplePanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().child("Hello from SamplePanel")
    }
}
```

逐段说明：

- **`type DataEntity = ()`**：面板没有数据实体就写 `()`。有表格时是 `TableState<你的Delegate>`（第 2 节）。
- **`NAME`** 是持久化布局里的主键，也是 `PanelRegistry` 的注册名。**发布后不要再改**：
  改了，用户已保存布局里的这个面板会变成"未知面板"占位。`panel_name()` 必须返回
  `Self::NAME`——两边是同一个常量，才能保证注册名和持久化名一致。
- **`ICON` / `TITLE_KEY`** 同时用于 tab 标题和 "Panels" 菜单项，只写一处。
- **`new(…, info)`**：`info` 是上次 `dump()` 持久化的 JSON（新建实例时是 `Null`）。占位面板不读它。
- **`title()`** 直接 `Self::default_title()`：图标 + 翻译后的 `TITLE_KEY`。需要动态标题
  （比如显示选中的 symbol）见第 3 节。
- **`Render`** 里的根元素要撑满：真实面板用 `v_flex().size_full()` / `div().size_full()`，
  否则内容区会露出一片未绘制的白（见 [约定与陷阱](guides/conventions.md)）。

### 1.2 登记到总表（`crates/client/src/panels.rs`，三处）

```rust
pub mod sample;                      // 1. 模块
pub use sample::SamplePanel;         // 2. 导出

/// Every shipped panel, in "Panels" menu order.
pub fn all() -> [PanelSpec; 10] {    // 3. 数组长度 +1，按菜单顺序插入
    [
        WatchlistPanel::spec(),
        // …
        ExperimentPanel::spec(),
        SamplePanel::spec(),
    ]
}
```

做完这一步，以下三件事自动成立，**不用碰 `actions.rs` 和 `widgets/menu/`**：

- `panels::init` 把 `SamplePanel` 注册进 `PanelRegistry`（布局恢复用）
- 主窗口和自定义窗口的 "Panels" 菜单多出一项
- 点菜单项 → `OpenPanel { name: "SamplePanel" }` → 在当前窗口中央 tab 组里打开

### 1.3 文案与图标

两份 locale 都要加 `TITLE_KEY` 对应的条目：

```yaml
# crates/client/locales/en-us.yml
app:
  panels:
    sample: Sample

# crates/client/locales/zh-cn.yml
app:
  panels:
    sample: 示例
```

图标可以先复用现有的 `AppIcon` 变体。要新图标：把 SVG（不要写死颜色，用
`currentColor`）放到 `crates/client/assets/icons/`，在 `utils/icons.rs` 的 `AppIcon` 加变体并在
`path()` 里加一行。不要直接用 gpui-kit 的 `IconName::X` 当面板图标——内置集只带了约 106 个
SVG，枚举里有而文件没有的会渲染成空白。

### 1.4 跑起来

```bash
cargo clippy -p app --all-targets   # 零警告
cargo run -p client
```

手测：菜单 **Panels → 示例** 能打开；把它拖到任意位置；重启后布局还原（靠 `NAME` 注册）。

## 2. 带远程数据的表格面板

参照 `panels/calendar/ipo.rs` + `widgets/datatable/ipo_calendar.rs`（最简单的远程表格，无凭据、无参数）。
分两层：**delegate** 持有数据并画单元格，**面板**只做 UI 壳和拉数据。

### 2.1 Delegate：`crates/client/src/widgets/datatable/<族>/<name>.rs`（独立面板平铺在 datatable/ 下，族根登记）

```rust
use gpui_kit::{
    App, Context, IntoElement, Window,
    component::table::{Column, TableDelegate, TableState},
};
use rust_i18n::t;

use crate::widgets::datatable::{self, Loadable};

#[derive(Clone)]
pub struct IPODelegate {
    pub data: Vec<gateway_sdk::ipo::Item>,
    pub is_loading: bool,
    columns: Vec<Column>,
}

impl IPODelegate {
    pub fn new(data: Vec<gateway_sdk::ipo::Item>) -> Self {
        Self {
            data,
            is_loading: false,
            columns: vec![
                Column::new("company", t!("app.calendar.ipo.company")).width(90.),
                Column::new("symbol", t!("app.calendar.ipo.symbol")).width(90.).resizable(false),
                // …
            ],
        }
    }
}

/// 让 `datatable::load_rows` 能统一驱动"加载中"标志和行存储。
impl Loadable for IPODelegate {
    type Row = gateway_sdk::ipo::Item;

    fn loading_mut(&mut self) -> &mut bool {
        &mut self.is_loading
    }

    fn rows_mut(&mut self) -> &mut Vec<Self::Row> {
        &mut self.data
    }
}

impl TableDelegate for IPODelegate {
    fn columns_count(&self, _: &App) -> usize { self.columns.len() }
    fn rows_count(&self, _: &App) -> usize { self.data.len() }
    fn column(&self, col_ix: usize, _: &App) -> Column { self.columns[col_ix].clone() }
    fn loading(&self, _cx: &App) -> bool { self.is_loading }

    fn render_loading(
        &mut self,
        _size: gpui_kit::component::Size,
        _window: &mut Window,
        _cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        datatable::loading_view()          // 统一的居中 spinner
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let row = &self.data[row_ix];
        match self.columns[col_ix].key.as_ref() {
            "company" => row.company.to_string(),
            "symbol" => row.symbol.clone(),
            // …
            _ => String::new(),
        }
    }
}
```

要点：

- `Column::new(key, title)` 的 `key` 和 `render_td` 里的 `match` 分支一一对应。
- 空表提示实现 `render_empty`，用 `datatable::empty_view(tr!("app.<name>.empty"), cx)`。
- 列标题这类"构造时就定死"的文案可以直接传 `t!(…)`；要 `SharedString` 的地方用 `tr!`。
- 在 `widgets/datatable.rs` 里 `pub mod <name>; pub use <name>::XDelegate;`。

### 2.2 面板：`crates/client/src/panels/<name>.rs`

```rust
use gpui_kit::{
    App, Context, Entity, EventEmitter, FocusHandle, IntoElement, ParentElement as _, Render,
    Styled, WeakEntity, Window,
    base::dock::PanelEvent,
    component::{button::Button, table::TableState},
    div,
};

use crate::panels::{Panel, refresh_button};
use crate::tr;
use crate::utils::AppIcon;
use crate::widgets::datatable::{self, IPODelegate};

pub struct IPOCalendarPanel {
    focus_handle: FocusHandle,
    table: Entity<TableState<IPODelegate>>,
}

impl Panel for IPOCalendarPanel {
    type DataEntity = TableState<IPODelegate>;
    const NAME: &'static str = "IPOCalendarPanel";
    const ICON: AppIcon = AppIcon::Calendar;
    const TITLE_KEY: &'static str = "app.panels.ipo_calendar";

    fn new(window: &mut Window, cx: &mut Context<Self>, _info: &serde_json::Value) -> Self {
        let this = Self {
            focus_handle: cx.focus_handle(),
            table: datatable::new_table_state(cx, window, |_| IPODelegate::new(vec![])),
        };
        Self::refresh(this.table.downgrade(), cx);     // 启动即加载
        this
    }

    /// Fetch the calendar; a failure lands in the message center.
    fn refresh(entity: WeakEntity<Self::DataEntity>, cx: &mut App) {
        let client = cx.global::<AppState>().gateway_client.clone();
        datatable::load_rows(
            entity,
            tr!("app.calendar.ipo.failed"),
            |_http| async move { client.ipo().await },
            cx,
        );
    }
}

// base::dock::Panel / EventEmitter / Focusable 同第 1 节 …

impl gpui_kit::component::dock::Panel for IPOCalendarPanel {
    fn title(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        Self::default_title()
    }

    fn toolbar_buttons(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> Option<Vec<Button>> {
        Some(vec![refresh_button::<Self>("ipo-refresh", self.table.downgrade())])
    }
}

impl Render for IPOCalendarPanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(datatable::table_render(&self.table))
    }
}
```

关键点：

- **`refresh` 是静态钩子**：只拿得到 `WeakEntity<DataEntity>` 和 `&mut App`，语义是
  "把数据从源重载进 DataEntity"。`new` 里调一次 = 启动自动加载；工具栏刷新按钮
  `refresh_button::<Self>(id, weak)` 调的也是它。需要额外上下文（symbol、preset 名）
  就放进 delegate，从 `entity.read(cx).delegate()` 读。
- **`datatable::load_rows(table, 失败标题, |http| async {…}, cx)`** 负责整个流程：
  置 `is_loading = true` → 用应用的 HTTP client 跑你的 future → 成功写回行、失败清空并
  `message::error(失败标题, err)` → `is_loading = false` → `notify`。future 返回
  `anyhow::Result<Vec<Row>>`。
- 失败标题加一个 locale key（`app.<name>.failed`），两份都写。

### 2.3 需要凭据或 API Key 时：**守卫放在 `load_rows` 之前**

参照 `panels/news.rs` 的 `alpaca_credentials`（两个新闻面板共享，缺凭据时推"去设置页"的消息）和 `panels/screener/stock.rs` 的 `finviz_api_key`：

```rust
fn fetch(table: WeakEntity<TableState<NewsDelegate>>, cx: &mut App) {
    let Some(symbol) = table.upgrade().and_then(|t| t.read(cx).delegate().symbol.clone()) else {
        return;
    };
    // 缺凭据:推一条"没配置,去设置页"的消息,然后直接 return。
    // 不进 load_rows,所以不会发请求,也不会把表格置成加载中。
    let Some((key_id, secret)) = alpaca_credentials(cx) else {
        return;
    };
    datatable::load_rows(table, tr!("app.stock_news.failed"), move |http| async move { … }, cx);
}

fn alpaca_credentials(cx: &mut App) -> Option<(String, String)> {
    let found = cx.global::<AppState>().settings.alpaca.credentials()
        .map(|(k, s)| (k.to_string(), s.to_string()));
    if found.is_none() {
        message::error(tr!("app.stock_news.no_credentials"), "alpaca api key/secret is not configured", cx);
    }
    found
}
```

`message::error(message, detail, cx)` 里 **`message` 才是用户看见的**，`detail` 进日志；
两个参数都走 i18n（`detail` 用 `app.log.*` 键）。把"请求失败"写成 message、真实原因
塞进 detail，用户就会以为请求发出去了。

## 3. 持久化面板自己的状态

参照 `panels/news/stock.rs`：每个实例记住自己跟踪的 symbol。

```rust
impl gpui_kit::base::dock::Panel for StockNewsPanel {
    fn panel_name(&self) -> &'static str {
        Self::NAME
    }

    /// Persist the followed symbol per instance; `new` reads it back.
    fn dump(&self, cx: &App) -> PanelState {
        let mut state = PanelState::new(Self::NAME);
        state.info = PanelInfo::Panel(serde_json::json!({ "symbol": self.symbol(cx) }));
        state
    }
}

impl Panel for StockNewsPanel {
    fn new(window: &mut Window, cx: &mut Context<Self>, info: &serde_json::Value) -> Self {
        let persisted_symbol = info
            .get("symbol")
            .and_then(|value| value.as_str())
            .map(String::from);
        // …用 persisted_symbol 构造 delegate；有值就 Self::fetch 自动加载
    }
}
```

- `dump` 在每次布局变化时被调用并随窗口 cache 一起落到 `settings.json`，**内容尽量小**。
- 读的时候一律用 `get(..).and_then(..)` 这种宽容写法：字段缺了、类型变了就当没有，
  不要 `unwrap`。这样改 payload 格式不会让旧布局恢复失败。
- 动态标题用 `panel_title(Self::ICON, label)`：

```rust
fn title(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let label = self.symbol(cx).map(SharedString::from).unwrap_or_else(|| tr!(Self::TITLE_KEY));
    panel_title(Self::ICON, label)
}
```

## 4. 响应设置变化、共享实时流

参照 `panels/watchlist.rs`（完整版）和 `panels/screener/stock.rs`。

### 4.1 订阅设置变化

任何设置改动都经 `settings::update_settings` → `global_mut::<AppState>` → 所有
`observe_global::<AppState>` 观察者被通知：

```rust
cx.observe_global_in::<AppState>(window, |this, window, cx| {
    // 1. 先让 delegate 自己同步(例如:选中的分组被别处删了 → 回退到第一个)
    Self::refresh(this.table.downgrade(), cx);
    // 2. 再按 delegate 的结果设 picker —— 顺序反了 picker 会停在空白
    let current = this.group(cx);
    this.picker.set_items(group_names(cx), window, cx);
    this.picker.set_selected(current, window, cx);
    cx.notify();
})
.detach();
```

"设置变了，我的选择还在吗"这类回退逻辑放在 delegate 里
（`WatchlistDelegate::reload_from_settings` / `StockScreenerDelegate::sync_with_settings`，后者返回
是否变化，变了面板才重新拉数据）。

### 4.2 共享 Alpaca 实时流

- 面板用 `hub.set_symbols(自己的 EntityId, BTreeSet<String>)` 表达想要的 symbol；hub 只把
  **并集**的增减发到服务端，两个面板同看 NVDA 只订一次。
- 面板各自 `hub.read(cx).stream().map(|s| s.messages())` 拿一个 broadcast 接收端，在
  `cx.spawn` 里循环 `recv` 按自己的 symbol 过滤；把 `Task` 存进字段，丢弃即取消。
- tab 被切走时退订、切回来补订：覆写 `set_active(active)`，里面重新 `set_symbols`。
- 清理双保险：`on_removed` 和 `cx.on_release` 都把自己的集合设为空（两个都要幂等）。

## 5. 检查清单

- [ ] `NAME` 唯一、以后不改；`panel_name()` 返回 `Self::NAME`
- [ ] `panels::all()` 加了一行，数组长度同步；`widgets/menu/panels.rs` 菜单补了一行
- [ ] `app.panels.<name>` 两份 locale 都有；失败/空态文案也都有
- [ ] 图标存在（`AppIcon` 变体 + SVG 文件，或确认 gpui-kit 内置集真的有）
- [ ] 根元素 `size_full()`
- [ ] 所有 `cx.spawn(...)` 都 `.detach()` 了，或把 `Task` 存进字段
- [ ] 需要凭据的请求，守卫在 `load_rows` 之前，缺失时消息文案说清"去哪配置"
- [ ] `cargo clippy -p app --all-targets` 零警告；`cargo run -p client` 从菜单打开、重启后还在

## 6. 常见坑

| 现象 | 原因 | 处理 |
|---|---|---|
| 面板打开后下半截是白的 | 根元素没 `size_full()`，或中间夹了裸 `div()` | 根和中间容器都加 `size_full()`；`size_full` 是 `Styled` trait 方法，不在作用域会报 E0599 |
| 请求发出去了但 UI 没反应 | `cx.spawn(...)` 的返回值没 `.detach()`，语句结束任务就被取消 | `.detach()`，且不要写成 `let _ = …` |
| 重启后面板变成空占位 | 改了 `NAME`，或注册名和 `panel_name()` 不一致 | 不改 `NAME`；`panel_name()` 用 `Self::NAME` |
| 新加的文案显示成 key | 只改了 YAML 没改任何 `.rs`，rust-i18n 的宏不会重新读文件 | `touch` 任一 `.rs` 或 `cargo clean -p app`；或者两份 locale 有一份漏了 |
| 字段叫 `gen` 编译不过 | edition 2024 保留字 | 换名 |
| 用户看到"请求失败"但其实是没配 Key | `message::error` 的 message 写成了失败标题 | 守卫里推专门的 `no_credentials` 文案（见 2.3） |
