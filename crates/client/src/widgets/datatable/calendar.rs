//! The calendar-family table delegates: the economic calendar, the earnings
//! calendar and the IPO calendar. The shared row helpers
//! ([`crate::widgets::datatable::format_ts_local`] and
//! [`crate::widgets::datatable::tint_upcoming_row`]) live in the datatable
//! root.

pub mod earnings;
pub mod economic;
pub mod ipo;

pub use earnings::EarningsDelegate;
pub use economic::EconomicDelegate;
pub use ipo::IPODelegate;
