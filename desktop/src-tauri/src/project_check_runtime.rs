use serde::Serialize;
use std::path::{Path, PathBuf};

/// Evidence from the fixed, read-only native checker, not a model authentication probe.
#[derive(Debug, Serialize)]
pub struct ProjectCheckEvidence {
    /// Actual child process working directory.
    pub actual_cwd: String,
    /// Git revision; absent for a folder that is not a repository.
    pub revision: Option<String>,
    /// Symbolic branch or detached revision.
    pub branch: Option<String>,
    /// Whether Git reports uncommitted or untracked work.
    pub dirty: Option<bool>,
    /// Explicit runtime scope; never a provider readiness claim.
    pub runtime: &'static str,
}

#[cfg(target_os = "macos")]
fn profile(root: &Path, executable: &str) -> Result<String, String> {
    let root = root.to_str().ok_or("workspace path must be Unicode")?;
    let quote = |value: &str| serde_json::to_string(value).map_err(|error| error.to_string());
    Ok(format!("(version 1) (allow default)
        (deny network*) (deny file-write* (require-not (literal \"/dev/null\"))) (deny process-fork)
        (deny process-exec (require-not (literal {})))
        (deny file-read-data (require-not (require-any (literal \"/\") (subpath {}) (subpath \"/System\")
            (subpath \"/usr/lib\") (subpath \"/Library/Developer/CommandLineTools\")
            (literal {}) (literal \"/dev/null\"))))",
        quote(executable)?, quote(root)?, quote(executable)?))
}

#[cfg(target_os = "macos")]
async fn run(root: &Path, executable: &str, args: &[&str]) -> Result<String, String> {
    match run_with_limit(root, executable, args, 65536).await? {
        (output, false) => Ok(output),
        (_, true) => Err("checker output exceeded limit".into()),
    }
}

/// Like [`run`], but returns `(output, truncated)` instead of failing when the
/// child prints more than `limit` bytes. Used for diffs, whose size is not ours
/// to bound; the caller shows the truncation instead of pretending completeness.
#[cfg(target_os = "macos")]
async fn run_with_limit(
    root: &Path,
    executable: &str,
    args: &[&str],
    limit: u64,
) -> Result<(String, bool), String> {
    use std::process::Stdio;
    use tokio::io::AsyncReadExt;
    #[allow(non_snake_case)]
    let LIMIT = limit;
    let mut child = tokio::process::Command::new("/usr/bin/sandbox-exec")
        .args(["-p", &profile(root, executable)?, executable])
        .args(args)
        .current_dir(root)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LC_ALL", "C")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| format!("isolated checker unavailable: {error}"))?;
    let stdout = child.stdout.take().ok_or("checker stdout missing")?;
    let stderr = child.stderr.take().ok_or("checker stderr missing")?;
    let operation = async {
        let read = async |stream: tokio::process::ChildStdout| {
            let mut bytes = Vec::new();
            stream
                .take(LIMIT + 1)
                .read_to_end(&mut bytes)
                .await
                .map_err(|error| error.to_string())?;
            let truncated = bytes.len() as u64 > LIMIT;
            if truncated {
                bytes.truncate(LIMIT as usize);
            }
            Ok((bytes, truncated))
        };
        let error_read = async {
            let mut bytes = Vec::new();
            stderr
                .take(LIMIT + 1)
                .read_to_end(&mut bytes)
                .await
                .map_err(|error| error.to_string())?;
            if bytes.len() as u64 > LIMIT {
                return Err("checker diagnostics exceeded limit".to_string());
            }
            Ok(bytes)
        };
        let ((stdout, truncated), stderr) = tokio::try_join!(read(stdout), error_read)?;
        let status = child.wait().await.map_err(|error| error.to_string())?;
        if !status.success() {
            return Err(format!(
                "isolated checker failed ({status}): {}",
                String::from_utf8_lossy(&stderr)
            ));
        }
        let output = if truncated {
            String::from_utf8_lossy(&stdout).into_owned()
        } else {
            String::from_utf8(stdout).map_err(|_| "checker output is not Unicode")?
        };
        Ok((output, truncated))
    };
    match tokio::time::timeout(std::time::Duration::from_secs(5), operation).await {
        Ok(Ok(output)) => Ok(output),
        result => {
            child
                .kill()
                .await
                .map_err(|error| format!("checker containment failed: {error}"))?;
            match result {
                Ok(Err(error)) => Err(error),
                _ => Err("isolated checker deadline exceeded".into()),
            }
        }
    }
}

/// Run only fixed read operations; no shell, arbitrary command, network, or write grant.
pub(crate) async fn inspect(root: PathBuf) -> Result<ProjectCheckEvidence, String> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = root;
        Err("native project checker requires verified macOS sandbox support".into())
    }
    #[cfg(target_os = "macos")]
    {
        let root_text = root.to_str().ok_or("workspace path must be Unicode")?;
        crate::archive::channel_workspaces::validate_root(root_text)?;
        let actual_cwd = run(&root, "/bin/pwd", &["-P"])
            .await?
            .trim_end()
            .to_string();
        if Path::new(&actual_cwd) != root {
            return Err("checker working directory does not match current binding".into());
        }
        let mut evidence = ProjectCheckEvidence {
            actual_cwd,
            revision: None,
            branch: None,
            dirty: None,
            runtime: "native-read-only-checker-v1",
        };
        if !root
            .join(".git")
            .try_exists()
            .map_err(|error| error.to_string())?
        {
            return Ok(evidence);
        }
        let git = "/Library/Developer/CommandLineTools/usr/bin/git";
        let common = [
            "--no-optional-locks",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.untrackedCache=false",
            "-c",
            "core.hooksPath=/dev/null",
        ];
        let call = async |args: &[&str]| {
            let mut combined = common.to_vec();
            combined.extend_from_slice(args);
            run(&root, git, &combined).await
        };
        let toplevel = call(&["rev-parse", "--show-toplevel"]).await?;
        if Path::new(toplevel.trim_end()) != root {
            return Err("Git root differs from bound project".into());
        }
        let status = call(&[
            "status",
            "--porcelain=v2",
            "--branch",
            "--untracked-files=normal",
            "--ignore-submodules=all",
        ])
        .await?;
        let revision = status
            .lines()
            .find_map(|line| line.strip_prefix("# branch.oid "))
            .ok_or("Git did not report a revision state")?;
        if revision != "(initial)" {
            evidence.revision = Some(revision.into());
        }
        evidence.branch = Some(
            status
                .lines()
                .find_map(|line| line.strip_prefix("# branch.head "))
                .ok_or("Git did not report a branch state")?
                .into(),
        );
        evidence.dirty = Some(status.lines().any(|line| !line.starts_with("# ")));
        Ok(evidence)
    }
}

/// One changed path reported by `git status --porcelain=v1`.
#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct ChangedPath {
    /// Two-character porcelain status (`XY`), e.g. ` M`, `??`, `A `.
    pub status: String,
    /// Path relative to the project root.
    pub path: String,
}

/// Working-tree changes from the fixed, read-only checker.
#[derive(Debug, Serialize)]
pub struct ProjectChanges {
    /// Actual child process working directory.
    pub actual_cwd: String,
    /// Whether the root is a Git repository.
    pub repository: bool,
    /// Changed and untracked paths (bounded to 500 entries).
    pub changed_paths: Vec<ChangedPath>,
    /// Whether the path list was cut at the bound.
    pub paths_truncated: bool,
    /// `git diff --stat` for tracked changes.
    pub diff_stat: String,
    /// Unified diff of tracked changes, bounded.
    pub diff: String,
    /// Whether the diff was cut at the bound.
    pub diff_truncated: bool,
    /// Checker identity.
    pub runtime: &'static str,
}

/// Read the working-tree changes with the same fixed, sandboxed Git invocations
/// the inspection uses. Never writes, never runs hooks, never touches the network.
pub(crate) async fn changes(root: PathBuf) -> Result<ProjectChanges, String> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = root;
        Err("native project checker requires verified macOS sandbox support".into())
    }
    #[cfg(target_os = "macos")]
    {
        const DIFF_LIMIT: u64 = 512 * 1024;
        const MAX_PATHS: usize = 500;
        let root_text = root.to_str().ok_or("workspace path must be Unicode")?;
        crate::archive::channel_workspaces::validate_root(root_text)?;
        let actual_cwd = run(&root, "/bin/pwd", &["-P"])
            .await?
            .trim_end()
            .to_string();
        if Path::new(&actual_cwd) != root {
            return Err("checker working directory does not match current binding".into());
        }
        let mut result = ProjectChanges {
            actual_cwd,
            repository: false,
            changed_paths: Vec::new(),
            paths_truncated: false,
            diff_stat: String::new(),
            diff: String::new(),
            diff_truncated: false,
            runtime: "native-read-only-checker-v1",
        };
        if !root
            .join(".git")
            .try_exists()
            .map_err(|error| error.to_string())?
        {
            return Ok(result);
        }
        result.repository = true;
        let git = "/Library/Developer/CommandLineTools/usr/bin/git";
        let common = [
            "--no-optional-locks",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.untrackedCache=false",
            "-c",
            "core.hooksPath=/dev/null",
        ];
        let call = async |args: &[&str], limit: u64| {
            let mut combined = common.to_vec();
            combined.extend_from_slice(args);
            run_with_limit(&root, git, &combined, limit).await
        };
        let toplevel = call(&["rev-parse", "--show-toplevel"], 65536).await?.0;
        if Path::new(toplevel.trim_end()) != root {
            return Err("Git root differs from bound project".into());
        }
        let (status, status_truncated) = call(
            &[
                "status",
                "--porcelain=v1",
                "--untracked-files=normal",
                "--ignore-submodules=all",
            ],
            DIFF_LIMIT,
        )
        .await?;
        for line in status.lines() {
            if line.len() < 4 {
                continue;
            }
            if result.changed_paths.len() >= MAX_PATHS {
                result.paths_truncated = true;
                break;
            }
            result.changed_paths.push(ChangedPath {
                status: line[..2].to_string(),
                path: line[3..].to_string(),
            });
        }
        result.paths_truncated |= status_truncated;
        result.diff_stat = call(&["diff", "--no-color", "--stat=120"], DIFF_LIMIT)
            .await?
            .0;
        let (diff, diff_truncated) = call(&["diff", "--no-color"], DIFF_LIMIT).await?;
        result.diff = diff;
        result.diff_truncated = diff_truncated;
        Ok(result)
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn project_checker_uses_actual_cwd_and_rejects_escape_and_writes() {
        let directory = tempfile::tempdir().unwrap();
        let base = directory.path().canonicalize().unwrap();
        let root = base.join("project");
        std::fs::create_dir(&root).unwrap();
        let outside = base.join("private.txt");
        std::fs::write(&outside, "synthetic secret").unwrap();
        std::fs::write(root.join("visible.txt"), "allowed").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("escape")).unwrap();
        assert_eq!(
            inspect(root.clone()).await.unwrap().actual_cwd,
            root.to_str().unwrap()
        );
        assert_eq!(
            run(&root, "/bin/cat", &["visible.txt"]).await.unwrap(),
            "allowed"
        );
        assert!(run(&root, "/bin/cat", &["../private.txt"]).await.is_err());
        assert!(run(&root, "/bin/cat", &["escape"]).await.is_err());
        assert!(run(&root, "/usr/bin/touch", &["unauthorized"])
            .await
            .is_err());
        assert!(!root.join("unauthorized").exists());
        assert!(run(&root, "/bin/sh", &["-c", "/bin/cat visible.txt"])
            .await
            .is_err());
    }

    #[tokio::test]
    async fn project_checker_git_new_repo_and_configuration_cannot_execute() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let status = std::process::Command::new("/Library/Developer/CommandLineTools/usr/bin/git")
            .args(["init", "--quiet", "--initial-branch=main"])
            .arg(&root)
            .env_clear()
            .status()
            .unwrap();
        assert!(status.success());
        let clean = inspect(root.clone()).await.unwrap();
        assert_eq!(clean.branch.as_deref(), Some("main"));
        assert_eq!(clean.revision, None);
        assert_eq!(clean.dirty, Some(false));
        std::fs::write(root.join("uncommitted.txt"), "preserved work").unwrap();
        let config = root.join(".git/config");
        let mut contents = std::fs::read_to_string(&config).unwrap();
        contents.push_str("\n[core]\nfsmonitor = touch unauthorized\n");
        std::fs::write(config, contents).unwrap();
        assert_eq!(inspect(root.clone()).await.unwrap().dirty, Some(true));
        assert!(!root.join("unauthorized").exists());
    }
}
