use axiom_index::{ProjectSymbolIndex, SemanticRevision, SemanticSnapshot};
use std::fs;

fn index_text(path: &std::path::Path, source: &str) -> ProjectSymbolIndex {
    let mut index = ProjectSymbolIndex::new();
    index.index_file_text(path, source).unwrap();
    index
}

#[test]
fn extracts_declared_throws_for_functions_and_methods() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Service.php");
    let source = r#"<?php
/** @throws \App\PersistenceException */
function persist() {}

class Repo {
    /**
     * @throws \App\PersistenceException
     * @throws \App\StorageException
     * @throws \App\PersistenceException
     */
    public function save() {}
}

function clean() {}
"#;
    let index = index_text(&path, source);

    let persist = index.find_fqn("persist").unwrap();
    assert_eq!(
        persist.declared_throws,
        vec!["\\App\\PersistenceException".to_owned()]
    );
    assert!(persist.docblock_range.is_some());

    let save = index.find_fqn("Repo::save").unwrap();
    assert_eq!(
        save.declared_throws,
        vec![
            "\\App\\PersistenceException".to_owned(),
            "\\App\\StorageException".to_owned()
        ]
    );
    assert!(save.docblock_range.is_some());

    let clean = index.find_fqn("clean").unwrap();
    assert!(clean.declared_throws.is_empty());
    assert!(clean.docblock_range.is_none());
}

#[test]
fn semantic_symbols_receive_declared_throws_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Service.php");
    let index = index_text(
        &path,
        r#"<?php
class Repo {
    /** @throws \App\StorageException */
    public function save() {}
}
"#,
    );
    let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(1));
    let symbol_id = snapshot.symbols_for_fqn("Repo::save")[0];
    let symbol = snapshot.symbol(symbol_id).unwrap();

    assert_eq!(
        symbol.declared_throws,
        vec!["\\App\\StorageException".to_owned()]
    );
    assert!(symbol.docblock_range.is_some());
}

#[test]
fn cache_round_trip_preserves_declared_throws() {
    let project = tempfile::tempdir().unwrap();
    let source = project.path().join("src/Service.php");
    let cache = project.path().join(".cache/project.json");
    fs::create_dir_all(source.parent().unwrap()).unwrap();
    fs::write(
        &source,
        r#"<?php
/** @throws \App\StorageException */
function persist() {}
"#,
    )
    .unwrap();

    let mut first = ProjectSymbolIndex::new();
    first.index_project_cached(project.path(), &cache).unwrap();
    let mut second = ProjectSymbolIndex::new();
    second.index_project_cached(project.path(), &cache).unwrap();

    let symbol = second.find_fqn("persist").unwrap();
    assert_eq!(
        symbol.declared_throws,
        vec!["\\App\\StorageException".to_owned()]
    );
    assert!(symbol.docblock_range.is_some());
}

#[test]
fn old_project_cache_schema_is_invalidated() {
    let project = tempfile::tempdir().unwrap();
    let source = project.path().join("src/Service.php");
    let cache = project.path().join(".cache/project.json");
    fs::create_dir_all(source.parent().unwrap()).unwrap();
    fs::write(
        &source,
        r#"<?php
/** @throws \App\OldException */
function persist() {}
"#,
    )
    .unwrap();

    let mut stale_index = ProjectSymbolIndex::new();
    stale_index
        .index_file_text(&source, fs::read_to_string(&source).unwrap())
        .unwrap();
    let mut stale_symbol = stale_index.find_fqn("persist").unwrap().clone();
    stale_symbol.declared_throws = vec!["\\App\\StaleException".to_owned()];

    fs::write(
        &source,
        r#"<?php
/** @throws \App\CurrentException */
function persist() {}
"#,
    )
    .unwrap();
    let metadata = fs::metadata(&source).unwrap();
    let modified = metadata
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let mut files = serde_json::Map::new();
    files.insert(
        source.to_string_lossy().into_owned(),
        serde_json::json!([
            metadata.len(),
            modified,
            [serde_json::to_value(stale_symbol).unwrap()]
        ]),
    );
    let old_cache = serde_json::json!({
        "schema_version": 3,
        "files": files,
    });
    fs::create_dir_all(cache.parent().unwrap()).unwrap();
    fs::write(&cache, serde_json::to_vec(&old_cache).unwrap()).unwrap();

    let mut index = ProjectSymbolIndex::new();
    index.index_project_cached(project.path(), &cache).unwrap();
    let symbol = index.find_fqn("persist").unwrap();
    assert_eq!(
        symbol.declared_throws,
        vec!["\\App\\CurrentException".to_owned()]
    );
}

#[test]
fn docblock_metadata_does_not_leak_between_symbols() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Service.php");
    let index = index_text(
        &path,
        r#"<?php
/** @throws \App\FunctionException */
function first() {}

class Repo {
    /** @throws \App\MethodException */
    public function save() {}
}
"#,
    );

    assert_eq!(
        index.find_fqn("first").unwrap().declared_throws,
        vec!["\\App\\FunctionException".to_owned()]
    );
    assert_eq!(
        index.find_fqn("Repo::save").unwrap().declared_throws,
        vec!["\\App\\MethodException".to_owned()]
    );
    assert!(index.find_fqn("Repo").unwrap().declared_throws.is_empty());
}
