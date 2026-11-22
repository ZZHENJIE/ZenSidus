//! 筛选器参数 DTO:镜像 AnySDK `finviz_sdk::screener::Parameter` 的字段
//! (postcard 全 owned,协议 crate 不依赖 finviz_sdk)。client 把 settings 里
//! 的 `Parameter` 转过来,server 再转回 finviz 类型发往 Finviz Elite。

use serde::{Deserialize, Serialize};

/// 一个筛选导出单元格:显示与排序语义分离。
///
/// `text` 是导出原文(渲染保持 Finviz 的格式与精度);`value` 是从中
/// 解析出的数值——K/M/B 后缀折算成真值、`%` 后缀取百分号前的数
/// (`+5.04%` → 5.04)、`$`/逗号剥除、`-`/空串/不可解析(如
/// `May 15 - May 19` 日期区间)→ `None`。排序/筛选用 value,渲染用
/// text;server 侧定型(`crates/server/src/finviz_data.rs`)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cell {
    pub text: String,
    pub value: Option<f64>,
}

impl Cell {
    /// 原文单元格(无数值语义)。
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            value: None,
        }
    }

    /// 带数值的单元格(文本仍保留原文)。
    pub fn valued(text: impl Into<String>, value: f64) -> Self {
        Self {
            text: text.into(),
            value: Some(value),
        }
    }
}

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

/// 筛选导出行的往返:`Cell` 携带原文 + 数值,postcard 可逆。
#[cfg(test)]
mod cell_tests {
    use super::*;

    #[test]
    fn cell_roundtrips() {
        let cells = vec![
            Cell::valued("2.589B", 2.589e9),
            Cell::valued("+5.04%", 5.04),
            Cell::text("May 15 - May 19"),
        ];
        let bytes = postcard::to_allocvec(&cells).unwrap();
        let back: Vec<Cell> = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(back, cells);
    }
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
