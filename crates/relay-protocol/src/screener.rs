//! 筛选器参数 DTO:镜像 AnySDK `finviz_sdk::screener::Parameter` 的字段
//! (postcard 全 owned,协议 crate 不依赖 finviz_sdk)。client 把 settings 里
//! 的 `Parameter` 转过来,server 再转回 finviz 类型发往 Finviz Elite。

use serde::{Deserialize, Serialize};

/// Finviz Elite 筛选导出参数。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenerParameter {
    /// `stocks` / `etf`([`finviz_sdk::screener::ScreenerType::as_str`] 的值)。
    pub screener_type: String,
    pub order_by: String,
    pub signal: Option<String>,
    /// Finviz `f=` 原串(逗号分隔的多组筛选)。
    pub filters: Option<String>,
    /// Finviz `c=` 原串(导出列序号)。
    pub columns: Option<String>,
}

impl Default for ScreenerParameter {
    fn default() -> Self {
        Self {
            screener_type: "stocks".to_string(),
            order_by: "-volume".to_string(),
            signal: None,
            filters: None,
            columns: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips() {
        let p = ScreenerParameter {
            screener_type: "etf".into(),
            order_by: "-marketcap".into(),
            signal: Some("top_gainers".into()),
            filters: Some("fa_highpaydiv,geo_usa".into()),
            columns: Some("0,1,65,66".into()),
        };
        let bytes = postcard::to_allocvec(&p).unwrap();
        let back: ScreenerParameter = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(back, p);
    }
}
