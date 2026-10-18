fn main() {
    // Embed the icon and version info into the Windows executable. GPUI
    // already links in an application manifest.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=resources/windows/gitter.rc");
        println!("cargo:rerun-if-changed=resources/gitter.ico");
        #[cfg(windows)]
        embed_resource::compile("resources/windows/gitter.rc", embed_resource::NONE).manifest_optional().expect("failed to embed Windows resources");
    }
}
