use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub(crate) fn require_supported_launchers(
    agent: &str,
    args: &[String],
    harness: &str,
) -> Result<(), String> {
    if agent != "buzz-agent" || harness != "buzz-acp" || !args.is_empty() {
        return Err("native project execution does not support custom inner or outer launchers; the configured launcher was not replaced".into());
    }
    Ok(())
}

pub(crate) fn require_native_contract(binary: &Path, harness: bool) -> Result<(), String> {
    require_contract(
        binary,
        "--native-project-runtime-capabilities",
        if harness {
            "buzz-native-project-harness-v1\n"
        } else {
            "buzz-native-project-text-v1\n"
        },
    )
}

pub(crate) fn require_codex_broker_contract(binary: &Path) -> Result<(), String> {
    require_contract(
        binary,
        "--codex-project-bridge-capabilities",
        "buzz-codex-reasoning-broker-v1\n",
    )
}

fn require_contract(binary: &Path, argument: &str, expected: &str) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err("native project runtime requires macOS confinement".into());
    }
    let quoted = serde_json::to_string(
        &binary
            .to_str()
            .ok_or("native binary path must be Unicode")?,
    )
    .map_err(|error| error.to_string())?;
    let profile=format!("(version 1) (allow default) (deny network*) (deny file-write* (require-not (literal \"/dev/null\")))
        (deny process-fork) (deny process-exec (require-not (literal {quoted})))
        (deny file-read-data (require-not (require-any (literal \"/\") (subpath \"/System\") (subpath \"/usr/lib\") (literal {quoted}) (literal \"/dev/null\"))))");
    let mut child = Command::new("/usr/bin/sandbox-exec")
        .args(["-p", &profile])
        .arg(binary)
        .arg(argument)
        .env_clear()
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| error.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(3);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            other => {
                child
                    .kill()
                    .map_err(|error| format!("native probe containment failed: {error}"))?;
                child.wait().map_err(|error| error.to_string())?;
                return Err(match other {
                    Err(error) => error.to_string(),
                    _ => {
                        "native runtime capability probe timed out; runtime was not started".into()
                    }
                });
            }
        }
    };
    let mut output = String::new();
    child
        .stdout
        .take()
        .ok_or("native probe output missing")?
        .take(65)
        .read_to_string(&mut output)
        .map_err(|error| error.to_string())?;
    if !status.success() || output != expected {
        return Err("native runtime does not implement the verified project contract; rebuild the development runtime, without replacing the installed app".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_project_rejects_custom_outer_and_inner_launchers_without_replacement() {
        assert!(require_supported_launchers("buzz-agent", &[], "buzz-acp").is_ok());
        assert!(require_supported_launchers("buzz-agent", &[], "/synthetic/custom-acp").is_err());
        assert!(require_supported_launchers("/synthetic/custom-agent", &[], "buzz-acp").is_err());
        assert!(
            require_supported_launchers("buzz-agent", &["--custom".into()], "buzz-acp").is_err()
        );
    }
}
