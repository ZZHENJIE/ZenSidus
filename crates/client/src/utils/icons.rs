//! The app's own SVG icons (`assets/icons/*.svg`, `currentColor` fills),
//! usable anywhere gpui-kit accepts an `IconNamed`. Icons not listed here
//! fall back to gpui-kit's built-in set — but only the ~106 SVGs that set
//! actually ships (see `default-icons.txt`); an unlisted `IconName` renders
//! blank.

use gpui_kit::{SharedString, assets::IconNamed};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppIcon {
    Market,
    Watchlist,
    Trading,
    News,
    Tools,
    Settings,
    WindowMinimize,
    WindowRestore,
    WindowMaximize,
    WindowClose,
    SmallClock,
    Search,
    Alpaca,
    Palette,
    Log,
    Quote,
    Trades,
    Filter,
    Bell,
    BellOff,
    Calendar,
    Chart,
    Book,
    Experiment,
    Refresh,
    Save,
    Custom,
    Plus,
    Trash,
    Pencil,
    Docs,
}

impl IconNamed for AppIcon {
    fn path(self) -> SharedString {
        match self {
            AppIcon::Market => "icons/market.svg".into(),
            AppIcon::Watchlist => "icons/watchlist.svg".into(),
            AppIcon::Trading => "icons/trading.svg".into(),
            AppIcon::News => "icons/news.svg".into(),
            AppIcon::Custom => "icons/custom.svg".into(),
            AppIcon::Tools => "icons/tools.svg".into(),
            AppIcon::Settings => "icons/settings.svg".into(),
            AppIcon::Alpaca => "icons/alpaca.svg".into(),
            AppIcon::Save => "icons/save.svg".into(),
            AppIcon::Log => "icons/log.svg".into(),
            AppIcon::Quote => "icons/quotes.svg".into(),
            AppIcon::Trades => "icons/trades.svg".into(),
            AppIcon::Filter => "icons/filter.svg".into(),
            AppIcon::Trash => "icons/trash.svg".into(),
            AppIcon::Pencil => "icons/pencil.svg".into(),
            AppIcon::Refresh => "icons/refresh.svg".into(),
            AppIcon::Plus => "icons/plus.svg".into(),
            AppIcon::Chart => "icons/chart.svg".into(),
            AppIcon::Book => "icons/book.svg".into(),
            AppIcon::Experiment => "icons/experiment.svg".into(),
            AppIcon::Calendar => "icons/calendar.svg".into(),
            AppIcon::Bell => "icons/bell.svg".into(),
            AppIcon::BellOff => "icons/bell-off.svg".into(),
            AppIcon::Docs => "icons/docs.svg".into(),
            AppIcon::Search => "icons/search.svg".into(),
            AppIcon::Palette => "icons/palette.svg".into(),
            AppIcon::SmallClock => "icons/small-clock.svg".into(),
            AppIcon::WindowMinimize => "icons/window-minimize.svg".into(),
            AppIcon::WindowRestore => "icons/window-restore.svg".into(),
            AppIcon::WindowMaximize => "icons/window-maximize.svg".into(),
            AppIcon::WindowClose => "icons/window-close.svg".into(),
        }
    }
}
