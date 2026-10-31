#![cfg(target_os = "macos")]

use std::process::Stdio;
use std::time::Duration;

async fn run_fixture(shell_probe: bool) {
    use nix::sys::signal::{killpg, Signal};
    use nix::unistd::Pid;

    let binary = std::env::var("BUZZ_TEST_CODEX")
        .expect("explicit BUZZ_TEST_CODEX absolute path required; never discover a live account");
    assert!(std::path::Path::new(&binary).is_absolute());
    let mut child = tokio::process::Command::new("/usr/bin/python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/codex_project_scope.py"
        ))
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("BUZZ_TEST_CODEX", binary)
        .env(
            "BUZZ_SYNTHETIC_SHELL_PROBE",
            if shell_probe { "1" } else { "0" },
        )
        .current_dir("/private/tmp")
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .process_group(0)
        .spawn()
        .expect("start isolated Codex contract fixture");
    let process_group = Pid::from_raw(child.id().expect("fixture pid") as i32);
    let result = tokio::time::timeout(Duration::from_secs(60), child.wait()).await;
    match killpg(process_group, Signal::SIGKILL) {
        Ok(()) | Err(nix::errno::Errno::ESRCH) => {}
        Err(error) => panic!("fixture process containment failed: {error}"),
    }
    let status = result
        .expect("bounded fixture timed out; process group terminated")
        .expect("wait for contract fixture");
    assert!(status.success(), "actual Codex contract was not enforced");
}

#[tokio::test]
#[ignore = "requires explicit Codex binary and authorization for an isolated localhost-only fixture"]
async fn codex_project_tool_inventory_is_broker_only() {
    run_fixture(false).await;
}

#[tokio::test]
#[ignore = "known failing with Codex 0.153.4: named profile permits sibling reads and writes"]
async fn codex_project_named_profile_confines_actual_model_tool() {
    run_fixture(true).await;
}
