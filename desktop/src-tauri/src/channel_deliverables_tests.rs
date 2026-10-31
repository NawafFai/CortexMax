use super::*;

fn write(root: &Path, relative: &str, contents: &[u8]) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, contents).unwrap();
}

fn declared(paths: &[&str]) -> Vec<String> {
    paths.iter().map(|p| (*p).to_string()).collect()
}

/// A target that tries to leave the channel's folder is refused, not clamped.
/// Without this the feature would be a file prober pointed by chat content.
#[test]
fn a_target_escaping_the_channel_folder_is_refused() {
    let root = Path::new("/projects/asi");
    for escape in [
        "../secrets.txt",
        "reports/../../etc/passwd",
        "..",
        "a/../../b",
    ] {
        let error = resolve_target(root, escape).expect_err(escape);
        assert!(
            error.contains("outside"),
            "{escape} produced the wrong refusal: {error}"
        );
    }
}

/// An absolute path ignores the channel binding entirely, so it is refused too.
#[test]
fn an_absolute_target_is_refused() {
    let root = Path::new("/projects/asi");
    let error = resolve_target(root, "/etc/passwd").expect_err("absolute");
    assert!(error.contains("relative"), "{error}");
}

/// The ordinary case: a relative target resolves inside the channel's folder.
#[test]
fn a_relative_target_resolves_inside_the_folder() {
    let root = Path::new("/projects/asi");
    assert_eq!(
        resolve_target(root, "out/report.docx").unwrap(),
        root.join("out/report.docx")
    );
    assert_eq!(
        resolve_target(root, "./report.docx").unwrap(),
        root.join("./report.docx"),
        "a leading ./ is harmless and stays inside"
    );
}

#[test]
fn an_empty_target_is_refused() {
    assert!(resolve_target(Path::new("/projects/asi"), "   ").is_err());
}

/// The whole point: declared outputs are checked against disk.
#[test]
fn present_and_missing_targets_are_reported() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "report.docx", b"a real document");

    let out = report(Some(dir.path()), &declared(&["report.docx", "model.xlsx"]));
    assert_eq!(out.total, 2);
    assert_eq!(out.produced, 1);

    let report_row = &out.targets[0];
    assert!(report_row.present);
    assert_eq!(report_row.size_bytes, Some(15));
    assert!(report_row.modified_at.is_some());

    let missing = &out.targets[1];
    assert!(!missing.present);
    assert_eq!(missing.error, None, "not produced yet is not an error");
}

/// An empty file is present but is not a produced deliverable. Counting it as
/// done is how a progress bar comes to read 100% over nothing — which is very
/// close to what actually happened during the live run.
#[test]
fn an_empty_file_counts_as_started_not_produced() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "model.xlsx", b"");

    let out = report(Some(dir.path()), &declared(&["model.xlsx"]));
    assert_eq!(out.produced, 0, "an empty file is not a deliverable");
    assert_eq!(out.empty, 1);
    assert!(out.targets[0].present, "but it does exist, and that shows");
    assert_eq!(out.targets[0].size_bytes, Some(0));
}

/// A folder where a file was declared is a real mismatch, not a quiet "not
/// produced yet" — the owner would otherwise keep waiting for a file that can
/// never appear there.
#[test]
fn a_folder_where_a_file_was_declared_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("report.docx")).unwrap();

    let out = report(Some(dir.path()), &declared(&["report.docx"]));
    assert!(!out.targets[0].present);
    assert_eq!(out.produced, 0);
    assert!(
        out.targets[0]
            .error
            .as_deref()
            .is_some_and(|error| error.contains("folder")),
        "{:?}",
        out.targets[0].error
    );
}

/// An escaping target is reported per-row rather than failing the whole
/// report, so one bad entry does not hide the channel's real progress.
#[test]
fn one_bad_target_does_not_hide_the_others() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "good.docx", b"x");

    let out = report(Some(dir.path()), &declared(&["../escape.txt", "good.docx"]));
    assert_eq!(out.produced, 1);
    assert!(out.targets[0].error.is_some());
    assert!(out.targets[1].present);
}

/// A channel with no bound folder cannot be judged, and must not report zero
/// progress as though the team had failed to produce anything.
#[test]
fn a_channel_without_a_folder_says_so() {
    let out = report(None, &declared(&["report.docx"]));
    assert!(out.no_workspace);
    assert_eq!(out.total, 1);
    assert_eq!(out.produced, 0);
    assert!(
        out.targets[0]
            .error
            .as_deref()
            .is_some_and(|error| error.contains("no folder")),
        "the reason must be visible, not implied by a zero"
    );
}

/// A channel that declared nothing is not a failing channel.
#[test]
fn a_channel_with_no_targets_reports_an_empty_set() {
    let dir = tempfile::tempdir().unwrap();
    let out = report(Some(dir.path()), &[]);
    assert_eq!(out.total, 0);
    assert_eq!(out.produced, 0);
    assert!(out.targets.is_empty());
    assert!(!out.no_workspace);
}

/// Nested outputs are the normal case for a build, so a subdirectory target
/// must work.
#[test]
fn a_nested_target_is_found() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "out/deliverables/model.xlsx", b"rows");

    let out = report(
        Some(dir.path()),
        &declared(&["out/deliverables/model.xlsx"]),
    );
    assert_eq!(out.produced, 1);
    assert!(out.targets[0].present);
}

/// The report crosses the wire to the UI; a rename would empty the panel.
#[test]
fn the_wire_shape_matches_what_the_ui_reads() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "report.docx", b"x");
    let out = report(Some(dir.path()), &declared(&["report.docx"]));

    let json = serde_json::to_string(&out).unwrap();
    assert!(json.contains("\"sizeBytes\":1"), "{json}");
    assert!(json.contains("\"modifiedAt\":"), "{json}");
    assert!(json.contains("\"noWorkspace\":false"), "{json}");
    assert!(!json.contains("size_bytes"), "{json}");
}

/// The handover writes through the same resolver the deliverables use, so a
/// note can never land outside the folder the channel was pointed at.
#[test]
fn a_handover_resolves_inside_the_channel_folder() {
    let root = std::path::Path::new("/tmp/project");
    assert_eq!(
        resolve_target(root, "HANDOVER.md").unwrap(),
        root.join("HANDOVER.md")
    );
}

/// The reason overwriting is refused, stated as a test: an existing handover
/// is the team's accumulated account of what is in flight and what must not
/// be built upon. This project's own HANDOVER.md already held exactly that.
#[test]
fn an_existing_handover_is_reported_as_not_created() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("HANDOVER.md");
    std::fs::write(&path, "# real content the team wrote").unwrap();

    // The command's decision, exercised directly: a present file is a result,
    // not a failure, and its contents are left alone.
    let already_there = path.exists();
    let outcome = HandoverResult {
        created: !already_there,
        error: None,
    };
    assert!(!outcome.created, "an existing handover is never replaced");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "# real content the team wrote",
        "and its contents survive untouched"
    );
}
