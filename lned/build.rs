use std::env::consts::{ARCH, OS};
use std::process::Command;

#[cfg(debug_assertions)]
const BUILD_TYPE: &'static str = "debug";
#[cfg(not(debug_assertions))]
const BUILD_TYPE: &'static str = "release";

fn main() {
    let version = env!("CARGO_PKG_VERSION");
    let commit = get_commit();
    let tree_state = if is_working_tree_clean() { "" } else { "+" };

    let version_string = format!(
        "v{version} ({commit}{tree_state} {BUILD_TYPE} {OS} {ARCH})"
    );

    println!("cargo::rustc-env=LNED_VERSION={version_string}");
}

fn get_commit() -> String {
    let git_out = Command::new("git")
        .arg("log")
        .arg("-1")
        .arg("--pretty=format:%h")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();

    assert!(git_out.status.success());

    String::from_utf8_lossy(&git_out.stdout).to_string()
}

fn is_working_tree_clean() -> bool {
    let status = Command::new("git")
        .arg("diff")
        .arg("--quiet")
        .arg("--exit-code")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .status()
        .unwrap();

    status.code().unwrap() == 0
}
