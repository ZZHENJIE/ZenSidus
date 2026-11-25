---
name: zensidus-server
description: 'Architecture, wire protocol contract, config and deployment of the ZenSidus server crate (crates/server): TCP session lifecycle (Hello TOTP auth-code verification with ±1 window, Welcome clock sync, Ping/Pong heartbeat), SubscriptionHub (per-client wishes → upstream union with refcounting), RPC id routing + handlers, anysdk upstream integration (ReqwestExecutor, supervisor self-healing streams), the three web proxies (futunn/deepseek/iposcoop, simd-json), finviz CSV → protocol DTO mapping, server.toml config, CI single-zip packaging + install-server.sh + systemd zensidus.service + nginx TLS termination. Use when touching any file under crates/server or the protocol crate wire format, changing auth/subscription/RPC behavior, editing server.toml, or deploying/upgrading the VPS service.'
---

# ZenSidus Server (crates/server)

项目级技能:server crate 的架构、线协议契约与部署。客户端视角见
`zensidus-client` 技能;上游 SDK(Alpaca/Finviz)细节见 `anysdk` 技能。

## 项目地图

```
crates/server/src/
├── main.rs       # 入口:argv[1] / SERVER_CONFIG env / server.toml → Config::load →
│                 # tracing(env-filter)→ 上游连接(connect_with_retry 直到成功)
│                 # → 常驻任务 → TCP listener 循环
├── config.rs     # server.toml 反序列化(listen/totp_secret/log_level/alpaca/finviz/
│                 # deepseek/proxy);真实文件 gitignored,样例 server.toml.example
├── state.rs      # AppState:client_id 分配 / apply_diff(HubDiff → 上游订阅命令)/
│                 # fan_out(纯路由:Quote/Trade/Bar 按 hub 订阅表只发给订阅了
│                 # 该符号对应通道的客户端,News 按 NEWS 位只发给订阅新闻的
│                 # 客户端;server 只做数据分发,无行情计算)/ spawn(上游泵:
│                 # anysdk 消息 → mapping → fan_out;上游命令走 mpsc)
├── session.rs    # 每连接一个 task:Hello 验凭据(TOTP ±1 窗口,或会话令牌;
│                 # 不符回 AuthFail 后断开)→ Welcome 对时+轮换下发令牌+版本 →
│                 # 心跳(客户端 10s Ping,60s 无帧判死)→ Subscribe/Unsubscribe
│                 # → RPC 帧转 rpc::handle;单客户端失败不影响其他会话
├── hub.rs        # SubscriptionHub:per-client 意愿(符号键大写归一,symbol+Channels)
│                 # → 上游并集(引用计数),apply_subscribe/apply_unsubscribe/
│                 # remove_client 返回 HubDiff;subscribed(client,symbol,channels)
│                 # 是 fan_out 路由的查表判定;纯逻辑,测试锚定并集/去重/引用计数
├── rpc.rs        # RPC id 路由 + handlers:bars(带 RestProgress 进度帧)/news
│                 # 回填/日历族/screener 导出(经 finviz_data::screener_cells 定型)/
│                 # 搜索/翻译/snapshot;错误统一 rpc_error(what) 字符串
├── mapping.rs    # anysdk StockMessage/NewsMessage → protocol Frame(纯函数 +
│                 # 测试;Quote/Trade/Bar 条件位与 tape 处理)
├── executor.rs   # ReqwestExecutor(HttpExecutor<reqwest::Client> 唯一实现)+
│                 # AlpacaClient/FinvizClient 泛型别名
├── web.rs        # 三个自研网关代理(futunn 搜索/DeepSeek 翻译/iposcoop IPO 抓取),
│                 # 从 TS Gateway worker 逐行移植
└── finviz_data.rs # Finviz Elite CSV 导出 → protocol DTO:economic/earnings 周窗口
                  # (美东时钟)+ screener_cells 筛选导出逐格定型(Cell{text,value},
                  # K/M/B/%/货币/千分位→数值,原文保留显示;行换算有测试)
```

## 线协议契约(protocol crate,唯一契约)

帧格式 `[u32 LE 长度][postcard 编码的 Frame]`,一条 TCP 长连接。会话流程:
client 首发 `Hello{auth}`——首连 `HelloAuth::Totp`(认证器 6 位码,服务端
roka_totp ±1 窗口校验),重连 `HelloAuth::Session`(会话令牌,免输码)→
server 回 `Welcome`(顺带对时 + **轮换下发新会话令牌**,旧令牌即作废,并下发
server 构建版本供客户端 About 页展示)→ 客户端
10s `Ping`/server `Pong`;订阅 `Subscribe`/`Unsubscribe`(symbol 名字符串,
server 侧 intern);推送 `Quote`/`Trade`/`Bar`/`News`;RPC 请求-响应按 id
路由,`bars` 拉取过程发 `RestProgress` 进度帧。

**Alpaca bars 端点缺 `start` 时返回空**(`200 + {"bars":{}}`),不是报错
——客户端没传 start 的请求(Chart 面板默认)由 server 按 timeframe+limit
推导起点并 `sort=desc` 单页取最新 limit 根(desc 第一页即最新,截断后翻
回升序;继续翻页只会更旧,不发进度帧)。显式带 start/end 的请求维持
`bars_all_with_progress` 分页语义。

**改协议 = 改 protocol crate + client/server 两侧映射与测试**;protocol 是
全 owned DTO + postcard 派生,别在两侧私自加字段或绕过它传私有格式。

## 上游对接(细节见 anysdk 技能)

- 断线自愈由 anysdk supervisor 负责(指数退避 + 订阅回放),server 只记
  日志——**不要在 server 侧再造一层重连**。
- 下游订阅意愿变化经 `state.apply_diff` 发 `UpstreamCommand` 到上游泵,泵持
  有 alpaca_stream 客户端执行订阅/退订。
- 本机开发翻墙:`[proxy] ws_socks5`(上游 WS 走 SOCKS5);HTTP 代理由
  reqwest 读 `HTTPS_PROXY` 环境变量。

## 部署(详见 crates/server/README.md)

- CI(`.github/workflows/server.yml`):打 `server-v*` tag(或手动触发)→
  musl 静态编译 → Release 挂**单一** `zensidus-linux-amd64.zip`(内含
  `zensidus`、`server.toml.example`、`install-server.sh`、README);main push
  只做 Linux 编译检查。Artifact 上传原始文件(下载时 GitHub 打包成单层 zip)。
- 命名:二进制 `zensidus`(`[[bin]]`,包名仍是 `server`,`cargo -p server`);
  服务 `zensidus.service`;用户 `zensidus`;安装目录 `/opt/zensidus`;
  配置 `/etc/zensidus/server.toml`;nginx upstream `zensidus_backend`。
- VPS 一键部署/升级:`sudo bash install-server.sh`(zip 内自带,或 curl 仓库
  原始文件)——下载最新 Release zip → 装二进制 → **首次**才落 server.toml
  (升级绝不覆盖)→ daemon-reload,已运行则重启。
- TLS 在 Nginx 终结(CF 源站证书),本服务只听 `127.0.0.1:9720` 明文 TCP。
- `SERVER_CONFIG` 环境变量可覆盖配置路径(systemd 单元里直接传 argv,两者都可)。

## 陷阱(实测)

1. crate 声明 `unsafe_code = "deny"`:simd-json 0.19 的 `from_str(&mut str)`
   是 `unsafe fn` → 一律用 `simd_json::from_slice(&mut bytes)`;原地解析会
   **改写缓冲区**,web.rs 的错误快照(`short`)必须在解析前截取。
2. 认证 = TOTP:server.toml 持 `totp_secret`(Base32),客户端 Hello 携带
   认证器生成的 6 位码,无效/过期回 `AuthFail` 后断开;凭据(Alpaca/Finviz/
   DeepSeek)只存在于 server.toml,**永远不下发客户端**;动态码本身 30 秒过期、
   重放窗口极窄,客户端不持密钥(见 zensidus-client)。本地开发可设
   `dev_skip_totp = true`(任意码放行,启动/放行打 warning,默认 false,
   会话令牌逻辑不变)——**公网部署配置严禁开启**。
3. web.rs 三个代理是从 TS Gateway worker 逐行移植的:改行为前先对照原逻辑
   (IPO 表第 6 列跳过、futunn `marketType == 2`、数值列容错 0 等)——测试
   锚定了这些行为,改了行为要同步改测试。
4. `fan_out` 是**纯路由分发**(2026-11-23 起):Quote/Trade/Bar 按 hub 订阅表
   只发给订阅了该符号对应通道的客户端(`hub.subscribed` 查表,符号键大写
   归一),News 按 NEWS 位只发给订阅新闻的客户端(客户端 attach 时发
   `Subscribe{symbols:[], channels:NEWS}`);订阅协商全在 SubscriptionHub
   (纯逻辑)。排查"客户端收不到推送"先看 HubDiff 是否为空(并集没变化就
   不会下发),再看客户端是否订阅了该符号/新闻通道(路由按订阅表丢弃)。
   **server 不做行情计算**(规则过滤/桶聚合都在客户端)——VPS 算力约束,
   加服务端计算前先问。
5. **下行通道有界 + 有损**(session.rs `OUTBOUND_CAPACITY` 2048):慢客户端
   吃 `try_send` 丢帧(计 `dropped_frames`,30s `ingest health` 日志汇总),
   而不是无限堆积延迟。**通道操作一律 `try_send`**——`Sender::send` 是
   async,`let _ = tx.send(..)` 会把 Future 静默丢掉、帧永远发不出去
   (Welcome 消失这种"客户端等 60s 超时"的坑就是这么来的)。
6. `ingest health` 日志(30s 一条:lag_ms_max/messages/fan_out_dropped)是
   延迟归因的唯一手段:lag 大且无 drop = 上游(Alpaca→server)段被限速,
   客户端重连无效,等量能回落或换 feed;drop 增长 = 下发段慢客户端。
5. 心跳判定是"60s 无**任何**客户端帧",不是只看 Ping——客户端的 RPC 请求
   同样续命。
7. `/etc/zensidus/server.toml` 期望属主 **root:zensidus 0640**(服务以
   `zensidus` 用户读它)。root 身份手改文件会把属主换成 root:root → 服务
   读配置 Permission denied → **崩溃循环**(2026-11-23 实测)。root 编辑后
   `chown root:zensidus` 或用 `install -o root -g zensidus -m 640` 落盘。

## 测试

`cargo test -p server`(hub 并集/引用计数/subscribed 路由判定、session 认证
+心跳+订阅回放、mapping 条件位、finviz_data 换算/周窗口/筛选单元格定型、
web.rs 代理解析与列裁剪)。CI 的 linux-check 保证全依赖在 Linux 下编译
通过(musl 发布包另走 tag 流程)。
