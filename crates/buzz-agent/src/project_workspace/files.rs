use std::path::{Component, Path};

pub(super) fn validate_relative(relative: &str) -> Result<(), String> {
    if relative.len()>4096 || relative.is_empty() || relative.as_bytes().contains(&0) || Path::new(relative).components().any(|component| !matches!(component,Component::Normal(name) if !name.to_string_lossy().starts_with('.'))) {
        return Err("project tools reject absolute, traversal, hidden, and oversized paths".into());
    }
    Ok(())
}

pub(super) fn absent(root: &str, relative: &str) -> Result<bool, String> {
    validate_relative(relative)?;
    #[cfg(not(unix))]
    {
        let _ = root;
        Err("native absence inspection is unsupported on this platform".into())
    }
    #[cfg(unix)]
    {
        use rustix::{
            fs::{open, openat, statat, AtFlags, Mode, OFlags},
            io::Errno,
        };
        if !Path::new(root).is_absolute() {
            return Err("project root must be absolute".into());
        }
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let mut directory = open("/", flags, Mode::empty()).map_err(|error| error.to_string())?;
        for component in Path::new(root).components() {
            match component {
                Component::RootDir => {}
                Component::Normal(name) => {
                    directory = openat(&directory, name, flags, Mode::empty())
                        .map_err(|error| format!("project root unavailable: {error}"))?
                }
                _ => return Err("invalid project root".into()),
            }
        }
        let parts: Vec<_> = Path::new(relative).components().collect();
        for (index, component) in parts.iter().enumerate() {
            let Component::Normal(name) = component else {
                return Err("invalid project file path".into());
            };
            if index + 1 == parts.len() {
                return match statat(&directory, *name, AtFlags::SYMLINK_NOFOLLOW) {
                    Ok(_) => Ok(false),
                    Err(Errno::NOENT) => Ok(true),
                    Err(error) => Err(error.to_string()),
                };
            }
            directory = match openat(&directory, *name, flags, Mode::empty()) {
                Ok(directory) => directory,
                Err(Errno::NOENT) => return Ok(true),
                Err(error) => return Err(error.to_string()),
            };
        }
        Err("missing project file path".into())
    }
}

pub(super) fn open_regular(
    root: &str,
    relative: &str,
    write: bool,
    create: bool,
) -> Result<std::fs::File, String> {
    #[cfg(not(unix))]
    {
        let _ = (root, relative, write, create);
        Err("native bounded filesystem tools are unsupported on this platform".into())
    }
    #[cfg(unix)]
    {
        use rustix::fs::{fstat, open, openat, FileType, Mode, OFlags};
        validate_relative(relative)?;
        let parts: Vec<_> = Path::new(relative).components().collect();
        if parts.iter().any(|component| !matches!(component,Component::Normal(name) if !name.to_string_lossy().starts_with('.'))) {
            return Err("project tools reject absolute, traversal, and hidden paths".into());
        }
        let mut directory = open(
            "/",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|error| error.to_string())?;
        if !Path::new(root).is_absolute() {
            return Err("project root must be absolute".into());
        }
        for component in Path::new(root).components() {
            match component {
                Component::RootDir => {}
                Component::Normal(name) => {
                    directory = openat(
                        &directory,
                        name,
                        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                        Mode::empty(),
                    )
                    .map_err(|error| format!("project root is unavailable or symlinked: {error}"))?
                }
                _ => return Err("invalid project root".into()),
            }
        }
        for (index, component) in parts.iter().enumerate() {
            let Component::Normal(name) = component else {
                return Err("invalid path".into());
            };
            if index + 1 == parts.len() {
                let mode = if create {
                    OFlags::RDWR | OFlags::CREATE | OFlags::EXCL
                } else if write {
                    OFlags::RDWR
                } else {
                    OFlags::RDONLY
                };
                let descriptor = openat(
                    &directory,
                    *name,
                    mode | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                    Mode::RUSR | Mode::WUSR,
                )
                .map_err(|error| error.to_string())?;
                let stat = fstat(&descriptor).map_err(|error| error.to_string())?;
                if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile
                    || stat.st_nlink != 1
                    || stat.st_size > 65536
                {
                    return Err(
                        "only bounded, non-hardlinked regular project text files may be read"
                            .into(),
                    );
                }
                return Ok(std::fs::File::from(descriptor));
            }
            directory = openat(
                &directory,
                *name,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| error.to_string())?;
        }
        Err("missing project file".into())
    }
}

pub(super) fn hash(content: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(content.as_bytes()))
}

pub(super) fn read(root: &str, relative: &str) -> Result<String, String> {
    use std::io::Read;
    let mut content = String::new();
    open_regular(root, relative, false, false)?
        .take(65537)
        .read_to_string(&mut content)
        .map_err(|error| error.to_string())?;
    if content.len() > 65536 {
        return Err("project file exceeds read limit".into());
    }
    Ok(serde_json::json!({"path":relative,"sha256":hash(&content),"content":content}).to_string())
}

pub(super) fn replace(
    root: &str,
    relative: &str,
    expected: &str,
    replacement: &str,
) -> Result<String, String> {
    use std::io::{Read, Seek, SeekFrom, Write};
    if replacement.len() > 65536
        || expected.len() != 64
        || !expected.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("invalid bounded replacement or expected file hash".into());
    }
    let mut file = open_regular(root, relative, true, false)?;
    let mut content = String::new();
    (&mut file)
        .take(65537)
        .read_to_string(&mut content)
        .map_err(|error| error.to_string())?;
    if content.len() > 65536 || hash(&content) != expected {
        return Err("project file changed; read and review the current file before editing".into());
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|error| error.to_string())?;
    file.write_all(replacement.as_bytes())
        .map_err(|error| error.to_string())?;
    file.set_len(replacement.len() as u64)
        .map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    Ok(serde_json::json!({"path":relative,"sha256":hash(replacement),"state":"edited_not_reviewed"}).to_string())
}

pub(super) fn create(root: &str, relative: &str, content: &str) -> Result<String, String> {
    use std::io::Write;
    if content.len() > 65536 {
        return Err("new project file exceeds text limit".into());
    }
    let mut file = open_regular(root, relative, true, true)?;
    file.write_all(content.as_bytes())
        .map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    Ok(
        serde_json::json!({"path":relative,"sha256":hash(content),"state":"created_not_reviewed"})
            .to_string(),
    )
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn native_project_absence_and_file_guards_do_not_follow_links_or_clobber() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let root = root.to_str().unwrap();
        assert!(absent(root, "missing/file.txt").unwrap());
        create(root, "new.txt", "preserved").unwrap();
        assert!(!absent(root, "new.txt").unwrap());
        assert!(create(root, "new.txt", "overwrite").is_err());
        assert!(read(root, "new.txt").unwrap().contains("preserved"));
        std::os::unix::fs::symlink("missing", Path::new(root).join("link")).unwrap();
        assert!(!absent(root, "link").unwrap());
        assert!(absent(root, "link/file.txt").is_err());
        assert!(absent(root, "../outside.txt").is_err());
        assert!(create(root, ".git", "forbidden").is_err());
        assert!(!Path::new(root).join(".git").exists());
        std::fs::hard_link(
            Path::new(root).join("new.txt"),
            Path::new(root).join("shared.txt"),
        )
        .unwrap();
        assert!(replace(root, "new.txt", &hash("preserved"), "must not change").is_err());
        assert_eq!(
            std::fs::read_to_string(Path::new(root).join("shared.txt")).unwrap(),
            "preserved"
        );
    }
}
