//! Display formatting shared by every data panel: prices, sizes, volumes,
//! money/percent cells and local-time stamps. Pure functions — no GPUI
//! types, no I/O — so they stay unit-testable and reusable outside panels.
//!
//! `millions`/`dollars`/`signed_pct` format the earnings calendar's typed
//! money/percent cells (`Option<f64>`, parsed server-side).

use chrono::TimeZone;

/// A calendar row's timestamp (epoch ms) shown in the viewer's local time
/// (`MM-DD HH:MM`), shared by the economic/earnings delegates and news rows.
pub fn format_ts_local(time_ms: i64) -> String {
    match chrono::Local.timestamp_millis_opt(time_ms) {
        chrono::LocalResult::Single(time) => time.format("%m-%d %H:%M").to_string(),
        _ => String::new(),
    }
}

/// A calendar row's date (epoch ms) in the viewer's local time (`MM-DD`),
/// for tables that only show the day part.
pub fn format_date_local(time_ms: i64) -> String {
    match chrono::Local.timestamp_millis_opt(time_ms) {
        chrono::LocalResult::Single(time) => time.format("%m-%d").to_string(),
        _ => String::new(),
    }
}

/// Price legend format: two decimals normally, sub-dollar prices keep four.
pub fn format_quote(price: f64) -> String {
    if price.abs() < 1. {
        format!("{:.4}", price)
    } else {
        format!("{:.2}", price)
    }
}

/// Volume abbreviation: 1_234_567 → "1.23M", 12_345 → "12.3K", < 1000 as-is.
pub fn format_volume(volume: i64) -> String {
    let v = volume as f64;
    if v.abs() >= 1e9 {
        format!("{:.2}B", v / 1e9)
    } else if v.abs() >= 1e6 {
        format!("{:.2}M", v / 1e6)
    } else if v.abs() >= 1e3 {
        format!("{:.1}K", v / 1e3)
    } else {
        volume.to_string()
    }
}

/// `1,234,567` — sizes read better grouped.
pub fn format_size(size: i64) -> String {
    let digits = size.abs().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    if size < 0 {
        out.insert(0, '-');
    }
    out
}

/// Money in the export is in millions: `$12.86M`, `≥1000` rolls to `$2.59B`.
pub fn millions(value: Option<f64>) -> String {
    match value {
        Some(v) if v.abs() >= 1000. => format!("${:.2}B", v / 1000.),
        Some(v) => format!("${v:.2}M"),
        None => String::new(),
    }
}

/// EPS dollars: `$0.12`, negative as `-$0.21`.
pub fn dollars(value: Option<f64>) -> String {
    match value {
        Some(v) if v < 0. => format!("-${:.2}", -v),
        Some(v) => format!("${v:.2}"),
        None => String::new(),
    }
}

/// Signed percentage: `+5.04%` / `-4.43%` / `0.00%`.
pub fn signed_pct(value: Option<f64>) -> String {
    match value {
        Some(v) if v > 0. => format!("+{v:.2}%"),
        Some(v) => format!("{v:.2}%"),
        None => String::new(),
    }
}

/// A trade's exchange time as the viewer's local wall clock (`HH:MM:SS`),
/// the watchlist's "last price at" column.
pub fn format_clock_local(time: chrono::DateTime<chrono::Utc>) -> String {
    time.with_timezone(&chrono::Local)
        .format("%H:%M:%S")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_format_keeps_sub_dollar_precision() {
        assert_eq!(format_quote(123.45), "123.45");
        assert_eq!(format_quote(0.1234), "0.1234");
    }

    #[test]
    fn volume_abbreviates() {
        assert_eq!(format_volume(999), "999");
        assert_eq!(format_volume(12_345), "12.3K");
        assert_eq!(format_volume(1_234_567), "1.23M");
        assert_eq!(format_volume(2_500_000_000), "2.50B");
    }

    #[test]
    fn size_groups_thousands_and_keeps_sign() {
        assert_eq!(format_size(0), "0");
        assert_eq!(format_size(999), "999");
        assert_eq!(format_size(1_000), "1,000");
        assert_eq!(format_size(1_234_567), "1,234,567");
        assert_eq!(format_size(-1_234_567), "-1,234,567");
    }

    #[test]
    fn money_cells_render_blank_without_value() {
        assert_eq!(millions(None), "");
        assert_eq!(millions(Some(12.86)), "$12.86M");
        assert_eq!(millions(Some(2_589.)), "$2.59B");
        assert_eq!(dollars(None), "");
        assert_eq!(dollars(Some(0.12)), "$0.12");
        assert_eq!(dollars(Some(-0.21)), "-$0.21");
    }

    #[test]
    fn signed_pct_carries_explicit_plus() {
        assert_eq!(signed_pct(None), "");
        assert_eq!(signed_pct(Some(5.04)), "+5.04%");
        assert_eq!(signed_pct(Some(-4.43)), "-4.43%");
        assert_eq!(signed_pct(Some(0.)), "0.00%");
    }

    #[test]
    fn local_timestamps_render_some_marking() {
        // The exact text depends on the machine's timezone; the contract is
        // "a valid epoch renders the pattern, an invalid one renders empty".
        assert_eq!(format_ts_local(0).len(), "MM-DD HH:MM".len());
        assert_eq!(format_date_local(0).len(), "MM-DD".len());
        assert_eq!(format_ts_local(i64::MAX), "");
        assert_eq!(format_date_local(i64::MAX), "");
    }
}
