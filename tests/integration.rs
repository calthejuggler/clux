use std::process::Command;

fn clux_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_clux"))
}

#[test]
fn no_args_prints_usage() {
    let output = clux_bin().output().expect("failed to execute");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Usage:"),
        "expected usage message, got: {stderr}"
    );
}

#[test]
fn unknown_command_prints_usage() {
    let output = clux_bin()
        .arg("foobar")
        .output()
        .expect("failed to execute");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Usage:"),
        "expected usage message, got: {stderr}"
    );
}

#[test]
fn list_runs_without_crash() {
    let output = clux_bin().arg("list").output().expect("failed to execute");
    assert!(
        output.status.success(),
        "list command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
