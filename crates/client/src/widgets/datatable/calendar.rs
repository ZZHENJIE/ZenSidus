//! The calendar-family table delegates: the economic calendar, the earnings
//! calendar and the IPO calendar. The shared helpers live in the datatable
//! root ([`crate::widgets::datatable::tint_upcoming_row`]) and the data
//! layer ([`crate::data::format`] for the local-time stamps).

pub mod earnings;
pub mod economic;
pub mod ipo;

pub use earnings::EarningsDelegate;
pub use economic::EconomicDelegate;
pub use ipo::IPODelegate;
