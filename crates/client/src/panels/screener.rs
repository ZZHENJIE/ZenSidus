//! The screener-family dock panels: the Finviz stock screener and the
//! market-screener placeholder.

pub mod market;
pub mod stock;

pub use market::MarketScreenerPanel;
pub use stock::StockScreenerPanel;

use protocol::ScreenerParameter;

/// settings 里保存的 finviz 参数 → 协议 DTO(`screener_type` 取线值,
/// 如 `stocks`/`etf`)。token 已下沉服务端,这里只搬参数本身。
pub(crate) fn screener_parameter_dto(
    parameter: &anysdk::finviz_sdk::screener::Parameter,
) -> ScreenerParameter {
    ScreenerParameter {
        screener_type: parameter.screener_type.as_str().to_string(),
        order_by: parameter.order_by.clone(),
        signal: parameter.signal.clone(),
        filters: parameter.filters.clone(),
        columns: parameter.columns.clone(),
    }
}
