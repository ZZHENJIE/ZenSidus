fn main() {
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=assets/icons/zen-sidus.ico");

        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icons/zen-sidus.ico");

        res.set("ProductVersion", env!("CARGO_PKG_VERSION"));
        res.set("FileDescription", "ZenSidus");
        res.set("LegalCopyright", "Copyright © 2026 zensidus.com");

        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed Windows icon: {e}");
        }
    }
}
