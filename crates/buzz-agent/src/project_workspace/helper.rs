use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::Path,
};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Request {
    root: String,
    path: String,
    replacement: Option<Replacement>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Replacement {
    pub expected_sha256: Option<String>,
    pub content: String,
}

pub(super) fn serve_if_requested() -> Option<Result<(), String>> {
    let args: Vec<_> = std::env::args_os().collect();
    if args
        .get(1)
        .is_none_or(|value| value != "--native-project-file-helper")
    {
        return None;
    }
    Some((|| {
        if args.len() != 2 {
            return Err("invalid native helper arguments".into());
        }
        let mut input = String::new();
        std::io::stdin()
            .take(524289)
            .read_to_string(&mut input)
            .map_err(|error| error.to_string())?;
        if input.len() > 524288 {
            return Err("native helper input exceeds limit".into());
        }
        let request: Request =
            serde_json::from_str(&input).map_err(|_| "invalid native helper input")?;
        let cwd = std::env::current_dir().map_err(|error| error.to_string())?;
        if cwd != Path::new(&request.root) {
            return Err("native helper workspace mismatch".into());
        }
        let content = match request.replacement {
            Some(replacement) => match replacement.expected_sha256 {
                Some(expected) => super::files::replace(
                    &request.root,
                    &request.path,
                    &expected,
                    &replacement.content,
                )?,
                None => super::files::create(&request.root, &request.path, &replacement.content)?,
            },
            None => super::files::read(&request.root, &request.path)?,
        };
        std::io::stdout()
            .write_all(content.as_bytes())
            .map_err(|error| error.to_string())
    })())
}

pub(super) async fn run(
    root: &str,
    path: &str,
    replacement: Option<Replacement>,
) -> Result<String, String> {
    super::files::validate_relative(path)?;
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (root, path, replacement);
        Err("native sandboxed project tools currently require macOS".into())
    }
    #[cfg(target_os = "macos")]
    {
        use std::process::Stdio;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let binary = std::env::current_exe().map_err(|error| error.to_string())?;
        let executable = serde_json::to_string(
            &binary
                .to_str()
                .ok_or("helper binary path must be Unicode")?,
        )
        .map_err(|error| error.to_string())?;
        let directory = serde_json::to_string(root).map_err(|error| error.to_string())?;
        let file = serde_json::to_string(
            &Path::new(root)
                .join(path)
                .to_str()
                .ok_or("file path must be Unicode")?,
        )
        .map_err(|error| error.to_string())?;
        let writable = if replacement.is_some() {
            format!("(literal {file})")
        } else {
            "(literal \"/dev/null\")".into()
        };
        let profile=format!("(version 1) (allow default) (deny network*)
            (deny file-write* (require-not (require-any (literal \"/dev/null\") {writable}))) (deny process-fork)
            (deny process-exec (require-not (literal {executable})))
            (deny file-read-data (require-not (require-any (literal \"/\") (path-ancestors {directory}) (subpath {directory})
                (subpath \"/System\") (subpath \"/usr/lib\") (literal {executable}) (literal \"/dev/null\"))))");
        let mut child = tokio::process::Command::new("/usr/bin/sandbox-exec")
            .args(["-p", &profile])
            .arg(binary)
            .arg("--native-project-file-helper")
            .env_clear()
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| format!("native sandbox helper unavailable: {error}"))?;
        let mut stdin = child.stdin.take().ok_or("helper stdin missing")?;
        let stdout = child.stdout.take().ok_or("helper stdout missing")?;
        let stderr = child.stderr.take().ok_or("helper stderr missing")?;
        let request = serde_json::to_vec(&Request {
            root: root.into(),
            path: path.into(),
            replacement,
        })
        .map_err(|error| error.to_string())?;
        let operation = async {
            stdin
                .write_all(&request)
                .await
                .map_err(|error| error.to_string())?;
            drop(stdin);
            let read_stdout = async {
                let mut bytes = Vec::new();
                stdout
                    .take(524289)
                    .read_to_end(&mut bytes)
                    .await
                    .map_err(|error| error.to_string())?;
                if bytes.len() > 524288 {
                    return Err("native helper exceeded output limit".to_string());
                }
                Ok(bytes)
            };
            let read_stderr = async {
                let mut bytes = Vec::new();
                stderr
                    .take(16385)
                    .read_to_end(&mut bytes)
                    .await
                    .map_err(|error| error.to_string())?;
                if bytes.len() > 16384 {
                    return Err("native helper exceeded diagnostic limit".to_string());
                }
                Ok(bytes)
            };
            let (content, errors) = tokio::try_join!(read_stdout, read_stderr)?;
            let status = child.wait().await.map_err(|error| error.to_string())?;
            if !status.success() {
                return Err(format!(
                    "native project read denied ({status}): {}",
                    String::from_utf8_lossy(&errors)
                ));
            }
            String::from_utf8(content).map_err(|_| "native project file is not UTF-8".into())
        };
        match tokio::time::timeout(std::time::Duration::from_secs(5), operation).await {
            Ok(Ok(content)) => Ok(content),
            failure => {
                child
                    .kill()
                    .await
                    .map_err(|error| format!("native helper containment failed: {error}"))?;
                match failure {
                    Ok(Err(error)) => Err(error),
                    _ => Err("native helper deadline exceeded".into()),
                }
            }
        }
    }
}
