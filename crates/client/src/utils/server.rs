//! server 客户端:应用唯一的对外连接(替代 executor 三客户端 + 两条 WS)。
//!
//! 一条到 server 的 TCP 长连接(`[u32 LE 长度][postcard Frame]`),自带
//! 自动重连(指数退避 1s→60s)+ 心跳(10s Ping)+ RPC id 路由;推送帧解出后
//! 映射回 anysdk 的 [`StockMessage`](anysdk::alpaca_stream::StockMessage) /
//! 新闻帧广播——`stream_dispatch` 级联因此零改动。
//!
//! 本模块是纯 tokio,不依赖 GPUI:连接状态机由后台 actor 任务驱动,UI 侧只
//! 拿广播接收端与命令发送端。订阅重放不在本模块——期望状态归 [`crate::utils::state::Hub`]
//! 所有,它在每次 `Connected` 事件时重发并集(与旧 Hub 同语义)。

use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};
use std::time::Duration;

use anyhow::Context as _;
use chrono::{TimeZone, Utc};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::tr;
use tokio::sync::{broadcast, mpsc, oneshot};

use anysdk::alpaca_stream::StockMessage;
use anysdk::alpaca_stream::stocks::{BarMessage, QuoteMessage, TradeMessage};
use protocol::{
    BarData, Channels, Frame, HelloAuth, NewsData, RpcRequestPayload, RpcResponsePayload,
};

/// 推送广播容量:与旧 anysdk 流一致。
const STOCK_CAPACITY: usize = 4096;
/// 新闻低频,256 足够。
const NEWS_CAPACITY: usize = 256;
const STATUS_CAPACITY: usize = 16;
/// 心跳间隔(与 server 约定 60s 读超时)。
const PING_INTERVAL: Duration = Duration::from_secs(10);
/// 会话静默上限:超过即认为链路死了(有数据流动时 Pong/推送都会刷新)。
const SILENT_AFTER: Duration = Duration::from_secs(30);
/// Hello→Welcome 握手时限。
const WELCOME_TIMEOUT: Duration = Duration::from_secs(15);
/// 重连退避:1s 起步,翻倍封顶 60s。
const BACKOFF_START: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(60);

/// 连接状态(镜像给 hub 的状态灯;`Failed` 为永久失败——token 被拒)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerStatus {
    Disconnected,
    /// 首连或重连退避中。
    Connecting,
    Connected,
    /// 服务端拒绝(token 不符);重试无意义,actor 已停止。
    Failed(String),
}

/// 对时与往返延迟的最新估计(毫秒)。`offset_ms > 0` 表示服务端钟快。
#[derive(Debug, Default)]
pub struct ClockEstimate {
    pub offset_ms: AtomicI64,
    pub rtt_ms: AtomicI64,
    /// 是否有过成功的对时样本。
    valid: AtomicU32,
}

impl ClockEstimate {
    fn set(&self, offset_ms: i64, rtt_ms: i64) {
        self.offset_ms.store(offset_ms, Ordering::Relaxed);
        self.rtt_ms.store(rtt_ms, Ordering::Relaxed);
        self.valid.store(1, Ordering::Relaxed);
    }

    /// 最新估计;`None` = 尚无样本。
    pub fn get(&self) -> Option<(i64, i64)> {
        (self.valid.load(Ordering::Relaxed) == 1).then(|| {
            (
                self.offset_ms.load(Ordering::Relaxed),
                self.rtt_ms.load(Ordering::Relaxed),
            )
        })
    }
}

/// 连接配置(地址/TLS 从 settings 现读;`code` 来自连接对话框,是本次
/// 连接使用的 TOTP 动态码——30 秒过期,过期后的重连由人工输新码发起)。
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// `host:port`。
    pub addr: String,
    pub code: String,
    /// true = TLS(服务器 Nginx 终结 TLS)。
    pub tls: bool,
    /// 固定校验的证书(PEM;CF Origin 证书不在公共信任链)。`None` = 标准
    /// webpki 根校验。
    pub tls_cert: Option<PathBuf>,
}

enum Command {
    Subscribe {
        symbols: Vec<String>,
        channels: Channels,
    },
    Unsubscribe {
        symbols: Vec<String>,
        channels: Channels,
    },
    Rpc {
        id: u32,
        payload: RpcRequestPayload,
        reply: oneshot::Sender<Result<RpcResponsePayload, String>>,
    },
    /// 强制断开并立即重连(staleness 看门狗用)。
    Reconnect,
    /// 停止 actor(手动断开)。
    Close,
}

struct Inner {
    commands: mpsc::UnboundedSender<Command>,
    stock_tx: broadcast::Sender<StockMessage>,
    news_tx: broadcast::Sender<NewsData>,
    progress_tx: broadcast::Sender<(u32, u64)>,
    status_tx: broadcast::Sender<ServerStatus>,
    version_tx: broadcast::Sender<String>,
    clock: Arc<ClockEstimate>,
    next_rpc_id: AtomicU32,
}

/// 应用级 server 客户端句柄:Clone 廉价,关闭靠 [`ServerClient::close`]。
#[derive(Clone)]
pub struct ServerClient {
    inner: Arc<Inner>,
}

impl ServerClient {
    /// 构建并启动后台 actor(不阻塞;连接在后台建立,状态经 `status()` 广播)。
    pub fn start(runtime: tokio::runtime::Handle, config: ServerConfig) -> anyhow::Result<Self> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let (commands, command_rx) = mpsc::unbounded_channel();
        let (stock_tx, _) = broadcast::channel(STOCK_CAPACITY);
        let (news_tx, _) = broadcast::channel(NEWS_CAPACITY);
        let (progress_tx, _) = broadcast::channel(16);
        let (status_tx, _) = broadcast::channel(STATUS_CAPACITY);
        let (version_tx, _) = broadcast::channel(4);

        let inner = Arc::new(Inner {
            commands,
            stock_tx,
            news_tx,
            progress_tx,
            status_tx,
            version_tx,
            clock: Arc::new(ClockEstimate::default()),
            next_rpc_id: AtomicU32::new(1),
        });

        runtime.spawn(run_actor(config, Arc::clone(&inner), command_rx));
        Ok(Self { inner })
    }

    pub fn subscribe(&self, symbols: Vec<String>, channels: Channels) {
        let _ = self
            .inner
            .commands
            .send(Command::Subscribe { symbols, channels });
    }

    pub fn unsubscribe(&self, symbols: Vec<String>, channels: Channels) {
        let _ = self
            .inner
            .commands
            .send(Command::Unsubscribe { symbols, channels });
    }

    /// 发起一次 RPC;响应(或连接丢失/离线错误)经返回的 future 送达。
    pub async fn rpc(&self, payload: RpcRequestPayload) -> Result<RpcResponsePayload, String> {
        let id = self.inner.next_rpc_id.fetch_add(1, Ordering::Relaxed);
        let (reply, rx) = oneshot::channel();
        self.inner
            .commands
            .send(Command::Rpc { id, payload, reply })
            .map_err(|_| "server client shut down".to_string())?;
        rx.await
            .map_err(|_| "server client shut down".to_string())?
    }

    /// [`ServerClient::rpc`] 的 anyhow 版:面板调用点用 `?` 时无需手动转换
    /// 错误字符串。
    pub async fn rpc_checked(
        &self,
        payload: RpcRequestPayload,
    ) -> anyhow::Result<RpcResponsePayload> {
        self.rpc(payload).await.map_err(|err| anyhow::anyhow!(err))
    }

    /// 实时行情消息(已映射回 anysdk 类型,`stream_dispatch` 直接消费)。
    pub fn stock_messages(&self) -> broadcast::Receiver<StockMessage> {
        self.inner.stock_tx.subscribe()
    }

    /// 市场新闻推送帧。
    pub fn news_messages(&self) -> broadcast::Receiver<NewsData> {
        self.inner.news_tx.subscribe()
    }

    /// bars RPC 的翻页进度 `(rpc_id, 累计条数)`。
    pub fn progress(&self) -> broadcast::Receiver<(u32, u64)> {
        self.inner.progress_tx.subscribe()
    }

    /// server 版本(Welcome 下发,每次连接收到一帧;设置页展示用)。
    pub fn version_updates(&self) -> broadcast::Receiver<String> {
        self.inner.version_tx.subscribe()
    }

    pub fn status(&self) -> broadcast::Receiver<ServerStatus> {
        self.inner.status_tx.subscribe()
    }

    /// 对时估计的共享句柄(`get()` 给 `(offset_ms, rtt_ms)`,尚无样本为
    /// `None`)。`Arc` 共享:后台线程的 lag 探针拿它做延迟补偿。
    pub fn clock(&self) -> Arc<ClockEstimate> {
        Arc::clone(&self.inner.clock)
    }

    /// 强制断开并立即重连(staleness 看门狗换新路由)。
    pub fn force_reconnect(&self) {
        let _ = self.inner.commands.send(Command::Reconnect);
    }

    /// 手动断开:actor 停止,重连结束。
    pub fn close(&self) {
        let _ = self.inner.commands.send(Command::Close);
    }
}

/// 后台 actor:连接状态机。见模块注释。
async fn run_actor(
    config: ServerConfig,
    inner: Arc<Inner>,
    mut commands: mpsc::UnboundedReceiver<Command>,
) {
    let mut backoff = BACKOFF_START;
    let mut close_requested = false;
    // 认证成功后服务端下发的会话令牌:断线重连时替代 TOTP 码(免输)。
    // 仅存活于本 actor(内存),客户端重启即重新走 TOTP。
    let mut session_token: Option<String> = None;
    set_status(&inner, ServerStatus::Connecting);

    loop {
        let connect_result = connect_stream(&config).await;
        let stream = match connect_result {
            Ok(stream) => stream,
            Err(err) => {
                tracing::warn!(error = format!("{err:#}"), "server connect failed");
                if sleep_backoff(&mut backoff, &mut commands, &mut close_requested).await {
                    break;
                }
                continue;
            }
        };

        match run_session(
            stream,
            &config,
            &mut session_token,
            Arc::clone(&inner),
            &mut commands,
        )
        .await
        {
            SessionEnd::Closed => break,
            SessionEnd::Rejected(reason) => {
                // 服务端拒绝(token 不符等):重试无意义。
                set_status(&inner, ServerStatus::Failed(reason));
                break;
            }
            SessionEnd::Dropped(reason) => {
                tracing::warn!(reason, "server session dropped; reconnecting");
                set_status(&inner, ServerStatus::Connecting);
                if sleep_backoff(&mut backoff, &mut commands, &mut close_requested).await {
                    break;
                }
            }
        }
    }
    set_status(&inner, ServerStatus::Disconnected);
}

/// 退避等待;期间继续消费命令(RPC 立即报离线,Reconnect 跳过剩余等待)。
/// 返回 true = 收到 Close,actor 应退出。
async fn sleep_backoff(
    backoff: &mut Duration,
    commands: &mut mpsc::UnboundedReceiver<Command>,
    close_requested: &mut bool,
) -> bool {
    let timer = tokio::time::sleep(*backoff);
    tokio::pin!(timer);
    loop {
        tokio::select! {
            () = &mut timer => {
                *backoff = (*backoff * 2).min(BACKOFF_MAX);
                return false;
            }
            command = commands.recv() => match command {
                None | Some(Command::Close) => {
                    *close_requested = true;
                    return true;
                }
                Some(Command::Rpc { reply, .. }) => {
                    let _ = reply.send(Err("server offline".into()));
                }
                // 离线期间的订阅丢弃:hub 在 Connected 时重发并集。
                Some(Command::Subscribe { .. }) | Some(Command::Unsubscribe { .. }) => {}
                Some(Command::Reconnect) => {}
            },
        }
    }
}

enum SessionEnd {
    /// 手动关闭。
    Closed,
    /// 服务端拒绝(token 不符)——永久失败。
    Rejected(String),
    /// 链路断了——可重连。
    Dropped(&'static str),
}

/// 单次会话:Hello → Welcome → 双向收发,直到链路断/手动关。
/// 返回前把在途 RPC 全部报"connection lost"。`session_token` 进出同一处:
/// 进 = 上次认证下发的令牌(优先于 TOTP 码);出 = 本次 Welcome 轮换的新
/// 令牌,actor 持有它供后续重连使用。
async fn run_session(
    mut stream: Box<dyn AsyncReadWrite>,
    config: &ServerConfig,
    session_token: &mut Option<String>,
    inner: Arc<Inner>,
    commands: &mut mpsc::UnboundedReceiver<Command>,
) -> SessionEnd {
    let mut rpcs: HashMap<u32, oneshot::Sender<Result<RpcResponsePayload, String>>> =
        HashMap::new();
    let fail_all_rpcs = |rpcs: &mut HashMap<u32, oneshot::Sender<_>>, reason: &str| {
        for (_, reply) in rpcs.drain() {
            let _ = reply.send(Err(reason.to_string()));
        }
    };

    // Hello:重连优先用会话令牌(免输动态码);首次连接才用 TOTP 码。
    let auth = match session_token {
        Some(token) => HelloAuth::Session(token.clone()),
        None => HelloAuth::Totp(config.code.clone()),
    };
    let hello = Frame::Hello {
        auth,
        app_version: concat!("ZenSidus/", env!("CARGO_PKG_VERSION")).to_string(),
    };
    if write_frame(&mut stream, &hello).await.is_err() {
        fail_all_rpcs(&mut rpcs, "connection lost");
        return SessionEnd::Dropped("hello write failed");
    }

    // Welcome(带对时)。
    let mut decoder = FrameDecoderBuf::default();
    let mut read_buf = [0u8; 16 * 1024];
    let welcome_at = std::time::Instant::now();
    let welcome = loop {
        match tokio::time::timeout(WELCOME_TIMEOUT, stream.read(&mut read_buf)).await {
            Ok(Ok(0)) | Ok(Err(_)) => {
                fail_all_rpcs(&mut rpcs, "connection lost");
                return SessionEnd::Dropped("closed before welcome");
            }
            Ok(Ok(n)) => match decoder.feed(&read_buf[..n]) {
                Ok(frames) => {
                    // 认证被拒(动态码无效/过期、会话令牌失效):终态——
                    // 重试无意义,等人工输新码(与网络断开的 Dropped 语义相反)。
                    if let Some(Frame::AuthFail { .. }) =
                        frames.iter().find(|f| matches!(f, Frame::AuthFail { .. }))
                    {
                        fail_all_rpcs(&mut rpcs, "auth failed");
                        // 令牌已失效,清掉:下次连接回到 TOTP 码。
                        *session_token = None;
                        return SessionEnd::Rejected(tr!("app.log.auth_rejected").to_string());
                    }
                    if let Some(Frame::Welcome {
                        server_time_ms,
                        session_token: fresh,
                        version,
                    }) = frames
                        .into_iter()
                        .find(|f| matches!(f, Frame::Welcome { .. }))
                    {
                        // 轮换:服务端每次认证都下发新令牌,旧的立即作废。
                        *session_token = Some(fresh);
                        // server 版本随 Welcome 下发(设置页 About 展示)。
                        let _ = inner.version_tx.send(version);
                        break server_time_ms;
                    }
                }
                Err(err) => {
                    fail_all_rpcs(&mut rpcs, "connection lost");
                    return SessionEnd::Rejected(format!("protocol error: {err}"));
                }
            },
            Err(_elapsed) => {
                fail_all_rpcs(&mut rpcs, "connection lost");
                return SessionEnd::Dropped("welcome timeout");
            }
        }
    };
    // offset ≈ 服务端钟 − 本地钟(采样点即到达点;RTT 的另一半尚不可知,
    // 精度由 Pong 的 RTT 供上层自行校正)。打一条日志:lag 行里
    // oldest_max 的恒定偏差可以直接对照它分辨"本机时钟偏移 vs 真延迟"。
    let offset_ms = server_time_offset(welcome, welcome_at);
    let rtt_guess = welcome_at.elapsed().as_millis() as i64 / 2;
    inner.clock.set(offset_ms, rtt_guess);
    tracing::info!(offset_ms, rtt_guess, "server clock estimate from welcome");

    set_status(&inner, ServerStatus::Connected);
    let mut ping = tokio::time::interval(PING_INTERVAL);
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last_activity = std::time::Instant::now();

    let outcome = loop {
        tokio::select! {
            read = stream.read(&mut read_buf) => match read {
                Ok(0) => {
                    break SessionEnd::Dropped("server closed");
                }
                Ok(n) => {
                    last_activity = std::time::Instant::now();
                    let frames = match decoder.feed(&read_buf[..n]) {
                        Ok(frames) => frames,
                        Err(err) => break SessionEnd::Rejected(format!("protocol error: {err}")),
                    };
                    for frame in frames {
                        match frame {
                            Frame::Quote(_) | Frame::Trade(_) | Frame::Bar(_) => {
                                if let Some(message) = frame_to_stock_message(frame) {
                                    let _ = inner.stock_tx.send(message);
                                }
                            }
                            Frame::News(news) => {
                                let _ = inner.news_tx.send(news);
                            }
                            Frame::RestProgress { req_id, fetched } => {
                                let _ = inner.progress_tx.send((req_id, fetched));
                            }
                            Frame::RpcResponse { id, result } => {
                                if let Some(reply) = rpcs.remove(&id) {
                                    let _ = reply.send(result);
                                }
                            }
                            Frame::Subscribed { .. } => {}
                            Frame::Pong => {}
                            // 服务端不会发这些。
                            _ => {}
                        }
                    }
                }
                Err(_) => break SessionEnd::Dropped("read error"),
            },

            command = commands.recv() => match command {
                None | Some(Command::Close) => {
                    break SessionEnd::Closed;
                }
                Some(Command::Subscribe { symbols, channels }) => {
                    let frame = Frame::Subscribe { id: next_sub_id(), symbols, channels };
                    if write_frame(&mut stream, &frame).await.is_err() {
                        break SessionEnd::Dropped("subscribe write failed");
                    }
                }
                Some(Command::Unsubscribe { symbols, channels }) => {
                    let frame = Frame::Unsubscribe { id: next_sub_id(), symbols, channels };
                    if write_frame(&mut stream, &frame).await.is_err() {
                        break SessionEnd::Dropped("unsubscribe write failed");
                    }
                }
                Some(Command::Rpc { id, payload, reply }) => {
                    let frame = Frame::RpcRequest { id, payload };
                    if write_frame(&mut stream, &frame).await.is_err() {
                        let _ = reply.send(Err("connection lost".into()));
                        break SessionEnd::Dropped("rpc write failed");
                    }
                    rpcs.insert(id, reply);
                }
                Some(Command::Reconnect) => {
                    break SessionEnd::Dropped("forced reconnect");
                }
            },

            _ = ping.tick() => {
                if last_activity.elapsed() > SILENT_AFTER {
                    break SessionEnd::Dropped("silent link");
                }
                if write_frame(&mut stream, &Frame::Ping).await.is_err() {
                    break SessionEnd::Dropped("ping write failed");
                }
            }
        }
    };

    fail_all_rpcs(&mut rpcs, "connection lost");
    outcome
}

// ---------- 时间 ----------

fn server_time_offset(server_time_ms: i64, sampled_at: std::time::Instant) -> i64 {
    let local_ms = Utc::now().timestamp_millis();
    server_time_ms - local_ms - sampled_at.elapsed().as_millis() as i64 / 2
}

fn ms_to_time(ms: i64) -> chrono::DateTime<Utc> {
    Utc.timestamp_millis_opt(ms).single().unwrap_or_default()
}

fn frame_to_stock_message(frame: Frame) -> Option<StockMessage> {
    match frame {
        Frame::Quote(q) => Some(StockMessage::Quote(QuoteMessage {
            symbol: q.symbol,
            ask_exchange: q.ask_exchange,
            ask_price: q.ask_price,
            ask_size: q.ask_size,
            bid_exchange: q.bid_exchange,
            bid_price: q.bid_price,
            bid_size: q.bid_size,
            conditions: Vec::new(),
            time: ms_to_time(q.time_ms),
            tape: None,
        })),
        Frame::Trade(t) => Some(StockMessage::Trade(TradeMessage {
            symbol: t.symbol,
            id: t.id,
            exchange: t.exchange,
            price: t.price,
            size: t.size,
            conditions: t.conditions,
            time: ms_to_time(t.time_ms),
            tape: None,
        })),
        Frame::Bar(b) => {
            let BarData {
                symbol,
                kind,
                open,
                high,
                low,
                close,
                volume,
                time_ms,
            } = b;
            let bar = BarMessage {
                symbol,
                open,
                high,
                low,
                close,
                volume,
                // 协议草图不带这两个字段;历史数据走 RPC,补零即可。
                vwap: 0.0,
                trade_count: 0,
                time: ms_to_time(time_ms),
            };
            Some(match kind {
                protocol::BarKind::Minute => StockMessage::MinuteBar(bar),
                protocol::BarKind::Updated => StockMessage::UpdatedBar(bar),
                protocol::BarKind::Daily => StockMessage::DailyBar(bar),
            })
        }
        _ => None,
    }
}

// ---------- 帧编解码 ----------

/// 订阅 id:进程内自增即可(服务端只回显,不关联合并)。
fn next_sub_id() -> u32 {
    use std::sync::atomic::AtomicU32;
    static NEXT: AtomicU32 = AtomicU32::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn write_frame<'a>(
    stream: &'a mut Box<dyn AsyncReadWrite>,
    frame: &'a Frame,
) -> impl Future<Output = io::Result<()>> + Send + 'a {
    let bytes = protocol::encode_frame(frame);
    async move { stream.write_all(&bytes).await }
}

/// 帧解码器包装(读循环用)。
#[derive(Default)]
struct FrameDecoderBuf {
    decoder: protocol::FrameDecoder,
}

impl FrameDecoderBuf {
    fn feed(&mut self, bytes: &[u8]) -> Result<Vec<Frame>, protocol::FrameDecodeError> {
        self.decoder.feed(bytes)
    }
}

/// 传输流的类型擦除(明文 TCP 或 TLS)。
type BoxStream = Box<dyn AsyncReadWrite>;

trait AsyncReadWrite: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> AsyncReadWrite for T {}

async fn connect_stream(config: &ServerConfig) -> anyhow::Result<BoxStream> {
    // 带上目标地址:解析失败(如缺端口)与远端 RST 在日志里才分得清。
    let tcp = tokio::net::TcpStream::connect(&config.addr)
        .await
        .with_context(|| format!("tcp connect {}", config.addr))?;
    tcp.set_nodelay(true)?;
    if !config.tls {
        return Ok(Box::new(tcp));
    }

    let host = config
        .addr
        .rsplit_once(':')
        .map(|(host, _)| host.to_string())
        .unwrap_or_else(|| config.addr.clone());
    let server_name = rustls::pki_types::ServerName::try_from(host)
        .map_err(|err| anyhow::anyhow!("invalid server name: {err}"))?;

    let tls_config = match &config.tls_cert {
        Some(path) => {
            let pinned = load_pinned_cert(path)?;
            rustls::ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(PinnedCertVerifier { pinned }))
                .with_no_client_auth()
        }
        None => {
            let mut roots = rustls::RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            rustls::ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth()
        }
    };

    let connector = tokio_rustls::TlsConnector::from(Arc::new(tls_config));
    let stream = connector.connect(server_name, tcp).await?;
    Ok(Box::new(stream))
}

/// 读取 PEM 文件的第一个证书为 DER(固定校验的基准)。
fn load_pinned_cert(path: &PathBuf) -> anyhow::Result<Vec<u8>> {
    let pem = std::fs::read(path)?;
    let mut reader = std::io::BufReader::new(pem.as_slice());
    let cert = rustls_pemfile::certs(&mut reader)
        .next()
        .transpose()?
        .ok_or_else(|| anyhow::anyhow!("no certificate in {}", path.display()))?;
    Ok(cert.as_ref().to_vec())
}

/// 固定证书校验器:只接受呈现的叶子证书与预置证书逐字节一致的链。
/// (CF Origin 证书不在公共信任链;握手层的签名校验仍由 rustls 核心
/// 用默认 provider 完成,与本验证器无关。)
#[derive(Debug)]
struct PinnedCertVerifier {
    pinned: Vec<u8>,
}

impl rustls::client::danger::ServerCertVerifier for PinnedCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        if end_entity.as_ref() == self.pinned.as_slice() {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General(
                "server certificate does not match the pinned certificate".into(),
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        let provider = rustls::crypto::ring::default_provider();
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        let provider = rustls::crypto::ring::default_provider();
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn set_status(inner: &Inner, status: ServerStatus) {
    let _ = inner.status_tx.send(status);
}
