//! server 入口:加载配置 → 连上游(anysdk 股票/新闻流,重试直到成功)
//! → 起常驻任务 → TCP listener 循环。

mod config;
mod executor;
mod finviz_data;
mod hub;
mod mapping;
mod rpc;
mod session;
mod state;
mod web;

use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::mpsc;

use config::Config;
use executor::{AlpacaClient, FinvizClient, ReqwestExecutor};
use state::AppState;

fn main() -> anyhow::Result<()> {
    let config_path = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("SERVER_CONFIG").ok())
        .unwrap_or_else(|| "server.toml".to_string());
    let config = Config::load(&config_path)?;

    let filter = tracing_subscriber::EnvFilter::try_new(&config.log_level)
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(serve(config))
}

async fn serve(config: Config) -> anyhow::Result<()> {
    let http = reqwest::Client::builder()
        .user_agent(concat!("server/", env!("CARGO_PKG_VERSION")))
        .build()?;

    let alpaca = AlpacaClient::new(
        http.clone(),
        config.alpaca.key_id.clone(),
        config.alpaca.secret.clone(),
        ReqwestExecutor,
    );
    let finviz = FinvizClient::new(http.clone(), &config.finviz.token, ReqwestExecutor);

    let ws_proxy = config.proxy.as_ref().and_then(|proxy| proxy.ws_socks5.clone());
    let stock = connect_with_retry("stock stream", || {
        anysdk::alpaca_stream::StockStream::connect_with_url_and_proxy(
            anysdk::alpaca_stream::STOCK_STREAM_URL,
            config.alpaca.feed,
            config.alpaca.key_id.clone(),
            config.alpaca.secret.clone(),
            ws_proxy.clone(),
        )
    })
    .await;
    let news = connect_with_retry("news stream", || {
        anysdk::alpaca_stream::NewsStream::connect_to_with_proxy(
            anysdk::alpaca_stream::NEWS_STREAM_URL,
            config.alpaca.key_id.clone(),
            config.alpaca.secret.clone(),
            ws_proxy.clone(),
        )
    })
    .await;

    let (upstream_tx, upstream_rx) = mpsc::unbounded_channel();
    let app = Arc::new(AppState {
        config,
        http,
        alpaca,
        finviz,
        upstream_tx,
        hub: Mutex::new(hub::SubscriptionHub::new()),
        clients: Mutex::new(HashMap::new()),
        next_client_id: AtomicU64::new(1),
    });
    state::spawn(Arc::clone(&app), stock, news, upstream_rx);

    let listener = tokio::net::TcpListener::bind(&app.config.listen).await?;
    tracing::info!(listen = %app.config.listen, "server listening");

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, peer)) => {
                        tracing::debug!(%peer, "connection accepted");
                        let task_app = Arc::clone(&app);
                        tokio::spawn(session::run(task_app, stream));
                    }
                    Err(err) => tracing::warn!(error = %err, "accept failed"),
                }
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("ctrl-c; shutting down");
                return Ok(());
            }
        }
    }
}

/// 上游首连:失败即重试(指数不退避,固定 5s;Alpaca 侧故障不应拖垮服务)。
/// 成功后断线重连归 anysdk supervisor。
async fn connect_with_retry<T, F, Fut>(what: &str, connect: F) -> T
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<T>>,
{
    loop {
        match connect().await {
            Ok(handle) => {
                tracing::info!(what, "upstream connected");
                return handle;
            }
            Err(err) => {
                tracing::error!(what, error = %err, "upstream connect failed; retry in 5s");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}
