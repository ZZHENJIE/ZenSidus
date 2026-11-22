#!/usr/bin/env bash
# ZenSidus server 一键部署/升级:从 GitHub Release 下载 zensidus-linux-amd64.zip
# (默认最新,可传 tag),安装二进制到 /opt/zensidus,首次安装落一份
# /etc/zensidus/server.toml,并装配 systemd 单元 zensidus.service。
#
# 用法(root;Debian / EL 系):
#   bash install-server.sh                # 最新 server-v* Release
#   bash install-server.sh server-v0.1.0  # 指定 tag
# 升级幂等:已存在的 server.toml 不会被覆盖;服务在运行则重启换新二进制。
set -euo pipefail

REPO="ZZHENJIE/ZenSidus"
ASSET="zensidus-linux-amd64.zip"
INSTALL_DIR="/opt/zensidus"
CONFIG="/etc/zensidus/server.toml"
SERVICE_NAME="zensidus"
RUN_USER="zensidus"

[[ $EUID -eq 0 ]] || { echo "需要 root:sudo bash install-server.sh" >&2; exit 1; }
# unzip 缺失时按包管理器自动补装,其余依赖缺失直接报错
if ! command -v unzip >/dev/null 2>&1; then
  if command -v apt-get >/dev/null 2>&1; then
    apt-get update -qq && apt-get install -y -qq unzip
  elif command -v dnf >/dev/null 2>&1; then
    dnf install -y -q unzip
  fi
fi
for cmd in curl unzip systemctl install; do
  command -v "$cmd" >/dev/null 2>&1 || { echo "缺少依赖:$cmd" >&2; exit 1; }
done

VERSION="${1:-latest}"

# ── 1. 定位 Release 下载 URL ────────────────────────────────────────────
# latest 不取 /releases/latest:仓库将来可能有 client-v* 等其他 release,
# 只认 server-v* tag,取其中最新一个。
case "$VERSION" in
  latest)
    tag=$(curl -fsSL "https://api.github.com/repos/$REPO/releases?per_page=100" \
      | grep -o '"tag_name": *"[^"]*"' \
      | grep -o 'server-v[^"]*' | head -n 1 || true)
    [[ -n "$tag" ]] || { echo "仓库没有任何 server-v* Release" >&2; exit 1; }
    api="https://api.github.com/repos/$REPO/releases/tags/$tag"
    ;;
  *) api="https://api.github.com/repos/$REPO/releases/tags/$VERSION" ;;
esac
# grep 无匹配时管道退出码会被 pipefail 放大成整条命令失败,|| true 保住
# 下面的友好报错(否则脚本静默退出 1)
url=$(curl -fsSL "$api" | grep -o "https://[^\"]*$ASSET" | head -n 1 || true)
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
# 全新 VPS 上目录不存在,install 不建目录直接报 No such file or directory
mkdir -p "$INSTALL_DIR"
install -m 0755 "$tmp/zensidus" "$INSTALL_DIR/zensidus"

# ── 5. 配置:只在首次落盘,绝不覆盖已有配置 ─────────────────────────────
# 属主 root:$RUN_USER 0640——服务以 $RUN_USER 运行,配置必须可读;
# 0600 root:root 会让首次安装直接进 crash loop。
fresh=0
if [[ ! -f "$CONFIG" ]]; then
  mkdir -p "$(dirname "$CONFIG")"
  install -o root -g "$RUN_USER" -m 0640 "$tmp/server.toml.example" "$CONFIG"
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
