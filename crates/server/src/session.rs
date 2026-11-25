//! 客户端会话:每条 TCP 连接一个任务。流程 = Hello 认证 → Welcome;
//! 之后处理 Ping/订阅协商/RPC 分发,推送与 RPC 响应经每会话的**有界**
//! 出站通道(write 任务)下发:消费跟不上时 fan_out 丢帧计数,而不是
//! 无限堆积延迟。60s 无任何客户端帧判定死链,断开。

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

use crate::state::AppState;
use protocol::{Frame, FrameDecoder, HelloAuth};

/// 认证前等待 Hello 的时限。
const HELLO_TIMEOUT: Duration = Duration::from_secs(15);
/// 认证后读超时:期间没有任何客户端帧(Ping 周期 10s)即视为死链。
const READ_TIMEOUT: Duration = Duration::from_secs(60);
/// 出站通道深度:能吃掉秒级突发;满了由 fan_out 丢新帧(计 `dropped_frames`)。
/// 无界队列在慢客户端手里会把"丢帧"变成"分钟级延迟堆积",更糟。
const OUTBOUND_CAPACITY: usize = 2048;

pub async fn run(state: Arc<AppState>, stream: tokio::net::TcpStream) {
    let (mut reader, writer) = tokio::io::split(stream);
    let (tx, rx) = mpsc::channel::<Frame>(OUTBOUND_CAPACITY);

    // 写任务:出站帧 → 线上字节;tx 落尽即退出。
    let write_task = tokio::spawn(async move {
        let mut writer = writer;
        let mut rx = rx;
        while let Some(frame) = rx.recv().await {
            let bytes = protocol::encode_frame(&frame);
            if writer.write_all(&bytes).await.is_err() {
                break;
            }
        }
        let _ = writer.shutdown().await;
    });

    let client_id = state.new_client_id();
    let outcome = read_loop(Arc::clone(&state), client_id, &mut reader, tx.clone()).await;

    // 清理:撤销意愿 + 注销出站口,然后让写任务自然结束。
    let diff = state.hub.lock().unwrap().remove_client(client_id);
    state.apply_diff(diff);
    state.clients.lock().unwrap().remove(&client_id);
    drop(tx);
    let _ = write_task.await;

    match outcome {
        Ok(()) => tracing::info!(client_id, "client disconnected"),
        Err(reason) => tracing::warn!(client_id, reason, "client session ended"),
    }
}

/// 读循环:解码帧并逐个处理。`Ok(())` = 客户端正常断开;`Err` = 异常原因。
async fn read_loop(
    state: Arc<AppState>,
    client_id: u64,
    reader: &mut tokio::io::ReadHalf<tokio::net::TcpStream>,
    tx: mpsc::Sender<Frame>,
) -> Result<(), String> {
    let mut decoder = FrameDecoder::new();
    let mut buf = vec![0u8; 16 * 1024];
    let mut authenticated = false;

    loop {
        let timeout = if authenticated {
            READ_TIMEOUT
        } else {
            HELLO_TIMEOUT
        };
        let n = match tokio::time::timeout(timeout, reader.read(&mut buf)).await {
            Ok(Ok(0)) => return Ok(()), // 对端正常关闭
            Ok(Ok(n)) => n,
            Ok(Err(err)) => return Err(format!("read error: {err}")),
            Err(_elapsed) => {
                return Err(if authenticated {
                    "read timeout (no frames in 60s)".to_string()
                } else {
                    "hello timeout".to_string()
                });
            }
        };

        let frames = decoder
            .feed(&buf[..n])
            .map_err(|err| format!("protocol error: {err}"))?;
        for frame in frames {
            match handle_frame(Arc::clone(&state), client_id, &tx, frame, authenticated) {
                FrameOutcome::Continue => {}
                FrameOutcome::Authenticated => authenticated = true,
                FrameOutcome::Reject(reason) => return Err(reason),
            }
        }
    }
}

enum FrameOutcome {
    Continue,
    /// Hello 通过:进入已认证状态。
    Authenticated,
    /// 协议违规,断开。
    Reject(String),
}

fn handle_frame(
    state: Arc<AppState>,
    client_id: u64,
    tx: &mpsc::Sender<Frame>,
    frame: Frame,
    authenticated: bool,
) -> FrameOutcome {
    match frame {
        Frame::Hello { auth, app_version } => {
            if authenticated {
                return FrameOutcome::Reject("duplicate hello".into());
            }
            // 认证:TOTP 动态码(首连)或会话令牌(断线重连,免输码)。
            // 两种凭据成功后都轮换下发新令牌,客户端在内存里持有它用于
            // 后续重连;被轮换/过期的旧令牌立即作废。
            let session_token = match &auth {
                HelloAuth::Totp(code) => {
                    if !verify_auth_code(&state.config, code) {
                        tracing::warn!(client_id, "hello rejected: bad auth code");
                        // 明确告知拒绝原因再断开:客户端要区分"码无效/过期"
                        //(终态,等人工输新码)与"网络断了"(瞬态,自动重连)。
                        let _ = tx.try_send(Frame::AuthFail {
                            reason: "invalid auth code".into(),
                        });
                        return FrameOutcome::Reject("bad auth code".into());
                    }
                    state.new_session_token()
                }
                HelloAuth::Session(token) => {
                    let Some(fresh) = state.resume_session(token) else {
                        tracing::warn!(client_id, "hello rejected: session token unknown/expired");
                        let _ = tx.try_send(Frame::AuthFail {
                            reason: "session expired; re-authenticate".into(),
                        });
                        return FrameOutcome::Reject("session expired".into());
                    };
                    fresh
                }
            };
            tracing::info!(client_id, app_version, "client authenticated");
            // 注册进扇出表:推送泵(上游行情/新闻)只发给这张表里的会话。
            // RPC/Welcome 走会话自己的 tx,不依赖这里。
            state.clients.lock().unwrap().insert(client_id, tx.clone());
            let _ = tx.try_send(Frame::Welcome {
                server_time_ms: chrono::Utc::now().timestamp_millis(),
                session_token,
                version: env!("CARGO_PKG_VERSION").to_string(),
            });
            FrameOutcome::Authenticated
        }

        _ if !authenticated => FrameOutcome::Reject("first frame must be hello".into()),

        Frame::Ping => {
            let _ = tx.try_send(Frame::Pong);
            FrameOutcome::Continue
        }

        Frame::Subscribe {
            id,
            symbols,
            channels,
        } => {
            let diff = state
                .hub
                .lock()
                .unwrap()
                .apply_subscribe(client_id, &symbols, channels);
            state.apply_diff(diff);
            let _ = tx.try_send(Frame::Subscribed { id });
            FrameOutcome::Continue
        }

        Frame::Unsubscribe {
            symbols, channels, ..
        } => {
            let diff = state
                .hub
                .lock()
                .unwrap()
                .apply_unsubscribe(client_id, &symbols, channels);
            state.apply_diff(diff);
            FrameOutcome::Continue
        }

        Frame::RpcRequest { id, payload } => {
            let task_state = Arc::clone(&state);
            let task_tx = tx.clone();
            tokio::spawn(async move {
                let started = std::time::Instant::now();
                let result = crate::rpc::handle(&task_state, id, &task_tx, payload).await;
                if let Err(err) = &result {
                    tracing::warn!(rpc_id = id, error = err, "rpc failed");
                }
                tracing::debug!(
                    rpc_id = id,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "rpc done"
                );
                let _ = task_tx.try_send(Frame::RpcResponse { id, result });
            });
            FrameOutcome::Continue
        }

        // 服务端不会收到这些帧;出现即视为协议 misuse,断开。
        _ => FrameOutcome::Reject(format!("unexpected frame from client: {frame:?}")),
    }
}

/// 校验 TOTP 动态码:±1 窗口(±30s)容忍时钟偏差与输入耗时;密钥无效
/// (配置错误)一律拒绝。`dev_skip_totp = true` 时任意码放行(仅本地
/// 开发用,启动与每次放行都打 warning)。
fn verify_auth_code(config: &crate::config::Config, code: &str) -> bool {
    if config.dev_skip_totp {
        tracing::warn!("dev_skip_totp is enabled: accepting auth code without verification");
        return true;
    }
    let Ok(secret) = roka_totp::Secret::from_base32(&config.totp_secret) else {
        // 密钥本身不是合法 Base32(如混入 0/1/8/9 或小写未清洗):静默 false
        // 会让"所有连接被拒"无从排查,必须打到日志里。
        tracing::error!("totp_secret is not valid base32; every client will be rejected");
        return false;
    };
    let totp = roka_totp::Totp::builder(secret).build();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    totp.verify(code.trim(), now, 1).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicU64;

    use crate::config::Config;
    use crate::executor::{AlpacaClient, FinvizClient, ReqwestExecutor};
    use crate::hub::SubscriptionHub;
    use protocol::{Channels, FrameDecoder, HelloAuth};

    /// 测试用 Base32 密钥(RFC 示例,解码为 "Hello!")。
    const TEST_SECRET: &str = "JBSWY3DPEHPK3PXP";

    fn test_totp() -> roka_totp::Totp {
        roka_totp::Totp::builder(roka_totp::Secret::from_base32(TEST_SECRET).unwrap()).build()
    }

    /// 本机回环:认证失败回 AuthFail 后断;正确动态码走 Hello→Welcome→
    /// Ping/Pong→Subscribe/Subscribed 全链路(不依赖真实上游)。
    async fn test_state(secret: &str) -> Arc<AppState> {
        test_state_with(secret, false).await
    }

    async fn test_state_with(secret: &str, dev_skip_totp: bool) -> Arc<AppState> {
        let config: Config = toml::from_str(&format!(
            "listen = '127.0.0.1:0'\ntotp_secret = '{secret}'\ndev_skip_totp = {dev_skip_totp}\n[alpaca]\nkey_id = 'k'\nsecret = 's'\nfeed = 'iex'\n[finviz]\ntoken = 'f'\n[deepseek]\napi_key = 'd'"
        ))
        .unwrap();
        let http = reqwest::Client::new();
        let (upstream_tx, _upstream_rx) = mpsc::unbounded_channel();
        std::mem::forget(_upstream_rx); // 接收端保活,命令进队列即可(无人消费)
        Arc::new(AppState {
            config,
            http: http.clone(),
            alpaca: AlpacaClient::new(http.clone(), "k", "s", ReqwestExecutor),
            finviz: FinvizClient::new(http, "f", ReqwestExecutor),
            upstream_tx,
            hub: Mutex::new(SubscriptionHub::new()),
            clients: Mutex::new(HashMap::new()),
            dropped_frames: AtomicU64::new(0),
            sessions: Mutex::new(HashMap::new()),
            next_client_id: AtomicU64::new(1),
        })
    }

    /// 回归:`dev_skip_totp = true`(本地开发开关)时,任意动态码都放行
    /// 且走完整认证流程(注册扇出表、下发会话令牌)。
    #[tokio::test]
    async fn dev_skip_totp_accepts_any_code() {
        let state = test_state_with(TEST_SECRET, true).await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let observer = Arc::clone(&state);

        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            crate::session::run(state, stream).await;
        });

        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let mut decoder = FrameDecoder::new();
        let mut buf = [0u8; 4096];

        let bytes = protocol::encode_frame(&Frame::Hello {
            auth: HelloAuth::Totp("000000".into()),
            app_version: "test".into(),
        });
        stream.write_all(&bytes).await.unwrap();
        let n = stream.read(&mut buf).await.unwrap();
        let frames = decoder.feed(&buf[..n]).unwrap();
        let welcome_token = frames
            .iter()
            .find_map(|f| match f {
                Frame::Welcome { session_token, .. } => Some(session_token),
                _ => None,
            })
            .expect("dev_skip_totp must accept any code and issue a welcome");
        assert!(!welcome_token.is_empty());
        assert!(
            !observer.clients.lock().unwrap().is_empty(),
            "authenticated session must be registered for push fan-out"
        );
        assert!(frames.iter().all(|f| !matches!(f, Frame::AuthFail { .. })));
    }

    #[tokio::test]
    async fn loopback_auth_heartbeat_and_subscribe() {
        let state = test_state(TEST_SECRET).await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let observer = Arc::clone(&state);

        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            crate::session::run(state, stream).await;
        });

        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let mut decoder = FrameDecoder::new();
        let mut buf = [0u8; 4096];

        // Hello(当前动态码)→ Welcome(携带会话令牌)。
        let bytes = protocol::encode_frame(&Frame::Hello {
            auth: HelloAuth::Totp(test_totp().code_now()),
            app_version: "test".into(),
        });
        stream.write_all(&bytes).await.unwrap();
        let n = stream.read(&mut buf).await.unwrap();
        let welcome_token = decoder
            .feed(&buf[..n])
            .unwrap()
            .into_iter()
            .find_map(|f| match f {
                Frame::Welcome {
                    session_token,
                    server_time_ms,
                    ..
                } if server_time_ms > 0 => Some(session_token),
                _ => None,
            })
            .expect("expected welcome with a positive server time and a session token");
        assert!(!welcome_token.is_empty(), "welcome carries a session token");

        // 回归:认证成功后,会话必须已注册进推送扇出表——否则上游泵的
        // fan_out 找不到任何收件人,行情/新闻推送静默丢失。
        assert!(
            !observer.clients.lock().unwrap().is_empty(),
            "authenticated session must be registered for push fan-out"
        );

        // Ping → Pong。
        stream
            .write_all(&protocol::encode_frame(&Frame::Ping))
            .await
            .unwrap();
        let n = stream.read(&mut buf).await.unwrap();
        assert!(matches!(
            decoder.feed(&buf[..n]).unwrap().as_slice(),
            [Frame::Pong]
        ));

        // Subscribe → Subscribed{id}(hub 聚合不 panic;上游差量进 dummy 队列)。
        stream
            .write_all(&protocol::encode_frame(&Frame::Subscribe {
                id: 9,
                symbols: vec!["SPY".into()],
                channels: Channels::TRADES | Channels::DAILY,
            }))
            .await
            .unwrap();
        let n = stream.read(&mut buf).await.unwrap();
        assert!(matches!(
            decoder.feed(&buf[..n]).unwrap().as_slice(),
            [Frame::Subscribed { id: 9 }]
        ));
    }

    /// 错误动态码:服务端回 `AuthFail`(客户端据此区分"码失效"与"网络断")
    /// 然后断开。
    #[tokio::test]
    async fn bad_code_gets_auth_fail_then_disconnect() {
        let state = test_state(TEST_SECRET).await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            crate::session::run(state, stream).await;
        });

        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let bytes = protocol::encode_frame(&Frame::Hello {
            auth: HelloAuth::Totp("wrong".into()),
            app_version: "test".into(),
        });
        stream.write_all(&bytes).await.unwrap();
        // 先收到 AuthFail,再断开读到 0。
        let mut decoder = FrameDecoder::new();
        let mut buf = [0u8; 64];
        let n = tokio::time::timeout(std::time::Duration::from_secs(5), stream.read(&mut buf))
            .await
            .unwrap()
            .unwrap();
        assert!(
            decoder
                .feed(&buf[..n])
                .unwrap()
                .iter()
                .any(|f| matches!(f, Frame::AuthFail { .. })),
            "server must send AuthFail before closing"
        );
        let n = tokio::time::timeout(std::time::Duration::from_secs(5), stream.read(&mut buf))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(n, 0, "server must close after AuthFail");
    }

    /// 过期动态码(90 秒前的码,超出 ±1 窗口)同样被拒。
    #[tokio::test]
    async fn expired_code_gets_rejected() {
        let state = test_state(TEST_SECRET).await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            crate::session::run(state, stream).await;
        });

        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let stale = test_totp().code_at(now - 90);
        let bytes = protocol::encode_frame(&Frame::Hello {
            auth: HelloAuth::Totp(stale),
            app_version: "test".into(),
        });
        stream.write_all(&bytes).await.unwrap();
        let mut buf = [0u8; 64];
        let n = tokio::time::timeout(std::time::Duration::from_secs(5), stream.read(&mut buf))
            .await
            .unwrap()
            .unwrap();
        assert!(
            decoder_frames(&buf[..n])
                .iter()
                .any(|f| matches!(f, Frame::AuthFail { .. })),
            "stale code must be rejected with AuthFail"
        );
    }

    /// 小助手:喂给解码器并返回帧列表(过期码测试里解码器在函数内新建)。
    fn decoder_frames(bytes: &[u8]) -> Vec<Frame> {
        let mut decoder = FrameDecoder::new();
        decoder.feed(bytes).unwrap()
    }

    /// 会话令牌重连全流程:首连(TOTP)下发令牌 → 令牌重连免输码且**轮换**
    /// 新令牌 → 被轮换的旧令牌立即作废(AuthFail)。
    #[tokio::test]
    async fn session_token_resume_rotates_and_rejects_stale() {
        let state = test_state(TEST_SECRET).await;
        let listener =
            std::sync::Arc::new(tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap());
        let addr = listener.local_addr().unwrap();
        let run_session_on = {
            let listener = Arc::clone(&listener);
            move |state: Arc<AppState>| {
                let listener = Arc::clone(&listener);
                tokio::spawn(async move {
                    let (stream, _) = listener.accept().await.unwrap();
                    crate::session::run(state, stream).await;
                })
            }
        };

        // 1) 首连:TOTP 动态码 → 下发令牌。
        run_session_on(Arc::clone(&state));
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        stream
            .write_all(&protocol::encode_frame(&Frame::Hello {
                auth: HelloAuth::Totp(test_totp().code_now()),
                app_version: "test".into(),
            }))
            .await
            .unwrap();
        let mut decoder = FrameDecoder::new();
        let mut buf = [0u8; 4096];
        let first_token = decoder
            .feed({
                let n = stream.read(&mut buf).await.unwrap();
                &buf[..n]
            })
            .unwrap()
            .into_iter()
            .find_map(|f| match f {
                Frame::Welcome { session_token, .. } => Some(session_token),
                _ => None,
            })
            .expect("welcome on totp auth");
        let _ = stream.shutdown().await;

        // 2) 令牌重连:免输动态码,拿到轮换后的新令牌。
        run_session_on(Arc::clone(&state));
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        stream
            .write_all(&protocol::encode_frame(&Frame::Hello {
                auth: HelloAuth::Session(first_token.clone()),
                app_version: "test".into(),
            }))
            .await
            .unwrap();
        let second_token = decoder
            .feed({
                let n = stream.read(&mut buf).await.unwrap();
                &buf[..n]
            })
            .unwrap()
            .into_iter()
            .find_map(|f| match f {
                Frame::Welcome { session_token, .. } => Some(session_token),
                _ => None,
            })
            .expect("welcome on session token resume");
        assert_ne!(first_token, second_token, "token must rotate on resume");
        let _ = stream.shutdown().await;

        // 3) 被轮换的旧令牌:立即作废。
        run_session_on(Arc::clone(&state));
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        stream
            .write_all(&protocol::encode_frame(&Frame::Hello {
                auth: HelloAuth::Session(first_token),
                app_version: "test".into(),
            }))
            .await
            .unwrap();
        let n = tokio::time::timeout(std::time::Duration::from_secs(5), stream.read(&mut buf))
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            decoder.feed(&buf[..n]).unwrap().as_slice(),
            [Frame::AuthFail { .. }]
        ));
    }
}
