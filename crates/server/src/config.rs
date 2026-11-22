//! 服务端配置(`server.toml`)。真实文件不入库:复制 `server.toml.example`
//! 后填入凭据。部署形态见 README(Nginx 终结 TLS,本服务只听明文 TCP)。

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Config {
    /// 下游监听地址,如 `127.0.0.1:9720`(公网暴露由 Nginx 转发)。
    pub listen: String,
    /// TOTP 预共享密钥,必须是合法 RFC 4648 Base32:**仅 `A-Z` 与 `2-7`**
    /// (没有 0/1/8/9;大小写不敏感)。(Git Bash)`head -c 20 /dev/urandom | base32 | tr -d '='`
    /// (20 字节正好 32 字符,无 padding)。客户端 `Hello.code` 携带当前
    /// 动态码,±1 窗口(±30s)内有效。
    pub totp_secret: String,
    /// **仅本地开发**:跳过 TOTP 校验,任意动态码都放行(会话令牌逻辑
    /// 不变)。默认 false;开启时启动与每次放行都会打 warning,切勿在
    /// 任何有真实监听地址/公网转发(README 部署形态)的配置里打开。
    #[serde(default)]
    pub dev_skip_totp: bool,
    /// tracing 过滤级别(默认 `info`)。
    #[serde(default = "default_log_level")]
    pub log_level: String,

    pub alpaca: AlpacaConfig,
    pub finviz: FinvizConfig,
    pub deepseek: DeepSeekConfig,
    /// 可选出站代理(本机开发用;VPS 上通常不需要)。
    #[serde(default)]
    pub proxy: Option<ProxyConfig>,
}

#[derive(Debug, Deserialize)]
pub struct AlpacaConfig {
    pub key_id: String,
    pub secret: String,
    /// 任意 [`anysdk::alpaca_sdk::Feed`] 小写线值:`sip` / `iex` / `otc` /
    /// `delayed_sip` / `boats` / `overnight`。
    pub feed: anysdk::alpaca_sdk::Feed,
}

#[derive(Debug, Deserialize)]
pub struct FinvizConfig {
    pub token: String,
}

#[derive(Debug, Deserialize)]
pub struct DeepSeekConfig {
    pub api_key: String,
}

#[derive(Debug, Deserialize)]
pub struct ProxyConfig {
    /// 上游 WS(SOCKS5)代理,`socks5://host:port`;HTTP 代理走环境变量
    /// (reqwest 默认读 `HTTPS_PROXY`)。
    pub ws_socks5: Option<String>,
}

fn default_log_level() -> String {
    "info".to_string()
}

impl Config {
    pub fn load(path: &str) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)
            .map_err(|err| anyhow::anyhow!("failed to read config {path:?}: {err}"))?;
        let config: Self = toml::from_str(&raw)
            .map_err(|err| anyhow::anyhow!("failed to parse config {path:?}: {err}"))?;
        Ok(config)
    }
}
