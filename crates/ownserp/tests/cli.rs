use std::process::Command;

fn ownserp() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ownserp"))
}

#[test]
fn version_prints_the_package_version() {
    let output = ownserp().arg("--version").output().expect("run");
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).expect("utf8");
    assert!(text.contains(env!("CARGO_PKG_VERSION")), "got: {text}");
}

#[test]
fn a_subcommand_accepts_version_too() {
    let output = ownserp()
        .args(["serve", "--version"])
        .output()
        .expect("run");
    assert!(output.status.success());
}

#[test]
fn help_lists_every_subcommand() {
    let output = ownserp().arg("--help").output().expect("run");
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).expect("utf8");
    for name in ["serve", "doctor", "config", "audit"] {
        assert!(text.contains(name), "{name} is missing from help");
    }
}

#[test]
fn an_unknown_flag_is_a_usage_error() {
    let output = ownserp().args(["serve", "--bogus"]).output().expect("run");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
}

#[test]
fn a_missing_subcommand_is_a_usage_error() {
    let output = ownserp().output().expect("run");
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn conflicting_transport_flags_are_refused() {
    let output = ownserp()
        .args(["serve", "--http", "--no-http"])
        .output()
        .expect("run");
    assert_eq!(output.status.code(), Some(2));
}
