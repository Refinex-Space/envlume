use sha2::{Digest, Sha256};
use std::{env, fs, io, path::PathBuf};

fn check_embedded_cli() {
    let target = env::var("TARGET").unwrap();
    let stage = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join(".cli-stage")
        .join(&target)
        .join(env::var("PROFILE").unwrap());
    let image = stage.join("image");
    let manifest = stage.join("release.json");
    for path in [&manifest, &image] {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    let release: serde_json::Value = serde_json::from_slice(
        &fs::read(&manifest).expect("stage the matching CLI with the Tauri build hook"),
    )
    .unwrap();
    assert_eq!(
        release["version"],
        env!("CARGO_PKG_VERSION"),
        "desktop and CLI versions differ"
    );
    assert_eq!(release["target"], target, "desktop and CLI targets differ");
    let mut hash = Sha256::new();
    io::copy(&mut fs::File::open(&image).unwrap(), &mut hash).unwrap();
    assert_eq!(
        release["sha256"],
        format!("{:x}", hash.finalize()),
        "CLI staging digest mismatch"
    );
    println!("cargo:rustc-env=ENVLUME_BUNDLE_CLI={}", image.display());
    println!(
        "cargo:rustc-env=ENVLUME_BUNDLE_RELEASE={}",
        manifest.display()
    );
}
fn main() {
    if env::var_os("CARGO_FEATURE_BUNDLED_CLI").is_some() {
        check_embedded_cli();
    }
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
