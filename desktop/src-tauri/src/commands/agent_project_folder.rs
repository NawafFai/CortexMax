//! The folder an agent asks to start a project in, checked before the owner
//! is asked to confirm it.

use std::path::{Path, PathBuf};

use crate::archive::channel_workspaces::validate_root;

/// Where an agent-requested project may live.
///
/// A project the owner starts himself gets its folder from the system picker.
/// A folder named in an agent's request never passed through that picker, so
/// it is held to a narrower rule on top of the picker's own (absolute,
/// canonical, an existing readable directory): inside the owner's home, below
/// it rather than the home itself, and clear of the places that hold
/// credentials and app state — `~/Library`, and every hidden folder (`.ssh`,
/// `.codex`, `.claude`, and `.cortex-max`, where the agents' own identity
/// keys live). A project root is where agents are given read and write access;
/// none of those is a place to grant that by request. The owner still sees
/// the path and confirms it.
pub(crate) fn agent_project_folder(home: &Path, raw: &str) -> Result<PathBuf, String> {
    let root = validate_root(raw)?;
    let home = home
        .canonicalize()
        .map_err(|error| format!("home folder unavailable: {error}"))?;
    let inside = root
        .strip_prefix(&home)
        .map_err(|_| "an agent can only propose a folder inside your home folder".to_string())?;
    let mut components = inside.components();
    let first = components
        .next()
        .ok_or("an agent cannot propose your whole home folder as a project")?;
    if first.as_os_str() == "Library" {
        return Err("an agent cannot propose a folder under ~/Library".into());
    }
    if inside
        .components()
        .any(|component| component.as_os_str().to_string_lossy().starts_with('.'))
    {
        return Err(
            "an agent cannot propose a hidden folder, or one inside a hidden folder".into(),
        );
    }
    Ok(root)
}

/// Check the folder in an agent's project request, returning it canonical.
#[tauri::command]
pub async fn check_agent_project_folder(path: String) -> Result<String, String> {
    let home = dirs::home_dir().ok_or("home folder unavailable")?;
    agent_project_folder(&home, &path).map(|root| root.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        (dir, home)
    }

    fn folder(home: &Path, relative: &str) -> String {
        let path = home.join(relative);
        std::fs::create_dir_all(&path).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn a_project_folder_under_home_is_accepted_as_is() {
        let (_dir, home) = home();
        let path = folder(&home, "Desktop/Demo App - v2");
        let root = agent_project_folder(&home, &path).unwrap();
        assert_eq!(root, PathBuf::from(&path));
    }

    #[test]
    fn the_home_folder_itself_is_refused() {
        let (_dir, home) = home();
        let error = agent_project_folder(&home, &home.to_string_lossy()).unwrap_err();
        assert!(error.contains("whole home folder"), "{error}");
    }

    #[test]
    fn library_and_hidden_folders_are_refused() {
        let (_dir, home) = home();
        for refused in [
            "Library/Application Support/com.cortexmax.app",
            ".cortex-max",
            ".buzz",
            ".ssh",
            "Developer/.git",
            ".codex/sessions",
        ] {
            let path = folder(&home, refused);
            assert!(
                agent_project_folder(&home, &path).is_err(),
                "{refused} must be refused"
            );
        }
    }

    #[test]
    fn a_folder_outside_home_is_refused() {
        let (_dir, home) = home();
        let (_elsewhere, outside) = self::home();
        let path = folder(&outside, "project");
        let error = agent_project_folder(&home, &path).unwrap_err();
        assert!(error.contains("inside your home folder"), "{error}");
    }

    #[test]
    fn the_pickers_own_rules_still_apply() {
        let (_dir, home) = home();
        let missing = home.join("Desktop/nope").to_string_lossy().into_owned();
        assert!(agent_project_folder(&home, &missing).is_err());
        let traversal = format!("{}/Desktop/../Desktop", home.display());
        folder(&home, "Desktop");
        assert!(agent_project_folder(&home, &traversal).is_err());
        assert!(agent_project_folder(&home, "Desktop/relative").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_to_a_folder_is_refused() {
        let (_dir, home) = home();
        let real = folder(&home, "Desktop/real");
        let link = home.join("Desktop/link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(agent_project_folder(&home, &link.to_string_lossy()).is_err());
    }
}
