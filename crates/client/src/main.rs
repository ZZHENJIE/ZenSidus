#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
use client_lib::{bootstrap, init_app, panels, prompt_connect, utils, windows};
use gpui_kit::application;

fn main() -> anyhow::Result<()> {
    let (settings, _log_guard) = bootstrap()?;

    let app = application().with_assets(utils::Assets);
    app.run(move |cx| {
        let window_state = settings.window_state.clone();
        init_app(settings, cx);
        // Panel builders must be registered before any window restores a
        // persisted layout. The server is not auto-connected: the auth code
        // is entered per session via the connect dialog.
        panels::init(cx);
        windows::init(cx, &window_state);
        // 启动即把连接入口送到用户眼前(地址已配置时);动态码每会话现输。
        prompt_connect(cx);
    });

    Ok(())
}
