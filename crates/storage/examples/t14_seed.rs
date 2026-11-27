//! Seed an isolated old release through the production storage API, for T14 acceptance only.
use envlume_storage::{
    StorePaths,
    cli_install::{self, Release},
};
use std::{
    fs::{self, File},
    path::PathBuf,
};
fn main() {
    let [root, manifest, image]: [std::ffi::OsString; 3] = std::env::args_os()
        .skip(1)
        .collect::<Vec<_>>()
        .try_into()
        .expect("root manifest image");
    let paths = if root == "--local" {
        StorePaths::local()
    } else {
        StorePaths::at(PathBuf::from(root))
    }
    .unwrap();
    assert!(
        !paths.root().join("bin/cli-releases.json").exists(),
        "fixture requires a fresh installation"
    );
    let release: Release = serde_json::from_slice(&fs::read(manifest).unwrap()).unwrap();
    let installed =
        cli_install::install(&paths, &release, File::open(image).unwrap(), false).unwrap();
    println!("{}", serde_json::json!(installed));
}
