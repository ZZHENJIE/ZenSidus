//! The calendar-family dock panels: the economic calendar, the earnings
//! calendar and the IPO calendar, plus the scaffolding they share
//! ([`common`]).

pub(crate) mod common;
pub mod earnings;
pub mod economic;
pub mod ipo;

pub use earnings::EarningsCalendarPanel;
pub use economic::EconomicCalendarPanel;
pub use ipo::IPOCalendarPanel;
