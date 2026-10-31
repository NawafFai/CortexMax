use std::path::{Component, Path};

pub(crate) fn create_folder(parent: &str, name: &str) -> Result<String, String> {
    if name.is_empty()
        || name.trim() != name
        || name.len() > 128
        || name.contains(['/', '\\', '\0'])
        || !matches!(
            Path::new(name).components().next(),
            Some(Component::Normal(_))
        )
    {
        return Err("choose a single non-empty folder name without path separators".into());
    }
    let root = super::channel_workspaces::validate_root(parent)?;
    #[cfg(unix)]
    {
        use rustix::fs::{mkdirat, open, openat, Mode, OFlags};
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let mut directory = open("/", flags, Mode::empty()).map_err(|error| error.to_string())?;
        for component in root.components() {
            if let Component::Normal(segment) = component {
                directory = openat(&directory, segment, flags, Mode::empty()).map_err(|error| {
                    format!("project parent changed or is unavailable: {error}")
                })?;
            }
        }
        mkdirat(&directory, name, Mode::RWXU).map_err(|error| {
            format!("folder was not created; existing files are never replaced: {error}")
        })?;
        let created = root.join(name);
        let raw = created.to_str().ok_or("folder path must be Unicode")?;
        super::channel_workspaces::validate_root(raw).map_err(|error| {
            format!("folder creation outcome needs inspection; no cleanup performed: {error}")
        })?;
        Ok(raw.to_owned())
    }
    #[cfg(not(unix))]
    {
        let _ = root;
        Err("safe project directory creation is not implemented on this platform; attach an existing folder".into())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn channel_workspaces_new_folder_never_overwrites_or_escapes() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let parent = root.to_str().unwrap();
        let created = create_folder(parent, "project-a").unwrap();
        let evidence = Path::new(&created).join("evidence");
        std::fs::write(&evidence, "preserved").unwrap();
        for invalid in [
            "project-a",
            ".",
            "..",
            "../escape",
            "a/b",
            "a\\b",
            "",
            "/escape",
        ] {
            assert!(create_folder(parent, invalid).is_err(), "{invalid}");
        }
        std::os::unix::fs::symlink(&created, root.join("link")).unwrap();
        assert!(create_folder(root.join("link").to_str().unwrap(), "outside").is_err());
        assert_eq!(std::fs::read_to_string(evidence).unwrap(), "preserved");
        assert!(!Path::new(&created).join("outside").exists());
    }
}
