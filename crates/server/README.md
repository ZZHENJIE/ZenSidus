# relay-server

ZenSidus 行情中转服务:下游对 client 暴露一条 TCP(postcard 帧,协议见
`relay-protocol`),上游经 AnySDK 对接 Alpaca 股票/新闻 WS 与 REST、Finviz
Elite 导出,并本地实现搜索(futunn)/翻译(DeepSeek)/IPO(iposcoop)。

## 本地运行

```bash
cp crates/server/server.toml.example crates/server/server.toml   # 填入凭据
cargo run -p relay-server -- crates/server/server.toml
```

`server.toml` 已 gitignore。本机开发翻墙:`[proxy] ws_socks5` 走 SOCKS5,
HTTP 代理由 reqwest 读 `HTTPS_PROXY` 环境变量。

## 部署(JP VPS,Debian 系)

TLS 在 Nginx 终结(Cloudflare 颁发的源站证书),本服务只听本机明文 TCP。

`/etc/nginx/nginx.conf` 增加 stream 块(与 http 块平级;Debian 需 nginx 已编
stream 模块,bookworm 官方包自带):

```nginx
stream {
    upstream relay_backend {
        server 127.0.0.1:9720;
    }
    server {
        listen <公网端口> ssl;
        ssl_certificate     /etc/ssl/zensidus/origin.pem;   # CF 源站证书
        ssl_certificate_key /etc/ssl/zensidus/origin.key;
        proxy_pass relay_backend;
    }
}
```

客户端侧:`settings.relay.tls = true` + `tls_cert` 指向同一张源站证书
(CF Origin 证书不在公共信任链,客户端按固定证书校验)。

`/etc/systemd/system/relay-server.service`:

```ini
[Unit]
Description=ZenSidus relay server
After=network-online.target

[Service]
ExecStart=/opt/relay-server/relay-server /etc/relay-server/server.toml
Restart=always
RestartSec=3
User=relay

[Install]
WantedBy=multi-user.target
```

构建产物为单一二进制,**不用本地有 Linux 机器**:仓库带 GitHub Actions
(`.github/workflows/relay-server.yml`)——在 GitHub 仓库上打 `server-v*` tag(或在
Actions 页手动触发),musl 静态编译的 `relay-server-linux-amd64.tar.gz` 会自动
发布到 Release(静态链接,任何 x86_64 Linux 发行版直接运行;main 分支每次 push
也有 Linux 编译检查)。下载解压后把 `relay-server` 与 `server.toml` 放到服务器
对应路径即可。备选:在有 Rust 的机器上 `cargo build --release -p relay-server`。

## 行为要点

- 首连:客户端 `Hello.token` 与配置比对,不符断开;`Welcome` 顺带对时。
- 心跳:客户端 10s `Ping`,服务端回 `Pong`;60s 无客户端帧判定死链断开。
- 订阅:per-client 意愿 → 上游并集(引用计数),客户端断开自动撤销。
- RPC 与推送共用同一条连接,按 id 路由;`bars` 拉取过程发 `RestProgress`。
- 上游断线由 AnySDK supervisor 自愈(指数退避 + 订阅回放),服务端只记日志。
