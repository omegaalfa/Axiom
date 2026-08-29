use axiom_index::ProjectSymbolIndex;
use std::fs;

fn write_php(path: &std::path::Path, class_name: &str) {
    fs::create_dir_all(path.parent().expect("fixture parent")).unwrap();
    fs::write(path, format!("<?php\nclass {class_name} {{}}\n")).unwrap();
}

#[test]
fn warm_cache_preserves_hit_and_tracks_changed_new_and_removed_files() {
    let project = tempfile::tempdir().unwrap();
    let cache = project.path().join(".cache/project.json");
    let src = project.path().join("src/Existing.php");
    let new_file = project.path().join("src/New.php");
    write_php(&src, "Existing");

    let mut index = ProjectSymbolIndex::new();
    let first = index.index_project_cached(project.path(), &cache).unwrap();
    assert_eq!(first.files, 1);
    assert!(index.find_class("Existing").is_some());

    // A metadata-identical second scan must reuse the cached declaration.
    let mut warm = ProjectSymbolIndex::new();
    let hit = warm.index_project_cached(project.path(), &cache).unwrap();
    assert_eq!(hit.files, 1);
    assert!(warm.find_class("Existing").is_some());

    // Changing the file must invalidate only that cache entry.
    write_php(&src, "Changed");
    let mut changed = ProjectSymbolIndex::new();
    let changed_report = changed
        .index_project_cached(project.path(), &cache)
        .unwrap();
    assert_eq!(changed_report.files, 1);
    assert!(changed.find_class("Existing").is_none());
    assert!(changed.find_class("Changed").is_some());

    // A newly discovered PHP file is parsed and added to the cache.
    write_php(&new_file, "NewClass");
    let mut added = ProjectSymbolIndex::new();
    let added_report = added.index_project_cached(project.path(), &cache).unwrap();
    assert_eq!(added_report.files, 2);
    assert!(added.find_class("NewClass").is_some());

    // A file absent from discovery is removed from the rebuilt index.
    fs::remove_file(&src).unwrap();
    let mut removed = ProjectSymbolIndex::new();
    let removed_report = removed
        .index_project_cached(project.path(), &cache)
        .unwrap();
    assert_eq!(removed_report.files, 1);
    assert!(removed.find_class("Changed").is_none());
    assert!(removed.find_class("NewClass").is_some());
}

#[test]
fn warm_cache_accepts_dot_segment_root_and_keeps_discovery_exclusions() {
    let project = tempfile::tempdir().unwrap();
    let cache = project.path().join(".cache/project.json");
    write_php(&project.path().join("src/User.php"), "User");
    write_php(&project.path().join("vendor/pkg/Vendor.php"), "VendorClass");
    write_php(&project.path().join("target/generated.php"), "TargetClass");
    write_php(
        &project.path().join("node_modules/pkg/Node.php"),
        "NodeClass",
    );
    write_php(&project.path().join(".git/Hidden.php"), "HiddenClass");

    // Keep the root spelling lexical here: callers may provide dot segments.
    let dotted_root = project.path().join("src").join("..");
    let mut index = ProjectSymbolIndex::new();
    let report = index.index_project_cached(&dotted_root, &cache).unwrap();
    assert_eq!(report.files, 1);
    assert!(index.find_class("User").is_some());
    assert!(index.find_class("VendorClass").is_none());
    assert!(index.find_class("TargetClass").is_none());
    assert!(index.find_class("NodeClass").is_none());
    assert!(index.find_class("HiddenClass").is_none());
}

#[cfg(unix)]
#[test]
fn discovery_does_not_follow_symlinked_directories_or_cycles() {
    use std::os::unix::fs::symlink;

    let project = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    write_php(&project.path().join("src/Real.php"), "Real");
    write_php(&outside.path().join("Outside.php"), "Outside");
    symlink(
        project.path().join("src"),
        project.path().join("linked-src"),
    )
    .unwrap();
    symlink(outside.path(), project.path().join("external-link")).unwrap();
    symlink(project.path(), project.path().join("src/cycle")).unwrap();

    let mut index = ProjectSymbolIndex::new();
    let report = index
        .index_project_cached(project.path(), project.path().join("cache.json"))
        .unwrap();
    assert_eq!(report.files, 1);
    assert!(index.find_class("Real").is_some());
    assert!(index.find_class("Outside").is_none());
}

#[cfg(windows)]
#[test]
fn discovery_does_not_follow_windows_reparse_directories() {
    use std::os::windows::fs::symlink_dir;

    let project = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    write_php(&project.path().join("src/Real.php"), "Real");
    write_php(&outside.path().join("Outside.php"), "Outside");
    // Symlink creation may be disabled for non-developer test accounts. In
    // that environment the platform-specific traversal assertion is skipped.
    if symlink_dir(outside.path(), project.path().join("external-link")).is_err() {
        return;
    }
    let mut index = ProjectSymbolIndex::new();
    let report = index
        .index_project_cached(project.path(), project.path().join("cache.json"))
        .unwrap();
    assert_eq!(report.files, 1);
    assert!(index.find_class("Real").is_some());
    assert!(index.find_class("Outside").is_none());
}
