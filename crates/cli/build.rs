use std::{env, fs, path::PathBuf};

#[path = "../storage/src/migrations.rs"]
mod migrations;

fn main() {
    println!("cargo:rerun-if-changed=../storage/src/migrations.rs");
    let schema = migrations::STEPS.len();
    println!("cargo:rerun-if-changed=build.rs");
    let target = env::var("TARGET").unwrap();
    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let path = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("cli-build.json");
    fs::write(
        path,
        format!(r#"{{"target":"{target}","version":"{version}","maxSchema":{schema}}}"#),
    )
    .unwrap();
}
