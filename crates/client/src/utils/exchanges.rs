//! The US equity exchange codes Alpaca's quote `ax`/`bx` fields carry,
//! with their venue names. The Market Screener's preset dialog picks from
//! this table and the per-exchange tables label themselves with it.

/// `(code, short venue name)`, the standard single-letter SIP codes, as
/// Alpaca groups them: NYSE (P,N,A,C,M) / Cboe (Z,Y,K,J) / NASDAQ (B,X,Q)
/// plus the independents. Short names follow the MPID convention — the
/// letter is what the quote `ax`/`bx` fields actually carry.
pub const EXCHANGE_CODES: &[(&str, &str)] = &[
    ("A", "AMEX"),
    ("B", "BX"),
    ("C", "NSX"),
    ("F", "TXSE"),
    ("G", "24X"),
    ("H", "MIAX"),
    ("J", "EDGA"),
    ("K", "EDGX"),
    ("L", "LTSE"),
    ("M", "CHX"),
    ("N", "NYSE"),
    ("P", "ARCA"),
    ("Q", "NSDQ"),
    ("U", "MEMX"),
    ("V", "IEX"),
    ("X", "PSX"),
    ("Y", "BYX"),
    ("Z", "BZX"),
];

/// `"P"` → `"NYSE Arca (P)"`; an unknown code passes through bare, so a
/// venue outside the table still renders something meaningful.
pub fn exchange_label(code: &str) -> String {
    match EXCHANGE_CODES.iter().find(|(known, _)| *known == code) {
        Some((_, name)) => format!("{name} ({code})"),
        None => code.to_string(),
    }
}
