# server

ZenSidus 行情中转服务:下游对 client 暴露一条 TCP(postcard 帧,协议见
`protocol`),上游经 AnySDK 对接 Alpaca 股票/新闻 WS 与 REST、Finviz
Elite 导出,并本地实现搜索(futunn)/翻译(DeepSeek)/IPO(iposcoop)。

## 本地运行

```bash
cp crates/server/server.toml.example crates/server/server.toml   # 填入凭据
cargo run -p server -- crates/server/server.toml
```

`server.toml` 已 gitignore。本机开发需代理时:`[proxy] ws_socks5` 让上游
WS 走 SOCKS5,HTTP 代理由 reqwest 读 `HTTPS_PROXY` 环境变量。

## 部署(VPS:CentOS Stream 10;Debian 系要点附后)

TLS 在 Nginx 终结:公网 8443 用 **Let's Encrypt 公共信任证书**(acme.sh 签发
+ 自动续期,客户端无需 pin 证书),本服务只听本机明文 TCP。客户端经
`stream.zensidus.com:8443` 灰云直连——原生 TCP 过不了 CF 的 HTTP 代理,DNS
记录必须是「仅 DNS」而非「已代理」。

```bash
dnf install -y nginx nginx-mod-stream unzip firewalld
systemctl enable --now firewalld
firewall-cmd --permanent --add-service=ssh
firewall-cmd --permanent --add-service=http
firewall-cmd --permanent --add-service=https
firewall-cmd --permanent --add-port=8443/tcp
firewall-cmd --reload
systemctl enable --now nginx   # 默认配置先跑起来:80 可达是 webroot 签发的前提
```

`/etc/nginx/nginx.conf` **主上下文**(http 块之外)必须 include stream 配置
——漏掉这行 stream 块会静默不生效,`nginx -T` 的加载列表里也看不到它
(实际踩过):

```nginx
include /etc/nginx/stream.conf;
```

证书签发(acme.sh,HTTP-01 webroot 走 80 端口,零 API 凭据;装到固定路径并
注册续期后自动 reload nginx 的钩子,续期 cron 由安装器自带)。**顺序敏感**:
必须在 nginx 已运行后签发,且引用证书的 stream 块要等证书落盘后再加入
nginx 配置——否则 `nginx -t` 因证书不存在而失败,80 起不来,签发死锁:

```bash
curl -fsSL https://get.acme.sh | sh -s email=you@example.com
mkdir -p /etc/ssl/zensidus-le
~/.acme.sh/acme.sh --issue -d stream.zensidus.com \
  --webroot /usr/share/nginx/html --server letsencrypt
~/.acme.sh/acme.sh --install-cert -d stream.zensidus.com \
  --fullchain-file /etc/ssl/zensidus-le/fullchain.pem \
  --key-file /etc/ssl/zensidus-le/key.pem \
  --reloadcmd "nginx -s reload"
```

`/etc/nginx/stream.conf`。端口用 8443:恰在 SELinux `http_port_t` 默认列表
内,无需 semanage;换其他高位端口要
`semanage port -a -t http_port_t -p tcp <端口>`(包 policycoreutils-python-utils):

```nginx
stream {
    upstream zensidus_backend {
        server 127.0.0.1:9720;
    }
    server {
        listen 8443 ssl;
        ssl_certificate     /etc/ssl/zensidus-le/fullchain.pem;   # LE,公共信任
        ssl_certificate_key /etc/ssl/zensidus-le/key.pem;
        proxy_pass zensidus_backend;
    }
}
```

客户端侧:`settings.server.addr = stream.zensidus.com:8443` + `tls = true`,
**`tls_cert` 留空**(LE 证书在公共信任链,走标准根校验;只有用 CF Origin 证书
这类私有 CA 时才需要 pin)。Debian 系备查:bookworm 官方 nginx 自带 stream
模块,无需单独装包。

`/etc/systemd/system/zensidus.service`:

```ini
[Unit]
Description=ZenSidus server
After=network-online.target

[Service]
ExecStart=/opt/zensidus/zensidus /etc/zensidus/server.toml
Restart=always
RestartSec=3
User=zensidus

[Install]
WantedBy=multi-user.target
```

构建产物为单一二进制,**不用本地有 Linux 机器**:仓库带 GitHub Actions
(`.github/workflows/server.yml`)——在 GitHub 仓库上打 `server-v*` tag(或在
Actions 页手动触发),musl 静态编译并发布到 Release(静态链接,任何 x86_64
Linux 发行版直接运行;main 分支每次 push 也有 Linux 编译检查)。产物只有一层
压缩包 `zensidus-linux-amd64.zip`(内含 `zensidus`、`server.toml.example`、
`install-server.sh`、`server.README.md`——打包时由 README.md 改名,避免与
仓库根 README 混淆)。

VPS 一键部署/升级(下载最新 Release、装二进制、落 systemd 单元,升级不覆盖
已有配置):

```bash
sudo bash install-server.sh                # zip 内自带
# 或不下载 zip 直接跑仓库版:
curl -fsSL https://raw.githubusercontent.com/ZZHENJIE/ZenSidus/main/crates/server/install-server.sh | sudo bash
sudo install-server.sh server-v0.1.0       # 指定 tag
```

首次安装会落一份样例配置,编辑 `/etc/zensidus/server.toml` 填入
`totp_secret`(Base32,认证器录入同一密钥)与上游凭据后
`systemctl restart zensidus`。**这些凭据没有自动化来源**——是整个部署中
唯一必须人工提供的部分(新部署需生成新 `totp_secret`:
`head -c 20 /dev/urandom | base32 | tr -d '='`)。

手动安装备选:下载上面的 zip 解压,把 `zensidus` 与 `server.toml` 放到服务器
对应路径;或在有 Rust 的机器上 `cargo build --release -p server`。

## 行为要点

- 首连:客户端 `Hello.auth` 携带认证器生成的 6 位 TOTP 动态码,服务端按 ±1 窗口校验,不符回 `AuthFail` 后断开;`Welcome` 顺带对时并下发会话令牌(UUID,内存保存,30 分钟 TTL,每次重连轮换)——客户端断线重连凭令牌免输码,服务端重启/令牌过期则重新过 TOTP。
- 心跳:客户端 10s `Ping`,服务端回 `Pong`;60s 无客户端帧判定死链断开。
- 订阅:per-client 意愿 → 上游并集(引用计数),客户端断开自动撤销。
- RPC 与推送共用同一条连接,按 id 路由;`bars` 拉取过程发 `RestProgress`。
- 上游断线由 AnySDK supervisor 自愈(指数退避 + 订阅回放),服务端只记日志。
