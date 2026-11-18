#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
use gpui_kit::application;
use client_lib::{bootstrap, init_app, panels, utils, windows};

fn main() -> anyhow::Result<()> {
    let (settings, _log_guard) = bootstrap()?;

    let app = application().with_assets(utils::Assets);
    app.run(move |cx| {
        let window_state = settings.window_state.clone();
        init_app(settings, cx);
        // Panel builders must be registered before any window restores a
        // persisted layout.
        panels::init(cx);
        windows::init(cx, &window_state);
        // After the windows exist so their status bars pick up the result.
        utils::state::RelayHub::auto_connect(cx);
    });

    Ok(())
}
