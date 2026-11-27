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

#[test]
fn machine_errors_have_a_request_id_and_do_not_echo_input() {
    for args in [
        vec!["T13_PRIVATE_OPTION", "--json"],
        vec!["doctor", "--binding", "T13_PRIVATE_OPTION", "--json"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_envlume"))
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        let report: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("JSON error report");
        assert_eq!(report["errorCode"], "INVALID_INPUT");
        assert!(report["requestId"].is_string());
        assert!(!report.to_string().contains("T13_PRIVATE_OPTION"));
        assert!(!String::from_utf8_lossy(&output.stderr).contains("T13_PRIVATE_OPTION"));
    }
}
