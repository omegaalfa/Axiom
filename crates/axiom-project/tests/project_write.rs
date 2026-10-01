use std::{fs, path::Path};

use axiom_project::project_write::{
    ProjectWriteCapability, ProjectWriteError, MAX_CREATE_TEXT_BYTES,
};
use tempfile::tempdir;

fn names(path: &Path) -> Vec<String> {
    let mut names = fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    names.sort();
    names
}

#[test]
fn creates_utf8_file_in_existing_nested_directory() {
    let workspace = tempdir().unwrap();
    fs::create_dir_all(workspace.path().join("src/pages")).unwrap();
    let capability = ProjectWriteCapability::new(workspace.path()).unwrap();
    let content = "Olá, Axiom! 🚀\n".to_owned();

    let created = capability
        .create_text_file("src/pages/index.html", content.clone())
        .unwrap();

    assert_eq!(
        created,
        fs::canonicalize(workspace.path().join("src/pages/index.html")).unwrap()
    );
    assert_eq!(fs::read_to_string(created).unwrap(), content);
}

#[test]
fn existing_destination_is_rejected_and_failure_leaves_no_artifacts() {
    let workspace = tempdir().unwrap();
    let destination = workspace.path().join("existing.txt");
    fs::write(&destination, "original").unwrap();
    let before = names(workspace.path());
    let capability = ProjectWriteCapability::new(workspace.path()).unwrap();

    let error = capability
        .create_text_file("existing.txt", "replacement".into())
        .unwrap_err();

    assert!(matches!(error, ProjectWriteError::AlreadyExists(_)));
    assert_eq!(fs::read_to_string(destination).unwrap(), "original");
    assert_eq!(names(workspace.path()), before);
}

#[test]
fn rejects_traversal_absolute_and_outside_paths() {
    let workspace = tempdir().unwrap();
    let outside = tempdir().unwrap();
    let capability = ProjectWriteCapability::new(workspace.path()).unwrap();

    for path in [
        "../escape.txt".to_owned(),
        "nested/../../escape.txt".to_owned(),
        outside
            .path()
            .join("escape.txt")
            .to_string_lossy()
            .into_owned(),
    ] {
        assert!(matches!(
            capability.create_text_file(&path, "unsafe".into()),
            Err(ProjectWriteError::InvalidPath(_) | ProjectWriteError::OutsideWorkspace(_))
        ));
    }
    assert!(names(workspace.path()).is_empty());
}

#[test]
fn rejects_symlink_escape_where_supported() {
    let workspace = tempdir().unwrap();
    let outside = tempdir().unwrap();
    let link = workspace.path().join("escape");
    let created = {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(outside.path(), &link)
        }
        #[cfg(windows)]
        {
            std::os::windows::fs::symlink_dir(outside.path(), &link)
        }
        #[cfg(not(any(unix, windows)))]
        {
            return;
        }
    };
    if created.is_err() {
        return;
    }

    let capability = ProjectWriteCapability::new(workspace.path()).unwrap();
    assert!(matches!(
        capability.create_text_file("escape/escape.txt", "unsafe".into()),
        Err(ProjectWriteError::SymlinkNotAllowed(_))
    ));
    assert_eq!(names(outside.path()), Vec::<String>::new());
}

#[test]
fn enforces_content_size_limit() {
    let workspace = tempdir().unwrap();
    let capability = ProjectWriteCapability::new(workspace.path()).unwrap();

    let exact = "x".repeat(MAX_CREATE_TEXT_BYTES);
    capability.create_text_file("exact.txt", exact).unwrap();

    let oversized = "x".repeat(MAX_CREATE_TEXT_BYTES + 1);
    assert!(matches!(
        capability.create_text_file("oversized.txt", oversized),
        Err(ProjectWriteError::ContentTooLarge { limit, actual })
            if limit == MAX_CREATE_TEXT_BYTES && actual == MAX_CREATE_TEXT_BYTES + 1
    ));
    assert_eq!(
        names(workspace.path()),
        vec!["exact.txt".to_owned()]
    );
}

#[test]
fn failed_publication_leaves_no_temporary_artifact() {
    let workspace = tempdir().unwrap();
    let blocked = workspace.path().join("blocked");
    fs::create_dir(&blocked).unwrap();
    let capability = ProjectWriteCapability::new(workspace.path()).unwrap();

    let error = capability
        .create_text_file("blocked", "cannot replace a directory".into())
        .unwrap_err();

    assert!(matches!(
        error,
        ProjectWriteError::AlreadyExists(_) | ProjectWriteError::Io { .. }
    ));
    assert!(blocked.is_dir());
    assert_eq!(names(workspace.path()), vec!["blocked".to_owned()]);
}
