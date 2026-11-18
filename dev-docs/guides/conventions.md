# 约定与陷阱

两篇教程之外,写任何 GUI 代码都会用到的项目约定,以及实测踩过的坑。拿不准时先查这页,
再查 `.agents/skills/zensidus-app/SKILL.md`(同一套事实,更细的速查版)。

## 文案与 i18n(rust-i18n)

- **key 加两份**:`crates/client/locales/en-us.yml` 和 `zh-cn.yml`,结构对称。
- 代码里优先用 **`tr!`**(本项目的宏,`utils.rs` 定义):`tr!("app.x.y")` /
  `tr!("app.x.y", name = value)`,返回 `SharedString`——gpui-kit 所有文字槽位的类型。
  需要 `String` 做拼接时才用 `t!(..).to_string()`。
- 插值占位符是 **`%{name}`**。写成 `{name}` 不报错,但永远不会被替换(实测踩过)。
- **locale 名 = 文件名 stem,大小写敏感**:全项目统一小写 `en-us` / `zh-cn`。
  `set_locale` 传了不存在的名字不报错,**所有 `t!` 直接返回裸 key**(整个应用的文案
  变成 `app.xxx` 就是这个)。语言列表唯一来源是 `settings::LANGUAGES`,有测试
  `languages_match_locale_files` 守着。
- rust-i18n 4.2.4 的宏**不向 cargo 登记 YAML 文件**:只改 YAML 不改任何 `.rs` 不会
  重编译。新 key 不生效先 `touch` 一个 `.rs` 或 `cargo clean -p app`。
- 菜单类文案(标题栏下拉)在渲染时取值,改语言后随重渲染生效。

## 设置(settings)

- **读**:`cx.global::<AppState>().settings.<字段>`。
- **写**:一律 `settings::update_settings(cx, |s| …)`——它负责改内存、保存文件、失败进
  消息中心,并且 `global_mut` 会通知所有 `observe_global::<AppState>` 观察者(全窗口
  刷新零接线)。窗口几何/布局的持久化也走同一条路。
- `Settings` 及所有嵌套结构都是**结构级 `#[serde(default)]`**:旧文件缺字段照常加载,
  新增字段不用单独标注。真正的雷是**改已有字段的类型/枚举变体名**——会让整个文件
  解析失败、全部设置回默认。要改就得加 `#[serde(alias)]` 或自定义反序列化
  (参照 `Feed` 的 `alias = "Stock"`)。
- 应用运行时 `settings.json` 被锁,外部改不了;退出时会被内存数据覆盖回去。
- 凭据判空用 `settings.alpaca.credentials()`(两者都非空才 `Some`);发请求前守卫,
  缺失时推"去设置页配置"的消息(见下)。

## 消息中心(message)

```rust
message::error(message, detail, cx);  // message: 用户看见的; detail: 进日志
message::warning(..) / message::info(..) / message::success(..)
```

- **`message` 参数是用户看见的那条**,`detail` 进 `~/.zensidus/logs/`。常见反例:
  message 写"请求失败"、真实原因(没配 Key)塞 detail——用户看到"失败"以为请求
  发出去了。缺配置的情况,message 直接写"未设置 X,先在设置页填写"
  (参照 `app.stock_screener.no_api_key` / `app.stock_news.no_credentials`)。
- **两个参数都要走 i18n**:`message` 用用户可见键,`detail` 用 `app.log.*` 键
  (日志跟随 UI 语言);上游错误值 `{err:#}` 不是写死英文,作 `%{err}` 参数嵌入即可。
  纯 tracing 诊断(没有 message 推送的)同样用 `app.log.*`。
- 推送即通知,状态栏实时更新;连接 Alpaca 的成败会自动推消息。

## 图标与资源

- 自己的图标:SVG 放 `crates/client/assets/icons/`,统一 **lucide 线稿**:viewBox 24、
  `fill="none"`、`stroke="currentColor"`、`stroke-width="2"`、圆角
  (`stroke-linecap/linejoin="round"`);斜线图标在标题栏小尺寸显细,允许光学补偿
  (`window-close` 是 2.3,文件头有注释),**不写死颜色,用 `currentColor`**;
  `utils/icons.rs` 的 `AppIcon` 加变体 + `path()` 一行。
- 想用 gpui-kit 内置图标:先查 `default-icons.txt`(**只有约 106 个**)。枚举变体存在
  而 SVG 不在集里 → 渲染成空白,不报错。
- 其它资源走 `crate::utils::Assets::get("…")`(rust-embed),**禁止 cwd 相对路径**。

## 异步与实体(gpui 本 fork 的惯例)

- `cx.spawn(async move |this, cx| { … }).detach();` —— **`Task` 丢弃即取消**,不要
  `let _ = cx.spawn(...)`(那也是取消,还触发 clippy)。要么 `.detach()`,要么把
  `Task` 存进字段(可主动取消的场景,如 watchlist 的消息泵)。
- `AsyncApp::update(f) -> R`(不包 Result);`WeakEntity::update -> Result<R>`;
  `App::open_window -> anyhow::Result`;`Context::observe_global` 闭包**两个参数** `|_, cx|`。
- 全局读取没有 `Global::global(cx)`,用 `ReadGlobal` trait
  (`use gpui_kit::ReadGlobal as _;` 然后 `MessageLog::global(cx)`)。
- `gen` 是 edition 2024 保留字,不能当变量/字段名。
- `rust_i18n::locale()` 返回 `impl Deref<Target = str>`,用 `let locale = …;`
  再 `&*locale`,别链式 `.as_str()`。

## UI 排查速查

| 现象 | 原因 |
|---|---|
| 窗口/面板下半截纯白 | 根元素(或中间夹的裸 `div()`)没 `size_full()`;`size_full` 是 `Styled` 的方法,trait 不在作用域报 E0599,别误判成"没有这个方法" |
| 对话框内容被裁掉 | `Dialog` 宽度要显式 `dialog.width(px(N))`,否则内容比默认宽就静默裁切 |
| 对话框异步结果/加载态不出现 | 对话框私有的裸状态实体没有观察者,`cx.notify()` 不可见(首屏"能看见"只是 hover/键盘在驱动重绘);异步写回后须经 `window_handle` 强制 `window.refresh()`,加载态把结果置回 None 同帧重绘(translate/search 已接,详见 skill 对话框节) |
| 日志里 `window not found` 三连 + 误开新窗口 | 在窗口自己的回调里同步 `handle.update` 它自己;激活已开窗口必须 `cx.defer`,判活用 `AnyWeakView::upgrade()` |
| 配置整体回退默认 | settings.json 解析失败(改了字段类型/枚举变体名),见上 |
| 新加文案显示成 key | locale 漏了一份,或只改 YAML 没触发重编译(见上) |

## 验证

```bash
cargo check  -p app --all-targets
cargo clippy -p app --all-targets   # 当前零警告,保持
cargo test   -p client --lib           # settings 兼容 + locale 一致性
```

- UI 可见的验证由人做,自动化只做启动 + 截图(Windows 合成点击不可靠:窗口标题只读到
  首字符、副屏负坐标点击落空)。改设置页直接 `cargo run -p client` 进设置窗验证。
- 排查先看 `~/.zensidus/logs/YYYY-MM-DD.log`。
