use axiom_index::{
    ProjectSymbolIndex, ReferenceRole, ReferenceTarget, SemanticRevision, SemanticSnapshot,
    SnapshotBuilder, SymbolId,
};
use std::{fs, path::Path};

fn snapshot_from_text(path: &Path, source: &str) -> SemanticSnapshot {
    fs::write(path, source).unwrap();
    let mut index = ProjectSymbolIndex::new();
    index.index_file(path).unwrap();
    SemanticSnapshot::from_project_index(&index, SemanticRevision(1))
}

fn symbol(snapshot: &SemanticSnapshot, fqn: &str) -> SymbolId {
    snapshot.symbols_for_fqn(fqn)[0]
}

fn names(snapshot: &SemanticSnapshot, symbols: &[SymbolId]) -> Vec<String> {
    symbols
        .iter()
        .map(|id| snapshot.symbol(*id).unwrap().fully_qualified_name.clone())
        .collect()
}

#[test]
fn direct_function_call_is_materialized() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("calls.php");
    let snapshot = snapshot_from_text(
        &path,
        "<?php function b() {} function a() { b(); }",
    );
    let caller = symbol(&snapshot, "a");
    let callee = symbol(&snapshot, "b");

    assert_eq!(snapshot.direct_callees(caller), &[callee]);
}

#[test]
fn multiple_direct_callees_are_materialized() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("calls.php");
    let snapshot = snapshot_from_text(
        &path,
        "<?php function b() {} function c() {} function a() { b(); c(); }",
    );
    let caller = symbol(&snapshot, "a");
    let mut expected = vec![symbol(&snapshot, "b"), symbol(&snapshot, "c")];
    expected.sort_unstable();

    assert_eq!(snapshot.direct_callees(caller), expected);
}

#[test]
fn repeated_direct_call_is_deduplicated() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("calls.php");
    let snapshot = snapshot_from_text(
        &path,
        "<?php function b() {} function a() { b(); b(); }",
    );
    let caller = symbol(&snapshot, "a");
    let callee = symbol(&snapshot, "b");

    assert_eq!(snapshot.direct_callees(caller), &[callee]);
}

#[test]
fn call_cycle_is_materialized_without_recursive_traversal() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("calls.php");
    let snapshot = snapshot_from_text(
        &path,
        "<?php function a() { b(); } function b() { a(); }",
    );
    let a = symbol(&snapshot, "a");
    let b = symbol(&snapshot, "b");

    assert_eq!(snapshot.direct_callees(a), &[b]);
    assert_eq!(snapshot.direct_callees(b), &[a]);
}

#[test]
fn unresolved_dynamic_candidate_and_deferred_calls_are_excluded() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("calls.php");
    let snapshot = snapshot_from_text(
        &path,
        r#"<?php
function dup() {}
function dup() {}
function excluded() {
    $callback();
    missing();
    \Missing\missing();
    dup();
}
"#,
    );
    let caller = symbol(&snapshot, "excluded");
    assert!(snapshot.direct_callees(caller).is_empty());

    let file = snapshot.symbol(caller).unwrap().file;
    let references: Vec<_> = snapshot
        .references_for_file(file)
        .iter()
        .filter_map(|id| snapshot.reference(*id))
        .filter(|reference| {
            reference.source_symbol == Some(caller)
                && reference.role == ReferenceRole::FunctionCall
        })
        .collect();
    assert_eq!(references.len(), 4);
    assert!(
        references
            .iter()
            .any(|reference| matches!(&reference.target, ReferenceTarget::Dynamic))
    );
    assert!(
        references
            .iter()
            .any(|reference| matches!(&reference.target, ReferenceTarget::Unresolved))
    );
    assert!(
        references
            .iter()
            .any(|reference| matches!(&reference.target, ReferenceTarget::Deferred))
    );
    assert!(
        references
            .iter()
            .any(|reference| matches!(&reference.target, ReferenceTarget::Candidates(_)))
    );
}

#[test]
fn resolved_function_static_and_method_calls_are_included() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("calls.php");
    let snapshot = snapshot_from_text(
        &path,
        r#"<?php
function b() {}
class Repo {
    static function run() {}
    function save() {}
}
function a(Repo $repo) {
    b();
    Repo::run();
    $repo->save();
}
"#,
    );
    let caller = symbol(&snapshot, "a");
    let mut expected = vec![
        symbol(&snapshot, "b"),
        symbol(&snapshot, "Repo::run"),
        symbol(&snapshot, "Repo::save"),
    ];
    expected.sort_unstable();

    assert_eq!(snapshot.direct_callees(caller), expected);
}

#[test]
fn callable_without_calls_returns_empty_slice() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("calls.php");
    let snapshot = snapshot_from_text(&path, "<?php function a() {}");

    assert!(snapshot.direct_callees(symbol(&snapshot, "a")).is_empty());
}

#[test]
fn snapshot_rebuild_removes_stale_direct_callees() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("calls.php");
    let first_source = "<?php function b() {} function c() {} function a() { b(); }";
    let snapshot = snapshot_from_text(&path, first_source);
    let caller = symbol(&snapshot, "a");
    let old_callee = symbol(&snapshot, "b");
    assert_eq!(snapshot.direct_callees(caller), &[old_callee]);

    let mut builder = SnapshotBuilder::from_snapshot(&snapshot);
    builder.replace_workspace_file(
        path.canonicalize().unwrap(),
        "<?php function b() {} function c() {} function a() { c(); }",
    );
    let rebuilt = builder.finish();
    let rebuilt_caller = symbol(&rebuilt, "a");
    let new_callee = symbol(&rebuilt, "c");

    assert_eq!(rebuilt.direct_callees(rebuilt_caller), &[new_callee]);
    assert_eq!(
        names(&rebuilt, rebuilt.direct_callees(rebuilt_caller)),
        vec!["c".to_owned()]
    );
}
