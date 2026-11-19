#!/usr/bin/env bash
# ZenSidus server 一键部署/升级:从 GitHub Release 下载 zensidus-linux-amd64.zip
# (默认最新,可传 tag),安装二进制到 /opt/zensidus,首次安装落一份
# /etc/zensidus/server.toml,并装配 systemd 单元 zensidus.service。
#
# 用法(root;Debian 系):
#   bash install-server.sh                # 最新 Release
#   bash install-server.sh server-v0.3.0  # 指定 tag
# 升级幂等:已存在的 server.toml 不会被覆盖;服务在运行则重启换新二进制。
set -euo pipefail

REPO="ZZHENJIE/ZenSidus"
ASSET="zensidus-linux-amd64.zip"
INSTALL_DIR="/opt/zensidus"
CONFIG="/etc/zensidus/server.toml"
SERVICE_NAME="zensidus"
RUN_USER="zensidus"

[[ $EUID -eq 0 ]] || { echo "需要 root:sudo bash install-server.sh" >&2; exit 1; }
# unzip 缺失时在 Debian 系上自动补装,其余依赖缺失直接报错
if ! command -v unzip >/dev/null 2>&1 && command -v apt-get >/dev/null 2>&1; then
  apt-get update -qq && apt-get install -y -qq unzip
fi
for cmd in curl unzip systemctl install; do
  command -v "$cmd" >/dev/null 2>&1 || { echo "缺少依赖:$cmd" >&2; exit 1; }
done

VERSION="${1:-latest}"

# ── 1. 定位 Release 下载 URL ────────────────────────────────────────────
case "$VERSION" in
  latest) api="https://api.github.com/repos/$REPO/releases/latest" ;;
  *)      api="https://api.github.com/repos/$REPO/releases/tags/$VERSION" ;;
esac
url=$(curl -fsSL "$api" | grep -o "https://[^\"]*$ASSET" | head -n 1)
[[ -n "$url" ]] || { echo "该 Release 没有 $ASSET 资产" >&2; exit 1; }
echo "==> 下载 $url"

# ── 2. 下载并解压到临时目录 ─────────────────────────────────────────────
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
curl -fSL --retry 3 -o "$tmp/$ASSET" "$url"
unzip -oq "$tmp/$ASSET" -d "$tmp"

# ── 3. 运行用户 ────────────────────────────────────────────────────────
id "$RUN_USER" >/dev/null 2>&1 ||
  useradd --system --no-create-home --shell /usr/sbin/nologin "$RUN_USER"

# ── 4. 二进制(单一静态文件,替换即升级)────────────────────────────────
install -m 0755 "$tmp/zensidus" "$INSTALL_DIR/zensidus"

# ── 5. 配置:只在首次落盘,绝不覆盖已有配置 ─────────────────────────────
fresh=0
if [[ ! -f "$CONFIG" ]]; then
  mkdir -p "$(dirname "$CONFIG")"
  install -m 0600 "$tmp/server.toml.example" "$CONFIG"
  fresh=1
  echo "==> 已生成初始配置 $CONFIG(样例值,启动前填入真实凭据)"
else
  echo "==> 保留现有配置 $CONFIG"
fi

# ── 6. systemd 单元(与 README 部署段一致)────────────────────────────
cat > "/etc/systemd/system/$SERVICE_NAME.service" <<EOF
[Unit]
Description=ZenSidus server
After=network-online.target

[Service]
ExecStart=$INSTALL_DIR/zensidus $CONFIG
Restart=always
RestartSec=3
User=$RUN_USER

[Install]
WantedBy=multi-user.target
EOF
systemctl daemon-reload

# ── 7. 启动/升级 ───────────────────────────────────────────────────────
if [[ $fresh -eq 1 ]]; then
  systemctl enable "$SERVICE_NAME" >/dev/null 2>&1
  echo "==> 首次安装:编辑 $CONFIG 填好凭据后执行"
  echo "    systemctl enable --now $SERVICE_NAME"
elif systemctl is-active --quiet "$SERVICE_NAME"; then
  systemctl restart "$SERVICE_NAME"
  echo "==> 已重启 $SERVICE_NAME,运行新版本"
else
  systemctl enable --now "$SERVICE_NAME" >/dev/null 2>&1
  echo "==> 已启动 $SERVICE_NAME"
fi
systemctl --no-pager --lines=0 status "$SERVICE_NAME" || true
echo "==> 日志:journalctl -u $SERVICE_NAME -f"
