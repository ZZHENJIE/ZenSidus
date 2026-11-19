//! Finviz Elite 日历数据加工:SDK 保留导出单元格原文(无时区换算、无
//! "All Day" 归一),本模块把 US-Eastern 墙钟解析为毫秒时间戳并产出协议
//! DTO。逻辑与测试自原 `gateway_sdk::finviz` 移植(该 crate 随 relay 化退役)。

use anyhow::Context;
use chrono::{Datelike, Duration, NaiveDate, NaiveTime, TimeZone};
use chrono_tz::America::New_York;

use protocol::{EconomicItem, EarningsItem};

/// Convert a fetched economic export: every row's date/time cell becomes a
/// millisecond timestamp; one unparsable cell fails the whole batch (the
/// RPC error path carries the message).
pub fn economic_rows(items: Vec<economic::Item>) -> anyhow::Result<Vec<EconomicItem>> {
    items
        .into_iter()
        .map(|item| {
            Ok(EconomicItem {
                time_ms: build_timestamp(&item.date, &item.time)? * 1000,
                event: item.event,
                impact: item.impact,
                r#for: item.r#for,
                actual: item.actual,
                expected: item.expected,
                prior: item.prior,
            })
        })
        .collect()
}

/// Convert a fetched earnings export: the date cell (`2026-11-10 16:05`,
/// time part sometimes absent) becomes a millisecond timestamp.
pub fn earnings_rows(items: Vec<earnings::Item>) -> anyhow::Result<Vec<EarningsItem>> {
    items
        .into_iter()
        .map(|item| {
            let (date, time) = item.date.split_once(' ').unwrap_or((&item.date, ""));
            Ok(EarningsItem {
                time_ms: build_timestamp(date, time)? * 1000,
                symbol: item.symbol,
                company: item.company,
                market_cap: item.market_cap,
                eps_estimate: item.eps_estimate,
                eps_actual: item.eps_actual,
                eps_surprise: item.eps_surprise,
                eps_gaap_estimate: item.eps_gaap_estimate,
                eps_gaap_actual: item.eps_gaap_actual,
                eps_gaap_surprise: item.eps_gaap_surprise,
                revenue_estimate: item.revenue_estimate,
                revenue_actual: item.revenue_actual,
                revenue_surprise: item.revenue_surprise,
                one_day_price_reaction: item.one_day_price_reaction,
            })
        })
        .collect()
}

/// `date`(`YYYY-MM-DD`)所在周的 Finviz 导出窗口:吸附到周一、取
/// Mon–Fri,匹配 Finviz Elite 日历自身的周视图(RPC `EconomicCalendar`
/// 的周吸附从这里完成)。
pub fn economic_week_for_date(date: &str) -> anyhow::Result<economic::Parameter> {
    let d = NaiveDate::parse_from_str(date.trim(), "%Y-%m-%d")
        .with_context(|| format!("failed to parse date: {date:?}"))?;
    let from = snap_to_monday(d);
    let to = from + Duration::days(4);
    Ok(economic::Parameter::new(from.to_string(), to.to_string()))
}

fn snap_to_monday(d: NaiveDate) -> NaiveDate {
    d - Duration::days(d.weekday().num_days_from_monday() as i64)
}

/// Unix seconds for a US-Eastern wall-clock cell: `date` is `YYYY-MM-DD`,
/// `time` is `HH:MM`, `HH:MM:SS`, `All Day` or empty (the last two pin the
/// event to midnight).
fn build_timestamp(date: &str, time: &str) -> anyhow::Result<i64> {
    let d = NaiveDate::parse_from_str(date.trim(), "%Y-%m-%d")
        .with_context(|| format!("failed to parse date: {date:?}"))?;

    let t = match time.trim() {
        "" | "All Day" => "00:00",
        t => t,
    };

    let parsed_time = if t.matches(':').count() == 2 {
        NaiveTime::parse_from_str(t, "%H:%M:%S")
    } else {
        NaiveTime::parse_from_str(t, "%H:%M")
    };
    let t = parsed_time.with_context(|| format!("failed to parse time: {time:?}"))?;

    let naive = d.and_time(t);

    let local = New_York
        .from_local_datetime(&naive)
        .single()
        .with_context(|| {
            format!(
                "local time {naive} is ambiguous or nonexistent at the US Eastern DST transition"
            )
        })?;

    Ok(local.timestamp())
}

use anysdk::finviz_sdk::calendar::{earnings, economic};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_timestamp_matches_eastern_wall_clock() {
        // build_timestamp 返回秒;毫秒换算发生在 economic_rows/earnings_rows。
        let expected = |h: u32, m: u32, s: u32| {
            New_York
                .with_ymd_and_hms(2026, 1, 14, h, m, s)
                .single()
                .unwrap()
                .timestamp()
        };
        assert_eq!(
            build_timestamp("2026-01-14", "08:30").unwrap(),
            expected(8, 30, 0)
        );
        assert_eq!(
            build_timestamp("2026-01-14", "08:30:45").unwrap(),
            expected(8, 30, 45)
        );
        // "All Day" and a missing time both pin the event to midnight.
        assert_eq!(
            build_timestamp("2026-01-14", "All Day").unwrap(),
            expected(0, 0, 0)
        );
        assert_eq!(
            build_timestamp("2026-01-14", "").unwrap(),
            expected(0, 0, 0)
        );
        assert!(build_timestamp("2026-01-14", "bogus").is_err());
        assert!(build_timestamp("not a date", "08:30").is_err());
    }

    #[test]
    fn week_parameter_snaps_to_monday_through_friday() {
        // 2026-01-14 is a Wednesday; the week is Mon 12th – Fri 16th.
        let parameter = economic_week_for_date("2026-01-14").unwrap();
        assert_eq!(parameter.from, "2026-01-12");
        assert_eq!(parameter.to, "2026-01-16");

        // A Monday input snaps to itself.
        let parameter = economic_week_for_date("2026-01-12").unwrap();
        assert_eq!(parameter.from, "2026-01-12");
        assert_eq!(parameter.to, "2026-01-16");

        // 周六同样归到所在周的周一(2026-01-17 是周六)。
        let parameter = economic_week_for_date("2026-01-17").unwrap();
        assert_eq!(parameter.from, "2026-01-12");

        assert!(economic_week_for_date("not a date").is_err());
    }

    /// 经济日历行转换:日期/时间单元格 → 毫秒时间戳,其余字段原样搬运。
    #[test]
    fn economic_rows_convert_to_protocol_items() {
        let items = vec![economic::Item {
            event: "CPI m/m".into(),
            date: "2026-01-14".into(),
            time: "08:30".into(),
            impact: 3,
            r#for: Some("USD".into()),
            actual: Some("0.2%".into()),
            expected: None,
            prior: Some("0.1%".into()),
        }];
        let rows = economic_rows(items).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].event, "CPI m/m");
        assert_eq!(rows[0].impact, 3);
        assert_eq!(rows[0].r#for.as_deref(), Some("USD"));
        let expected_ms = New_York
            .with_ymd_and_hms(2026, 1, 14, 8, 30, 0)
            .single()
            .unwrap()
            .timestamp()
            * 1000;
        assert_eq!(rows[0].time_ms, expected_ms);

        // 单个坏单元格让整批失败(调用方走错误路径)。
        let bad = vec![economic::Item {
            event: "X".into(),
            date: "bogus".into(),
            time: "".into(),
            impact: 1,
            r#for: None,
            actual: None,
            expected: None,
            prior: None,
        }];
        assert!(economic_rows(bad).is_err());
    }

    /// 财报行转换:日期带时间/缺时间两种形态都解析。
    #[test]
    fn earnings_rows_convert_to_protocol_items() {
        let items = vec![earnings::Item {
            date: "2026-11-10 16:05".into(),
            symbol: "AAPL".into(),
            company: "Apple Inc.".into(),
            market_cap: Some(3500.0),
            eps_estimate: Some(1.5),
            eps_actual: Some(1.7),
            eps_surprise: Some(13.3),
            eps_gaap_estimate: None,
            eps_gaap_actual: None,
            eps_gaap_surprise: None,
            revenue_estimate: Some(100000.0),
            revenue_actual: None,
            revenue_surprise: None,
            one_day_price_reaction: Some(2.5),
        }];
        let rows = earnings_rows(items).unwrap();
        let expected_ms = New_York
            .with_ymd_and_hms(2026, 11, 10, 16, 5, 0)
            .single()
            .unwrap()
            .timestamp()
            * 1000;
        assert_eq!(rows[0].time_ms, expected_ms);
        assert_eq!(rows[0].symbol, "AAPL");
        assert_eq!(rows[0].eps_surprise, Some(13.3));
        assert_eq!(rows[0].eps_gaap_estimate, None);
    }
}
