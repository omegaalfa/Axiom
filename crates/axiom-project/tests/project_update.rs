use std::{fs, path::Path, str::FromStr};

use axiom_project::project_update::{
    ProjectUpdateCapability, ProjectUpdateError, TextFileFingerprint, MAX_UPDATE_TEXT_BYTES,
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
fn canonical_fingerprint_wire_round_trip_is_strict() {
    let fingerprint = TextFileFingerprint::from_bytes([0x2a; 32]);
    let wire = fingerprint.to_wire_string();
    assert_eq!(
        wire,
        format!("sha256:{}", "2a".repeat(32))
    );
    assert_eq!(TextFileFingerprint::from_str(&wire).unwrap(), fingerprint);
    for invalid in [
        String::new(),
        "2a".repeat(32),
        "sha256:".to_owned() + &"2A".repeat(32),
        "sha256:".to_owned() + &"2a".repeat(31),
        "sha256:".to_owned() + &"2a".repeat(33),
    ] {
        assert!(TextFileFingerprint::from_str(&invalid).is_err());
    }
}

#[test]
fn updates_existing_file_with_matching_fingerprint() {
    let workspace = tempdir().unwrap();
    fs::create_dir_all(workspace.path().join("src")).unwrap();
    fs::write(workspace.path().join("src/main.txt"), "old").unwrap();
    let capability = ProjectUpdateCapability::new(workspace.path()).unwrap();
    let expected = capability.fingerprint_text_file("src/main.txt").unwrap();

    let updated = capability
        .update_text_file("src/main.txt", expected, "new".into())
        .unwrap();

    assert_eq!(
        updated,
        fs::canonicalize(workspace.path().join("src/main.txt")).unwrap()
    );
    assert_eq!(fs::read_to_string(updated).unwrap(), "new");
}

#[test]
fn preserves_utf8_content() {
    let workspace = tempdir().unwrap();
    fs::write(workspace.path().join("unicode.txt"), "old").unwrap();
    let capability = ProjectUpdateCapability::new(workspace.path()).unwrap();
    let expected = capability.fingerprint_text_file("unicode.txt").unwrap();
    let content = "Olá, Axiom! 🚀\n".to_owned();

    capability
        .update_text_file("unicode.txt", expected, content.clone())
        .unwrap();

    assert_eq!(
        fs::read_to_string(workspace.path().join("unicode.txt")).unwrap(),
        content
    );
}

#[test]
fn rejects_stale_fingerprint_and_leaves_original_unchanged() {
    let workspace = tempdir().unwrap();
    fs::write(workspace.path().join("file.txt"), "original").unwrap();
    let capability = ProjectUpdateCapability::new(workspace.path()).unwrap();
    let stale = capability.fingerprint_text_file("file.txt").unwrap();
    fs::write(workspace.path().join("file.txt"), "changed").unwrap();
    let before = names(workspace.path());

    let error = capability
        .update_text_file("file.txt", stale, "replacement".into())
        .unwrap_err();

    assert!(matches!(
        error,
        ProjectUpdateError::FingerprintMismatch { .. }
    ));
    assert_eq!(
        fs::read_to_string(workspace.path().join("file.txt")).unwrap(),
        "changed"
    );
    assert_eq!(names(workspace.path()), before);
}

#[test]
fn rejects_missing_file_without_creating_it() {
    let workspace = tempdir().unwrap();
    let capability = ProjectUpdateCapability::new(workspace.path()).unwrap();

    assert!(matches!(
        capability.fingerprint_text_file("missing.txt"),
        Err(ProjectUpdateError::NotFound(_))
    ));
    assert!(matches!(
        capability.update_text_file(
            "missing.txt",
            TextFileFingerprint::from_bytes([0; 32]),
            "new".into()
        ),
        Err(ProjectUpdateError::NotFound(_))
    ));
    assert!(names(workspace.path()).is_empty());
}

#[test]
fn rejects_traversal_absolute_and_outside_paths() {
    let workspace = tempdir().unwrap();
    let outside = tempdir().unwrap();
    let capability = ProjectUpdateCapability::new(workspace.path()).unwrap();

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
            capability.update_text_file(
                &path,
                TextFileFingerprint::from_bytes([0; 32]),
                "unsafe".into()
            ),
            Err(ProjectUpdateError::InvalidPath(_) | ProjectUpdateError::OutsideWorkspace(_))
        ));
    }
    assert!(names(workspace.path()).is_empty());
}

#[test]
fn rejects_symlink_escape_where_supported() {
    let workspace = tempdir().unwrap();
    let outside = tempdir().unwrap();
    fs::write(outside.path().join("outside.txt"), "outside").unwrap();
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
    if created.is_err() {
        return;
    }
    let capability = ProjectUpdateCapability::new(workspace.path()).unwrap();

    assert!(matches!(
        capability.update_text_file(
            "escape.txt",
            TextFileFingerprint::from_bytes([0; 32]),
            "unsafe".into()
        ),
        Err(ProjectUpdateError::SymlinkNotAllowed(_))
    ));
    assert_eq!(fs::read_to_string(outside.path().join("outside.txt")).unwrap(), "outside");
}

#[test]
fn rejects_oversized_new_content_and_leaves_original_unchanged() {
    let workspace = tempdir().unwrap();
    fs::write(workspace.path().join("file.txt"), "original").unwrap();
    let capability = ProjectUpdateCapability::new(workspace.path()).unwrap();
    let expected = capability.fingerprint_text_file("file.txt").unwrap();
    let before = names(workspace.path());

    let error = capability
        .update_text_file(
            "file.txt",
            expected,
            "x".repeat(MAX_UPDATE_TEXT_BYTES + 1),
        )
        .unwrap_err();

    assert!(matches!(
        error,
        ProjectUpdateError::ContentTooLarge { limit, actual }
            if limit == MAX_UPDATE_TEXT_BYTES && actual == MAX_UPDATE_TEXT_BYTES + 1
    ));
    assert_eq!(
        fs::read_to_string(workspace.path().join("file.txt")).unwrap(),
        "original"
    );
    assert_eq!(names(workspace.path()), before);
}

#[test]
fn rejects_non_utf8_existing_file() {
    let workspace = tempdir().unwrap();
    fs::write(workspace.path().join("binary.txt"), [0xff, 0xfe]).unwrap();
    let capability = ProjectUpdateCapability::new(workspace.path()).unwrap();

    assert!(matches!(
        capability.fingerprint_text_file("binary.txt"),
        Err(ProjectUpdateError::UnsupportedEncoding(_))
    ));
    assert!(matches!(
        capability.update_text_file(
            "binary.txt",
            TextFileFingerprint::from_bytes([0; 32]),
            "new".into()
        ),
        Err(ProjectUpdateError::UnsupportedEncoding(_))
    ));
}

#[test]
fn rejects_oversized_current_file_without_reading_it_unbounded() {
    let workspace = tempdir().unwrap();
    fs::write(
        workspace.path().join("large.txt"),
        vec![b'x'; MAX_UPDATE_TEXT_BYTES + 1],
    )
    .unwrap();
    let capability = ProjectUpdateCapability::new(workspace.path()).unwrap();

    assert!(matches!(
        capability.fingerprint_text_file("large.txt"),
        Err(ProjectUpdateError::CurrentFileTooLarge { limit, .. })
            if limit == MAX_UPDATE_TEXT_BYTES
    ));
}
