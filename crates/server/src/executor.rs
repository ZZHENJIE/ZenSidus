//! HTTP 执行器:reqwest 驱动 AnySDK 的全部端点(服务端唯一 reqwest 落点,
//! 模式与 AnySDK `crates/test/src/main.rs` 的示例一致)。

/// 传给 [`anysdk::utils::HttpExecutor`] 的 reqwest 客户端就是 `T` 本身。
#[derive(Debug, Clone, Copy)]
pub struct ReqwestExecutor;

impl anysdk::utils::HttpExecutor<reqwest::Client> for ReqwestExecutor {
    async fn execute(
        &self,
        client: &reqwest::Client,
        request: &anysdk::utils::Method,
    ) -> anyhow::Result<anysdk::utils::HttpResponse> {
        let response = match request {
            anysdk::utils::Method::Get(request) => {
                let mut builder = client.get(&request.url);
                for (key, value) in &request.headers {
                    builder = builder.header(key.as_str(), value.as_str());
                }
                builder.send().await?
            }
            anysdk::utils::Method::Post(request) => {
                let mut builder = client.post(&request.url).body(request.body.clone());
                for (key, value) in &request.headers {
                    builder = builder.header(key.as_str(), value.as_str());
                }
                builder.send().await?
            }
        };
        Ok(anysdk::utils::HttpResponse {
            status: response.status().as_u16(),
            body: response.bytes().await?.to_vec(),
        })
    }
}

/// Alpaca 市场数据 REST 客户端别名。
pub type AlpacaClient = anysdk::alpaca_sdk::Client<reqwest::Client, ReqwestExecutor>;

/// Finviz Elite 导出客户端别名。
pub type FinvizClient = anysdk::finviz_sdk::Client<reqwest::Client, ReqwestExecutor>;
