use std::process::Command;

fn clux_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_clux"))
}

fn has_tmux() -> bool {
    Command::new("tmux")
        .arg("-V")
        .output()
        .is_ok_and(|o| o.status.success())
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
fn no_args_exits_with_code_2() {
    let output = clux_bin().output().expect("failed to execute");
    assert_eq!(output.status.code(), Some(2));
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
fn unknown_command_exits_with_code_2() {
    let output = clux_bin()
        .arg("foobar")
        .output()
        .expect("failed to execute");
    assert_eq!(output.status.code(), Some(2));
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

#[test]
fn list_output_is_tab_separated() {
    let output = clux_bin().arg("list").output().expect("failed to execute");
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        assert_eq!(
            fields.len(),
            8,
            "expected 8 tab-separated fields, got {}: {line}",
            fields.len()
        );
    }
}

#[test]
fn update_without_tmux_reports_error() {
    if has_tmux() {
        return;
    }
    let output = clux_bin()
        .arg("update")
        .output()
        .expect("failed to execute");
    assert!(!output.status.success());
}

#[test]
fn update_accepts_filter_argument() {
    if !has_tmux() {
        return;
    }
    for filter in &["all", "has-claude", "active", "idle"] {
        let output = clux_bin()
            .args(["update", filter])
            .output()
            .expect("failed to execute");
        assert!(
            output.status.success(),
            "update {filter} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn pick_without_tmux_reports_error() {
    if has_tmux() {
        return;
    }
    let output = clux_bin().arg("pick").output().expect("failed to execute");
    assert!(!output.status.success());
}

#[test]
fn select_without_tmux_reports_error() {
    if has_tmux() {
        return;
    }
    let output = clux_bin()
        .args(["select", "all"])
        .output()
        .expect("failed to execute");
    assert!(!output.status.success());
}
