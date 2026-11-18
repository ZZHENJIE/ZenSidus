//! Compile-time embedded assets (`assets/`: icons, images, audio). Paths are
//! looked up here first and fall back to gpui-kit's built-in asset set, so
//! the app never depends on the working directory.

use gpui_kit::{AssetSource, SharedString, assets::Assets as BuiltinAssets};
use rust_embed::RustEmbed;
use std::borrow::Cow;

#[derive(RustEmbed)]
#[folder = "./assets"]
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<Cow<'static, [u8]>>> {
        if path.is_empty() {
            return Ok(None);
        }
        if let Some(file) = Self::get(path) {
            return Ok(Some(file.data));
        }
        BuiltinAssets.load(path)
    }

    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        let mut names: Vec<SharedString> = Self::iter()
            .filter(|p| p.starts_with(path))
            .map(|p| p.into())
            .collect();
        for name in BuiltinAssets.list(path)? {
            if !names.contains(&name) {
                names.push(name);
            }
        }
        Ok(names)
    }
}
