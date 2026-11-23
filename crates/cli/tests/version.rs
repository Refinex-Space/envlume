use std::process::Command;

#[test]
fn reports_workspace_version_on_stdout() {
    let output = Command::new(env!("CARGO_BIN_EXE_envlume"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim_end(),
        format!("envlume {}", env!("CARGO_PKG_VERSION"))
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn rejects_unknown_arguments_without_polluting_stdout() {
    let output = Command::new(env!("CARGO_BIN_EXE_envlume"))
        .args(["--version", "unexpected"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());
}
