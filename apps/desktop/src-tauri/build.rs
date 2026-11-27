fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&["list_projects", "list_environments"]),
    ))
    .expect("build desktop command permissions");

    // tauri-winres links the compiled application manifest only into Cargo bins.
    // The native WebView example needs the same Common Controls v6 activation.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
        println!(
            "cargo::rustc-link-arg-examples={}",
            out.join("resource.lib").display()
        );
    }
}
