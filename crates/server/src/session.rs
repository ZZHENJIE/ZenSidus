//! 客户端会话:每条 TCP 连接一个任务。流程 = Hello 认证 → Welcome;
//! 之后处理 Ping/订阅协商/RPC 分发,推送与 RPC 响应经每会话的 unbounded
//! 出站通道(write 任务)下发。60s 无任何客户端帧判定死链,断开。

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

use crate::state::AppState;
use protocol::{Frame, FrameDecoder};

/// 认证前等待 Hello 的时限。
const HELLO_TIMEOUT: Duration = Duration::from_secs(15);
/// 认证后读超时:期间没有任何客户端帧(Ping 周期 10s)即视为死链。
const READ_TIMEOUT: Duration = Duration::from_secs(60);

pub async fn run(state: Arc<AppState>, stream: tokio::net::TcpStream) {
    let (mut reader, writer) = tokio::io::split(stream);
    let (tx, rx) = mpsc::unbounded_channel::<Frame>();

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
    tx: mpsc::UnboundedSender<Frame>,
) -> Result<(), String> {
    let mut decoder = FrameDecoder::new();
    let mut buf = vec![0u8; 16 * 1024];
    let mut authenticated = false;

    loop {
        let timeout = if authenticated { READ_TIMEOUT } else { HELLO_TIMEOUT };
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
    tx: &mpsc::UnboundedSender<Frame>,
    frame: Frame,
    authenticated: bool,
) -> FrameOutcome {
    match frame {
        Frame::Hello { token, app_version } => {
            if authenticated {
                return FrameOutcome::Reject("duplicate hello".into());
            }
            if token != state.config.token {
                tracing::warn!(client_id, "hello rejected: bad token");
                return FrameOutcome::Reject("bad token".into());
            }
            tracing::info!(client_id, app_version, "client authenticated");
            // 注册进扇出表:推送泵(上游行情/新闻)只发给这张表里的会话。
            // RPC/Welcome 走会话自己的 tx,不依赖这里。
            state.clients.lock().unwrap().insert(client_id, tx.clone());
            let _ = tx.send(Frame::Welcome {
                server_time_ms: chrono::Utc::now().timestamp_millis(),
            });
            FrameOutcome::Authenticated
        }

        _ if !authenticated => {
            FrameOutcome::Reject("first frame must be hello".into())
        }

        Frame::Ping => {
            let _ = tx.send(Frame::Pong);
            FrameOutcome::Continue
        }

        Frame::Subscribe { id, symbols, channels } => {
            let diff = state.hub.lock().unwrap().apply_subscribe(client_id, &symbols, channels);
            state.apply_diff(diff);
            let _ = tx.send(Frame::Subscribed { id });
            FrameOutcome::Continue
        }

        Frame::Unsubscribe { symbols, channels, .. } => {
            let diff = state.hub.lock().unwrap().apply_unsubscribe(client_id, &symbols, channels);
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
                tracing::debug!(rpc_id = id, elapsed_ms = started.elapsed().as_millis() as u64, "rpc done");
                let _ = task_tx.send(Frame::RpcResponse { id, result });
            });
            FrameOutcome::Continue
        }

        // 服务端不会收到这些帧;出现即视为协议 misuse,断开。
        _ => FrameOutcome::Reject(format!("unexpected frame from client: {frame:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;
    use std::sync::Mutex;
    use std::collections::HashMap;
    use std::sync::atomic::AtomicU64;

    use crate::config::Config;
    use crate::executor::{AlpacaClient, FinvizClient, ReqwestExecutor};
    use crate::hub::SubscriptionHub;
    use protocol::{Channels, FrameDecoder};

    /// 本机回环:认证失败即断;正确 token 走 Hello→Welcome→Ping/Pong→
    /// Subscribe/Subscribed 全链路(不依赖真实上游)。
    async fn test_state(token: &str) -> Arc<AppState> {
        let config: Config = toml::from_str(&format!(
            "listen = '127.0.0.1:0'\ntoken = '{token}'\n[alpaca]\nkey_id = 'k'\nsecret = 's'\nfeed = 'iex'\n[finviz]\ntoken = 'f'\n[deepseek]\napi_key = 'd'"
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
            next_client_id: AtomicU64::new(1),
        })
    }

    #[tokio::test]
    async fn loopback_auth_heartbeat_and_subscribe() {
        let state = test_state("secret-token").await;
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

        // Hello(正确 token)→ Welcome。
        let bytes = protocol::encode_frame(&Frame::Hello {
            token: "secret-token".into(),
            app_version: "test".into(),
        });
        stream.write_all(&bytes).await.unwrap();
        let n = stream.read(&mut buf).await.unwrap();
        let frames = decoder.feed(&buf[..n]).unwrap();
        assert!(
            frames
                .iter()
                .any(|f| matches!(f, Frame::Welcome { server_time_ms } if *server_time_ms > 0)),
            "expected welcome with a positive server time"
        );

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

    #[tokio::test]
    async fn bad_token_gets_disconnected() {
        let state = test_state("secret-token").await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            crate::session::run(state, stream).await;
        });

        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let bytes = protocol::encode_frame(&Frame::Hello {
            token: "wrong".into(),
            app_version: "test".into(),
        });
        stream.write_all(&bytes).await.unwrap();
        // 服务端拒绝(不回任何帧即断开):读到 0。
        let mut buf = [0u8; 64];
        let n = tokio::time::timeout(std::time::Duration::from_secs(5), stream.read(&mut buf))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(n, 0, "server must close without replying to a bad token");
    }
}
