//! 自有网关功能的服务端实现(自 Gateway worker 移植,原 TS 逻辑逐行对照):
//! 标的搜索(futunn 代理)、翻译(DeepSeek 代理)、IPO 日历(iposcoop 抓取)。

use anyhow::Context;
use serde_json::Value;

use relay_protocol::{IpoItem, SearchItem};

const FUTUNN_SEARCH_URL: &str = "https://www.futunn.com/search-stock/predict";
const DEEPSEEK_URL: &str = "https://api.deepseek.com/chat/completions";
const IPO_URL: &str = "https://www.iposcoop.com/ipo-calendar/";

/// 标的搜索:futunn search-suggest 代理,只保留美股(`marketType == 2`)。
pub async fn search(http: &reqwest::Client, keyword: &str, lang: &str) -> anyhow::Result<Vec<SearchItem>> {
    let url = format!(
        "{FUTUNN_SEARCH_URL}?keyword={}&lang={}",
        urlencoding::encode(keyword),
        urlencoding::encode(lang),
    );
    let response = http
        .get(&url)
        .send()
        .await
        .context("futunn search request failed")?;
    let status = response.status();
    let body = response.text().await.context("futunn search body read failed")?;
    if !status.is_success() {
        anyhow::bail!("Search API error {status}: {}", body.trim());
    }
    let data: Value =
        serde_json::from_str(&body).with_context(|| format!("futunn search not JSON: {}", short(&body)))?;
    Ok(parse_search(&data))
}

/// 从 futunn 响应里筛出美股建议(TS 版逻辑:`data.stock`,`marketType == 2`,
/// 股票码只含 `A-Z` 与 `.`)。
fn parse_search(data: &Value) -> Vec<SearchItem> {
    let mut result = Vec::new();
    if let Some(stock) = data.get("data").and_then(|data| data.get("stock")).and_then(|v| v.as_array()) {
        for item in stock {
            let market_type = item.get("marketType").and_then(|v| v.as_i64());
            let name = item.get("stockName").and_then(|v| v.as_str()).unwrap_or_default();
            let symbol = item.get("stockSymbol").and_then(|v| v.as_str()).unwrap_or_default();
            if market_type == Some(2) && is_valid_stock_symbol(symbol) {
                result.push(SearchItem {
                    name: name.to_string(),
                    symbol: symbol.to_string(),
                });
            }
        }
    }
    result
}

fn is_valid_stock_symbol(symbol: &str) -> bool {
    !symbol.is_empty() && symbol.chars().all(|c| c.is_ascii_uppercase() || c == '.')
}

/// 翻译:DeepSeek chat-completions 代理(与 Gateway worker 同参:思考开启、
/// 高推理力度),返回正文原文。
pub async fn translate(
    http: &reqwest::Client,
    api_key: &str,
    from: &str,
    to: &str,
    keyword: &str,
) -> anyhow::Result<String> {
    let body = serde_json::json!({
        "model": "deepseek-flash",
        "messages": [
            {
                "role": "user",
                "content": format!("Please translate the phrase \"{keyword}\" from \"{from}\" to \"{to}\". Return only the translated text, without any explanation."),
            },
        ],
        "thinking": { "type": "enabled" },
        "reasoning_effort": "high",
        "stream": false,
    });
    let response = http
        .post(DEEPSEEK_URL)
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .context("deepseek request failed")?;
    let status = response.status();
    let body = response.text().await.context("deepseek body read failed")?;
    if !status.is_success() {
        anyhow::bail!("DeepSeek API error {status}: {}", body.trim());
    }
    let data: Value = serde_json::from_str(&body).context("deepseek response not JSON")?;
    let content = data
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(|content| content.as_str())
        .unwrap_or_default();
    if content.is_empty() {
        anyhow::bail!("DeepSeek returned empty content");
    }
    Ok(content.trim().to_string())
}

/// IPO 日历:抓取 iposcoop.com 的日历页并解析主表。
pub async fn ipo(http: &reqwest::Client) -> anyhow::Result<Vec<IpoItem>> {
    let response = http.get(IPO_URL).send().await.context("iposcoop request failed")?;
    let status = response.status();
    let html = response.text().await.context("iposcoop body read failed")?;
    if !status.is_success() {
        anyhow::bail!("iposcoop error {status}");
    }
    Ok(parse_ipo(&html))
}

/// 解析 iposcoop 日历表(`#main-content table tbody tr`,≥9 列;TS 版取
/// 0/1/2/3/4/5/7 列,数值列解析失败按 0)。
fn parse_ipo(html: &str) -> Vec<IpoItem> {
    let document = scraper::Html::parse_document(html);
    let rows_selector = scraper::Selector::parse("#main-content table tbody tr")
        .expect("static selector");
    let cell_selector = scraper::Selector::parse("td").expect("static selector");
    let link_selector = scraper::Selector::parse("a").expect("static selector");

    let mut result = Vec::new();
    for row in document.select(&rows_selector) {
        let cells: Vec<_> = row.select(&cell_selector).collect();
        if cells.len() < 9 {
            continue;
        }
        let text = |index: usize| cells[index].text().collect::<String>().trim().to_string();
        let company_des_url = cells[0]
            .select(&link_selector)
            .next()
            .and_then(|a| a.value().attr("href"))
            .unwrap_or_default()
            .to_string();
        let number = |index: usize| text(index).parse::<f64>().unwrap_or(0.0);
        result.push(IpoItem {
            company: text(0),
            company_des_url,
            symbol: text(1),
            managers: text(2),
            shares_millions: number(3),
            price_low: number(4),
            price_high: number(5),
            expected_date: text(7),
        });
    }
    result
}

fn short(body: &str) -> String {
    let mut end = 200.min(body.len());
    while !body.is_char_boundary(end) {
        end -= 1;
    }
    body[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 只保留 marketType==2 且代码合法的建议(TS 版行为)。
    #[test]
    fn search_filters_us_equities() {
        let data: Value = serde_json::json!({
            "data": {
                "stock": [
                    {"stockName": "Apple Inc.", "stockSymbol": "AAPL", "marketType": 2},
                    {"stockName": "苹果A1507", "stockSymbol": "AAPL1507", "marketType": 3},
                    {"stockName": "Brighthouse", "stockSymbol": "BHF", "marketType": 2},
                    {"stockName": "Bad Chars", "stockSymbol": "aapl", "marketType": 2},
                    {"stockName": "Missing", "stockSymbol": "XYZ"}
                ]
            }
        });
        let items = parse_search(&data);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].symbol, "AAPL");
        assert_eq!(items[0].name, "Apple Inc.");
        assert_eq!(items[1].symbol, "BHF");
    }

    /// IPO 表解析:第 6 列被跳过(TS 版行为),数值列容错为 0。
    #[test]
    fn ipo_parses_table_rows() {
        let html = r#"<html><body><div id="main-content"><table>
            <tbody>
            <tr><td>Filler</td><td>F</td><td>f</td><td>f</td><td>f</td><td>f</td><td>f</td><td>f</td><td>f</td><td>f</td></tr>
            <tr>
                <td><a href="https://www.iposcoop.com/company/x">Acme Corp</a></td>
                <td>ACME</td>
                <td>Goldman</td>
                <td>12.5</td>
                <td>10</td>
                <td>12</td>
                <td>NASDAQ</td>
                <td>2026-12-01</td>
                <td>extra</td>
                <td>extra</td>
            </tr>
            <tr><td>short</td></tr>
            </tbody></table></div></body></html>"#;
        let items = parse_ipo(html);
        assert_eq!(items.len(), 2);

        let acme = &items[1];
        assert_eq!(acme.company, "Acme Corp");
        assert_eq!(acme.company_des_url, "https://www.iposcoop.com/company/x");
        assert_eq!(acme.symbol, "ACME");
        assert_eq!(acme.managers, "Goldman");
        assert_eq!(acme.shares_millions, 12.5);
        assert_eq!(acme.price_low, 10.0);
        assert_eq!(acme.price_high, 12.0);
        assert_eq!(acme.expected_date, "2026-12-01");

        // 数值列不是数字时按 0(TS 版 `|| 0`)。
        let filler = &items[0];
        assert_eq!(filler.shares_millions, 0.0);
        assert_eq!(filler.expected_date, "f");
    }

    #[test]
    fn short_truncates_at_char_boundary() {
        assert_eq!(short("hello"), "hello");
        assert!(short(&"中文内容很长很长很长".repeat(100)).chars().count() <= 200);
    }
}
