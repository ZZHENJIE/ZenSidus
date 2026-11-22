//! K 线组件的纯几何与时间函数:价格刻度、时间桶、美东时间标签。全部
//! 无状态、不触 GPUI,便于单测。(价格/量值等展示格式在
//! [`crate::data::format`],全 UI 共用。)
//!
//! 时区:美东(America/New_York)是美股的交易所时区,日界、会话边界都按它
//! 解释;桶对齐也在美东意义下成立——NY 的 UTC 偏移恒为整小时(只有凌晨
//! 2 点的 DST 切换例外,而该时段不处于交易会话内),所以分钟/小时桶直接
//! 按 UTC 整除对齐即可。

use chrono::{DateTime, Datelike, TimeZone, Utc};
use chrono_tz::{America::New_York, Tz};

/// 一根 bar 的时间毫秒 → 美东墙钟时间(时间轴标签与日界的唯一出处)。
fn ny_time(time_ms: i64) -> DateTime<Tz> {
    Utc.timestamp_millis_opt(time_ms)
        .earliest()
        .unwrap_or_else(Utc::now)
        .with_timezone(&New_York)
}

/// 把毫秒时间戳 floor 到 `bucket_minutes` 的整桶起点(UTC 对齐,见模块
/// 注释)。`bucket_minutes` 必须 ≥ 1;日线不用它(用 [`crate::widgets::kline::scale::ny_date`])。
pub fn bucket_start_ms(time_ms: i64, bucket_minutes: u32) -> i64 {
    debug_assert!(bucket_minutes >= 1);
    let step = (bucket_minutes.max(1) as i64) * 60_000;
    time_ms.div_euclid(step) * step
}

/// 毫秒时间戳对应的美东交易日。
pub fn ny_date(time_ms: i64) -> chrono::NaiveDate {
    ny_time(time_ms).date_naive()
}

/// 美东某日零点的毫秒时间戳(零点不落 DST 切换时刻,恒有唯一映射)。
pub fn ny_date_ms(date: chrono::NaiveDate) -> i64 {
    date.and_hms_opt(0, 0, 0)
        .and_then(|ny_midnight| New_York.from_local_datetime(&ny_midnight).single())
        .map(|dt| dt.with_timezone(&Utc).timestamp_millis())
        .unwrap_or_else(|| {
            date.and_hms_opt(0, 0, 0)
                .unwrap()
                .and_utc()
                .timestamp_millis()
        })
}

/// 美东该交易日所在周的周一零点(周桶起点)。
pub fn ny_week_start_ms(time_ms: i64) -> i64 {
    let date = ny_date(time_ms);
    let monday = date - chrono::Duration::days(date.weekday().num_days_from_monday() as i64);
    ny_date_ms(monday)
}

/// 美东该交易日所在月 1 日零点(月桶起点)。
pub fn ny_month_start_ms(time_ms: i64) -> i64 {
    let date = ny_date(time_ms);
    ny_date_ms(chrono::NaiveDate::from_ymd_opt(date.year(), date.month(), 1).unwrap())
}

/// 美东 "HH:mm"(分钟周期的时间轴刻度)。
pub fn format_ny_hm(time_ms: i64) -> String {
    ny_time(time_ms).format("%H:%M").to_string()
}

/// 美东 "MM-dd"(时间轴上跨日刻度)。
pub fn format_ny_md(time_ms: i64) -> String {
    ny_time(time_ms).format("%m-%d").to_string()
}

/// 美东 "yyyy-MM-dd"(日线的十字光标时间签、新年份刻度)。
pub fn format_ny_date(time_ms: i64) -> String {
    ny_time(time_ms).format("%Y-%m-%d").to_string()
}

/// 美东 "yyyy-MM-dd HH:mm"(分钟周期的十字光标时间签)。
pub fn format_ny_datetime(time_ms: i64) -> String {
    ny_time(time_ms).format("%Y-%m-%d %H:%M").to_string()
}

/// 价格轴刻度:在 `[min, max]` 上按 "1-2-5" 阶梯选步长,返回从
/// `ceil(min/step)*step` 起、≤ max 的刻度值列表。`target` 是期望刻度数
/// (过密的请求被步长机制自然稀释);`min >= max` 时返回空。
pub fn price_ticks(min: f64, max: f64, target: usize) -> Vec<f64> {
    if !min.is_finite() || !max.is_finite() || min >= max {
        return Vec::new();
    }
    let step = price_step(max - min, target);
    let first = (min / step).ceil() * step;
    let mut ticks = Vec::new();
    let mut tick = first;
    while tick <= max + step * 1e-9 {
        // 浮点累积误差:归一到步长网格再写回。
        ticks.push((tick / step).round() * step);
        tick += step;
    }
    ticks
}

/// "1-2-5" 步长:range/target 的数量级归一后就近取 1/2/5。边界值(恰为
/// 1.5/3.5/7.5)进位到更大步长,避免浮点误差把 0.9/6 这类请求打到过密的
/// 步长上。
pub fn price_step(range: f64, target: usize) -> f64 {
    if range <= 0. {
        return 1.;
    }
    let raw = range / target.max(1) as f64;
    let magnitude = 10f64.powf(raw.log10().floor());
    let normalized = raw / magnitude;
    let step = if normalized < 1.5 - 1e-9 {
        1.
    } else if normalized < 3.5 - 1e-9 {
        2.
    } else if normalized < 7.5 - 1e-9 {
        5.
    } else {
        10.
    };
    step * magnitude
}

/// 按步长的有效小数位格式化价格(1 → "102",0.5 → "102.5",0.25 →
/// "102.25")。
pub fn format_price(price: f64, step: f64) -> String {
    format!("{:.*}", decimals_for_step(step), price)
}

/// 步长的小数位数:使 `step * 10^d` 成为整数的最小 d(≤ 6)。
pub fn decimals_for_step(step: f64) -> usize {
    if step <= 0. || !step.is_finite() {
        return 2;
    }
    let mut decimals = 0;
    let mut scaled = step;
    while decimals < 6 && (scaled - scaled.round()).abs() > 1e-9 {
        scaled *= 10.;
        decimals += 1;
    }
    decimals
}

/// 时间轴刻度步长(单位:bar 数):按 "1-2-5" 阶梯取最小步长,保证相邻
/// 刻度间距 ≥ `min_gap_px`。`body_w <= 0` 时退到 1。
pub fn time_tick_step(body_w: f32, min_gap_px: f32) -> usize {
    if body_w <= 0. {
        return 1;
    }
    let mut step = 1usize;
    loop {
        if step as f32 * body_w >= min_gap_px {
            return step;
        }
        step = next_nice_step(step);
    }
}

fn next_nice_step(step: usize) -> usize {
    match step {
        1 => 2,
        2 => 5,
        5 => 10,
        10 => 20,
        20 => 50,
        50 => 100,
        100 => 200,
        200 => 500,
        _ => step * 10,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_floor_aligns_to_utc_minutes() {
        let epoch = Utc.timestamp_millis_opt(0).unwrap().timestamp_millis();
        // 14:37:05 UTC → 5 分钟桶 14:35:00。
        let t = epoch + 14 * 3_600_000 + 37 * 60_000 + 5_000;
        assert_eq!(bucket_start_ms(t, 5), epoch + 14 * 3_600_000 + 35 * 60_000);
        assert_eq!(bucket_start_ms(t, 1), epoch + 14 * 3_600_000 + 37 * 60_000);
        assert_eq!(bucket_start_ms(t, 60), epoch + 14 * 3_600_000);
        // 负偏移(1970 前)也必须整除,而不是向零截断。
        assert_eq!(bucket_start_ms(-1, 1), -60_000);
    }

    #[test]
    fn ny_date_uses_exchange_day() {
        // 2026-11-20 04:00 UTC = 美东 11-19 23:00(冬令时)→ 交易日是 11-19。
        let ms = Utc
            .with_ymd_and_hms(2026, 11, 20, 4, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        assert_eq!(ny_date(ms).to_string(), "2026-11-19");
    }

    #[test]
    fn ny_labels_format() {
        let ms = Utc
            .with_ymd_and_hms(2026, 3, 6, 15, 5, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        assert_eq!(format_ny_hm(ms), "10:05"); // EST(UTC-5)
        assert_eq!(format_ny_md(ms), "03-06");
        assert_eq!(format_ny_date(ms), "2026-03-06");
        assert_eq!(format_ny_datetime(ms), "2026-03-06 10:05");
    }

    #[test]
    fn price_ticks_pick_readable_steps() {
        let approx = |ticks: &[f64], expected: &[f64]| {
            assert_eq!(ticks.len(), expected.len());
            for (tick, want) in ticks.iter().zip(expected) {
                assert!((tick - want).abs() < 1e-9, "{tick} != {want}");
            }
        };
        approx(
            &price_ticks(100., 110., 5),
            &[100., 102., 104., 106., 108., 110.],
        );
        approx(&price_ticks(0., 0.9, 6), &[0., 0.2, 0.4, 0.6, 0.8]);
        assert!(price_ticks(5., 5., 5).is_empty());
    }

    #[test]
    fn price_format_matches_step() {
        assert_eq!(format_price(102., 2.), "102");
        assert_eq!(format_price(102.5, 0.5), "102.5");
        assert_eq!(format_price(102.25, 0.25), "102.25");
    }

    #[test]
    fn ny_bucket_starts() {
        // 2026-11-18 是周三(冬令时):周一 = 11-16,月初 = 11-01,年初 = 01-01。
        let wed = Utc
            .with_ymd_and_hms(2026, 11, 18, 15, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        assert_eq!(ny_date(ny_week_start_ms(wed)).to_string(), "2026-11-16");
        assert_eq!(ny_date(ny_month_start_ms(wed)).to_string(), "2026-11-01");
        // 周一零点是美东午夜:桶起点当天(冬令时 UTC 05:00)。
        assert_eq!(
            ny_date(ny_week_start_ms(wed)),
            ny_date(wed) - chrono::Duration::days(2)
        );
    }

    #[test]
    fn time_tick_step_grows_on_nice_grid() {
        assert_eq!(time_tick_step(100., 80.), 1);
        assert_eq!(time_tick_step(40., 80.), 2);
        assert_eq!(time_tick_step(10., 80.), 10);
        assert_eq!(time_tick_step(0., 80.), 1);
    }
}
