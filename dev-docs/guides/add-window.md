# 添加新窗口（含多实例）

窗口比面板多一层"身份"问题：单例（全局只有一个）还是多实例（可以开好几个、每个记住
自己的位置和布局）。本页先讲窗口模型，然后给出单例窗口的完整示例（已在当前工程编译
验证），最后以现有的 `CustomWindow` 为模板讲多实例和面板布局窗口。

## 0. 窗口模型：三个概念

**`WindowType`**（`windows.rs`）是窗口的身份，一个枚举：

- `Main` / `Settings` / `SmallClock`：单例。开过了再开 = 把已有的聚焦到前台。
- `Custom(u32)` / `ETFLinkCalc(u32)`：多实例，实例号在变体里。新实例拿"当前没开的最小 id"，
  所以关掉 Custom #1 再开一个，会**恢复 #1 记住的位置和布局**（联价器连参数一起恢复）。

**`WindowState`** 是每个窗口的持久化记录（几何 / 最大化 / 显示器 / 是否开机显示 /
`cache` 载荷），存在 `settings.json` 的 `window_state` 数组里。dock 布局也放在 `cache`
里，没有单独的布局文件。

**`AppWindow`**（`windows/app_window.rs`）是每种窗口要实现的契约。它把窗口管理全部
做成了默认方法，最关键的是 `open_instance`：

- 已开 → `cx.defer` 激活（不能同步操作正在更新的窗口，会假报 "window not found"）
- 未开 → 从 `WindowState` 恢复几何和 cache → `cx.open_window` → **窗口根包成 gpui-kit 的
  `Root`** → 登记进 `WindowRegistry` → 写 `WindowState`

> **铁律：永远走 `open_instance`，不要自己 `cx.open_window`。**
> `Root` 包装在里面；没有它，第一次弹对话框（`WindowExt::open_dialog` 找不到 overlay）
> 进程直接退出，而且没有 panic 窗口，只有 stderr 里一行日志。

内容是面板布局的窗口再实现 `DockWindow`（见第 3 节），其余窗口到 `AppWindow` 为止。

## 1. 单例窗口

以"加一个 Sample 窗口"为例。所有按窗口类型分派的 `match` 都集中在
`crates/client/src/windows.rs` 一个文件里，**加了枚举变体后跟着编译器错误走完这个
文件即可**，一共 7 个触点：

### 1.1 `windows.rs` 里的 7 个触点

```rust
// 1. 模块与导出
pub mod sample;
pub use sample::SampleWindow;

pub enum WindowType {
    Main,
    Settings,
    SmallClock,
    Sample,        // 2. 新变体(多实例是 X(u32),见第 2 节)
}

impl WindowType {
    pub fn same_kind(self, other: Self) -> bool {
        matches!(
            (self, other),
            (Self::Main, Self::Main)
                | (Self::Settings, Self::Settings)
                | (Self::SmallClock, Self::SmallClock)
                | (Self::Sample, Self::Sample)     // 3. 家族匹配
                | (Self::Custom(_), Self::Custom(_))
        )
    }

    fn sort_key(self) -> u8 {                      // 4. 启动恢复顺序(主窗口最先)
        match self {
            Self::Main => 0,
            Self::Settings => 1,
            Self::SmallClock => 2,
            Self::Sample => 2,
            Self::Custom(_) => 3,
        }
    }

    pub fn title(self) -> SharedString {           // 5. 窗口标题
        match self {
            // …
            Self::Sample => tr!("app.windows.sample.title"),
            // …
        }
    }

    pub fn dock_name(self) -> SharedString { … }   // 6. dock 区域名(不用 dock 可占位)

    // window_options 里的 kind / window_min_size 两个 match(7):
    // 普通窗口不需要改(落进 `_ =>` 分支);
    // 要 Dialog / PopUp 行为或特殊最小尺寸时才加分支。
}

fn default_bounds(cx: &App, window_type: WindowType) -> Bounds<Pixels> {
    let (width, height) = match window_type {
        // …
        WindowType::Sample => (480., 320.),        // 没记住几何时的默认尺寸
        // …
    };
    app_window::centered_bounds_size(cx, width, height)
}

pub fn init(cx: &mut App, states: &[WindowState]) {
    // …
    for window_type in types {
        match window_type {
            // …
            WindowType::Sample => SampleWindow::open_instance(window_type, cx),  // 启动恢复
            // …
        }
    }
}
```

### 1.2 新建 `crates/client/src/windows/sample.rs`

```rust
//! Sample window — the minimal singleton `AppWindow` from the dev docs.

use gpui_kit::{Context, IntoElement, ParentElement as _, Render, Styled as _, Window, base::v_flex};

use crate::windows::{AppWindow, WindowType, hook_should_close};

pub struct SampleWindow;

impl AppWindow for SampleWindow {
    fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        _window_type: WindowType,
        _cache: serde_json::Value,
    ) -> Self {
        // 把平台"点关闭按钮"接到 AppWindow::close:持久化几何/cache、
        // 从注册表注销、settings 里 show=false。普通窗口都要调这一句。
        hook_should_close(window, cx);
        Self
    }

    fn window_type(&self) -> WindowType {
        WindowType::Sample
    }

    fn default_window_type() -> WindowType {
        WindowType::Sample
    }

    /// 标题栏左侧:简单窗口放标题文字(带菜单栏的窗口放菜单行)。
    fn render_titlebar_left(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> impl IntoElement {
        super::titlebar_title(WindowType::Sample.title())
    }

    fn render_content(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        v_flex().flex_1().p_4().child("Hello from SampleWindow")
    }
}

impl Render for SampleWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_window(window, cx)      // 统一的自绘标题栏壳 + 内容
    }
}
```

说明：

- `new` 的 `cache` 参数是上次关闭时 `dump_cache` 存的内容（默认 `Null`），恢复状态用。
- `default_window_type` 只有单例有意义——`open()` 的默认实现拿它当要开的身份。
- 关闭语义默认就有：点 ✕ → `AppWindow::close` → 几何/cache 落盘、`show=false`，
  下次启动不再自动出现（手动从菜单打开时恢复记忆的位置）。
  主窗口是唯一例外（见第 4 节）。

### 1.3 入口：动作 + 菜单

`crates/client/src/utils/actions.rs`：

```rust
actions!(zen_sidus, [OpenSmallClock, OpenCustom, OpenSample]);
```

`crates/client/src/widgets/menu/windows.rs`（菜单项 + 动作安装器各一处）：

```rust
use crate::windows::{AppWindow, CustomWindow, SampleWindow, SmallClockWindow};

pub fn menu() -> impl gpui_kit::IntoElement {
    Button::new("windows-menu")
        .ghost()
        .small()
        .label(t!("app.menu.windows"))
        .dropdown_menu(|menu, _, _| {
            // …
            menu.menu_with_icon(
                t!("app.menu.sample"),
                AppIcon::Experiment,             // 换成你的 AppIcon
                Box::new(actions::OpenSample),
            )
        })
}

pub fn actions<T: 'static, E: IntoElement + InteractiveElement>(
    div: E,
    cx: &mut gpui_kit::Context<T>,
) -> E {
    div.on_action(cx.listener(|_, &actions::OpenSmallClock, _, cx| SmallClockWindow::open(cx)))
        .on_action(cx.listener(|_, &actions::OpenCustom, _, cx| CustomWindow::open(cx)))
        .on_action(cx.listener(|_, &actions::OpenSample, _, cx| SampleWindow::open(cx)))
}
```

### 1.4 文案

```yaml
# locales/en-us.yml
app:
  menu:
    sample: Sample Window
  windows:
    sample:
      title: Sample

# locales/zh-cn.yml
app:
  menu:
    sample: 示例窗口
  windows:
    sample:
      title: 示例
```

### 1.5 验证

```bash
cargo clippy -p app --all-targets    # 零警告(漏掉的 match 编译器会逐个指出来)
cargo run -p client
```

手测：菜单 **Windows → 示例窗口** 打开；再点一次 = 聚焦而不是开第二个；移动/缩放后
关掉再开，位置尺寸恢复；关掉后重启应用不自动出现。

## 2. 多实例窗口

参照 `windows/custom.rs`。差异只在"身份怎么来"：单例的身份是写死的变体，
多实例的身份 = 变体 + 实例号，实例号在 `open()` 时分配。

### 2.1 `WindowType` 侧

```rust
pub enum WindowType {
    // …
    Custom(u32),        // 多实例:变体带 u32
}

impl WindowType {
    pub fn same_kind(self, other: Self) -> bool {
        matches!((self, other), … | (Self::Custom(_), Self::Custom(_)))
        // 家族匹配:Custom(1) 和 Custom(2) 是同一"类"
    }

    pub fn instance_id(self) -> Option<u32> {
        match self {
            Self::Custom(id) => Some(id),
            _ => None,
        }
    }

    pub fn title(self) -> SharedString {
        match self {
            // 标题带实例号
            Self::Custom(id) => format!("{} #{id}", t!("app.windows.custom.title")).into(),
            // …
        }
    }

    pub fn dock_name(self) -> SharedString {
        match self {
            Self::Custom(id) => format!("custom-{id}-dock").into(),
            // …
        }
    }
}

fn default_bounds(…) {
    match window_type {
        WindowType::Settings | WindowType::Custom(_) => (960., 640.),   // 可以和别的分支共享
        // …
    }
}

pub fn init(…) {
    // 注意分支模式是 `Custom(_)`:
    // 启动恢复时,每个已保存且 show=true 的实例各走一次
    WindowType::Custom(_) => CustomWindow::open_instance(window_type, cx),
}
```

### 2.2 窗口类型侧（`windows/custom.rs` 的骨架）

```rust
pub struct CustomWindow {
    id: u32,
    dock_area: Entity<DockArea>,          // 面板布局窗口才需要(见第 3 节)
    _dock_subscription: Subscription,
}

impl AppWindow for CustomWindow {
    fn new(window: &mut Window, cx: &mut Context<Self>, window_type: WindowType, cache: serde_json::Value) -> Self {
        let (dock_area, _dock_subscription) = DockWindow::new_dock(window_type, &cache, window, cx);
        hook_should_close(window, cx);
        Self {
            id: window_type.instance_id().unwrap_or(0),   // 从身份里拿实例号
            dock_area,
            _dock_subscription,
        }
    }

    /// 多实例窗口必须覆写 open:分配"当前没开的最小 id"再打开。
    /// `CustomWindow::open()` 默认实现用的是 default_window_type() = Custom(0),
    /// 不覆写的话所有实例会互相顶掉。
    fn open(cx: &mut App) {
        let id = super::allocate_instance_id(cx, WindowType::Custom(0));
        Self::open_instance(WindowType::Custom(id), cx);
    }

    fn window_type(&self) -> WindowType {
        WindowType::Custom(self.id)       // 用自己的 id,不是写死的变体
    }

    fn default_window_type() -> WindowType {
        WindowType::Custom(0)             // 仅占位,正常路径不会用到
    }

    // render_titlebar_left / render_content 同单例;标题记得带 #id
}

impl Render for CustomWindow { /* 同单例 */ }
```

两个匹配语义**不能混用**（历史上各自踩出过 bug）：

- `find_open_window`：**精确 key**（`Custom(1)` 只匹配 `Custom(1)`）。判断"这个具体窗口
  是否已开"。写成家族匹配 → 开 #2 会被当成"#1 已开"而只去聚焦 #1。
- `allocate_instance_id`：**家族匹配**（任意 id 的 `Custom(_)`）。枚举该类已开实例、
  分配最小空闲 id。写成精确匹配 → 已开列表永远为空 → 永远分配 1 → **不会**恢复旧实例
  的位置和布局。

### 2.3 持久化行为（不用写代码，理解即可）

- 每个实例一条 `WindowState`（按 `WindowType::Custom(id)` 区分），几何和 `cache` 各自独立。
- `allocate_instance_id` 故意复用最小空闲 id：关掉 #1、#2 还开着，再开一个新窗口拿到
  #1 → **#1 记住的位置和布局回来了**。这是产品语义，不是巧合。
- 关闭即落盘（`hook_should_close` → `AppWindow::close`），退出走 `persist_open_windows`
  批量回写，所以杀进程最多丢最后一帧的几何。

## 3. 内容是面板布局的窗口（`DockWindow`）

第 2 节的代码其实已经带了：`dock_area` 字段、`new` 里的 `DockWindow::new_dock(...)`、
`impl DockWindow`。完整清单：

```rust
impl DockWindow for CustomWindow {
    fn dock_area(&self) -> &Entity<DockArea> {
        &self.dock_area
    }

    /// 出厂默认布局(没有可用 cache 时)。用 panels::fresh::<P> 建面板实例。
    fn build_default_layout(area: &mut DockArea, window: &mut Window, cx: &mut Context<DockArea>) {
        let chart = panels::fresh::<panels::ChartPanel>(window, cx);
        area.set_center(
            DockLayout::tabs().panel_view(panel_handle(chart), cx),
            window,
            cx,
        );
    }

    /// 只有中央 dock(不能有左右/底部 dock 的窗口置 true,加载和保存都会剥掉其余区域)
    fn center_only() -> bool {
        true
    }

    /// 持久化布局的版本号。**改了默认布局/面板结构必须 bump**,
    /// 否则用户存量 cache 永远盖住新默认。全局 DOCK_VERSION,或窗口覆写。
    fn dock_version() -> Option<usize> {
        Some(3)
    }
}

impl AppWindow for CustomWindow {
    fn dump_cache(&mut self, cx: &mut App) -> serde_json::Value {
        dump_dock::<Self>(&self.dock_area, cx)     // 布局进 WindowState.cache
    }
    // …
}

impl Render for CustomWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .on_actions(menu::panels::actions, cx)  // 响应"Panels"菜单动作
            .child(self.render_window(window, cx))
    }
}
```

布局的日常持久化不用管：`new_dock` 内部订阅了 `DockEvent::LayoutChanged`，用户每次拖动
面板都即时落盘。

## 4. 关闭语义速查

| 窗口 | 关闭行为 | 实现 |
|---|---|---|
| 普通/多实例窗口 | ✕ = 关闭：几何+cache 落盘、`show=false`、下次不自动恢复 | `new` 里调 `hook_should_close(window, cx)` |
| 主窗口 | ✕ = 弹"保存并退出?"对话框；保存 → 所有开着的窗口批量落盘后退出 | `window.on_window_should_close(cx, close_event::on_close)`，**不**调 `hook_should_close` |
| 主窗口的 `show` | 恒为 true：它是应用锚点，关掉也恢复 | `WindowType::always_show()` + `update_window_state` 里的硬规则 |

## 5. 检查清单与常见坑

- [ ] 只通过 `open_instance` 打开窗口（`Root` 包装在里面）
- [ ] `windows.rs` 的 7 个触点都加了（跟着编译器走）；两份 locale 都有
- [ ] 多实例：覆写 `open` + `window_type()` 返回带 id 的变体 + `same_kind`/`instance_id`/`title`/`dock_name`/`init` 分支都带 id
- [ ] 面板布局窗口：改默认布局 bump `dock_version`
- [ ] 手测：单例重复打开=聚焦；多实例 id 复用恢复旧布局；关闭重启的位置还原；副屏负坐标区也正常

| 现象 | 原因 |
|---|---|
| 点了按钮进程直接退出（无 panic 窗口） | 绕过了 `open_instance`，窗口根没包 gpui-kit `Root`，第一次弹对话框就 `expect` 失败 |
| 日志里 `window not found` 三连，还误开了新窗口 | 在窗口自己的回调里同步 `handle.update` 操作该窗口；激活已开窗口必须 `cx.defer(move \|cx\| handle.update(…))`，判活用 `AnyWeakView::upgrade()` |
| 多实例开第二个把第一个顶掉 / 永远恢复不了旧布局 | `find_open_window` 与 `allocate_instance_id` 的精确/家族匹配语义写串了（见 2.2） |
| 窗口每轮关→开往上漂 3~4px | 保存几何用了 `window.bounds()`；必须用 `persisted_geometry`（基于 `window_bounds()`）——`AppWindow` 已封装好，别绕 |
| 新窗口内容区一片白 | 根元素没 `size_full()`（见 [约定与陷阱](guides/conventions.md)） |
| 每次重启设置全回默认 | `WindowState` 格式改了导致整个 settings.json 解析失败；新增字段靠结构级 `#[serde(default)]` 已兜住，改已有字段类型才是雷 |
