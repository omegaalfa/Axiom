use std::{fs, str::FromStr};

use axiom_project::{
    project_delete::{ProjectDeleteCapability, ProjectDeleteError},
    project_update::{ProjectUpdateCapability, TextFileFingerprint},
};
use tempfile::tempdir;

#[test]
fn matching_fingerprint_deletes_one_regular_file() {
    let workspace = tempdir().unwrap();
    fs::write(workspace.path().join("remove.txt"), "remove me").unwrap();
    let fingerprint = ProjectUpdateCapability::new(workspace.path())
        .unwrap()
        .fingerprint_text_file("remove.txt")
        .unwrap();
    let expected_path = fs::canonicalize(workspace.path().join("remove.txt")).unwrap();
    let deleted = ProjectDeleteCapability::new(workspace.path())
        .unwrap()
        .delete_file("remove.txt", fingerprint)
        .unwrap();

    assert_eq!(deleted, expected_path);
    assert!(!workspace.path().join("remove.txt").exists());
}

#[test]
fn stale_or_missing_fingerprint_never_deletes() {
    let workspace = tempdir().unwrap();
    fs::write(workspace.path().join("keep.txt"), "original").unwrap();
    let capability = ProjectDeleteCapability::new(workspace.path()).unwrap();
    let stale = TextFileFingerprint::from_bytes([0; 32]);

    assert!(matches!(
        capability.delete_file("keep.txt", stale),
        Err(ProjectDeleteError::FingerprintMismatch { .. })
    ));
    assert!(workspace.path().join("keep.txt").exists());
    assert!(matches!(
        capability.delete_file("missing.txt", stale),
        Err(ProjectDeleteError::NotFound(_))
    ));
    assert!(TextFileFingerprint::from_str("invalid").is_err());
}

#[test]
fn rejects_directories_traversal_outside_and_symlink_escape() {
    let workspace = tempdir().unwrap();
    let outside = tempdir().unwrap();
    fs::create_dir(workspace.path().join("folder")).unwrap();
    fs::write(outside.path().join("outside.txt"), "outside").unwrap();
    let capability = ProjectDeleteCapability::new(workspace.path()).unwrap();
    let fingerprint = TextFileFingerprint::from_bytes([0; 32]);

    assert!(matches!(
        capability.delete_file("folder", fingerprint),
        Err(ProjectDeleteError::NotRegularFile(_))
    ));
    assert!(matches!(
        capability.delete_file("../outside.txt", fingerprint),
        Err(ProjectDeleteError::InvalidPath(_))
    ));
    assert!(matches!(
        capability.delete_file(
            &outside.path().join("outside.txt").to_string_lossy(),
            fingerprint
        ),
        Err(ProjectDeleteError::InvalidPath(_) | ProjectDeleteError::OutsideWorkspace(_))
    ));

    let link = workspace.path().join("escape.txt");
    let created = {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(outside.path().join("outside.txt"), &link)
        }
        #[cfg(windows)]
        {
            std::os::windows::fs::symlink_file(outside.path().join("outside.txt"), &link)
        }
        #[cfg(not(any(unix, windows)))]
        {
            return;
        }
    };
    if created.is_ok() {
        assert!(matches!(
            capability.delete_file("escape.txt", fingerprint),
            Err(ProjectDeleteError::SymlinkNotAllowed(_))
        ));
        assert!(outside.path().join("outside.txt").exists());
    }
}
