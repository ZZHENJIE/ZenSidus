//! The US equity exchange codes Alpaca's quote `ax`/`bx` fields carry,
//! with their venue names. The Market Screener's preset dialog picks from
//! this table and the per-exchange tables label themselves with it.

/// `(code, venue name)`, the standard single-letter SIP codes, as Alpaca
/// groups them: NYSE (P,N,A,C,M) / Cboe (Z,Y,K,J) / NASDAQ (B,X,Q) plus
/// the independents.
pub const EXCHANGE_CODES: &[(&str, &str)] = &[
    ("A", "NYSE American"),
    ("B", "NASDAQ Texas"),
    ("C", "NYSE National"),
    ("F", "Texas Stock Exchange"),
    ("G", "24X National Exchange"),
    ("H", "MIAX Pearl"),
    ("J", "EDGA Equities"),
    ("K", "EDGX Equities"),
    ("L", "LTSE"),
    ("M", "NYSE Texas"),
    ("N", "NYSE"),
    ("P", "NYSE Arca"),
    ("Q", "NASDAQ"),
    ("U", "MEMX Equities"),
    ("V", "Investors Exchange"),
    ("X", "NASDAQ PSX"),
    ("Y", "BYX Equities"),
    ("Z", "BZX Equities"),
];

/// `"P"` → `"NYSE Arca (P)"`; an unknown code passes through bare, so a
/// venue outside the table still renders something meaningful.
pub fn exchange_label(code: &str) -> String {
    match EXCHANGE_CODES.iter().find(|(known, _)| *known == code) {
        Some((_, name)) => format!("{name} ({code})"),
        None => code.to_string(),
    }
}
