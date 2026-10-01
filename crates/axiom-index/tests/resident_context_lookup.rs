use axiom_index::{
    PersistentFileKey, ProjectSymbolIndex, ProjectSymbolKind, SemanticRevision, SemanticSnapshot,
    semantic_text_fingerprint,
};
use std::path::Path;

fn snapshot_from_text(path: &Path, source: &str) -> SemanticSnapshot {
    std::fs::write(path, source).unwrap();
    let mut index = ProjectSymbolIndex::new();
    index.index_file_text(path, source).unwrap();
    SemanticSnapshot::from_project_index(&index, SemanticRevision(1))
}

#[test]
fn file_fingerprint_lookup_matches_the_index_algorithm() {
    let path = std::env::temp_dir().join("axiom-resident-fingerprint.php");
    let source = "<?php\nfunction persist() {}\n";
    let snapshot = snapshot_from_text(&path, source);
    let file = snapshot
        .file_id(&PersistentFileKey::workspace(&path))
        .unwrap();
    let fingerprint = semantic_text_fingerprint(source);

    assert_eq!(snapshot.file_fingerprint(file), Some(fingerprint));
    assert!(snapshot.matches_file_fingerprint(file, fingerprint));
    assert!(!snapshot.matches_file_fingerprint(file, fingerprint ^ 1));
}

#[test]
fn docblock_range_resolves_to_its_unique_callable() {
    let path = std::env::temp_dir().join("axiom-resident-docblock.php");
    let source = r#"<?php
/** @throws \App\PersistenceException */
function persist() {}

class Repo {
    /** @throws \App\StorageException */
    public function save() {}
}
"#;
    let snapshot = snapshot_from_text(&path, source);
    let file = snapshot
        .file_id(&PersistentFileKey::workspace(&path))
        .unwrap();

    for fqn in ["persist", "Repo::save"] {
        let symbol_id = snapshot.symbols_for_fqn(fqn)[0];
        let symbol = snapshot.symbol(symbol_id).unwrap();
        let docblock_range = symbol.docblock_range.clone().unwrap();
        assert!(matches!(
            symbol.kind,
            ProjectSymbolKind::Function | ProjectSymbolKind::Method
        ));
        assert_eq!(
            snapshot.callable_for_docblock(file, &docblock_range),
            Some(symbol_id)
        );
    }
}

#[test]
fn incorrect_or_missing_docblock_range_does_not_resolve() {
    let path = std::env::temp_dir().join("axiom-resident-wrong-docblock.php");
    let source = r#"<?php
/** Class documentation. */
class Repo {
    /** @throws \App\StorageException */
    public function save() {}
}
"#;
    let snapshot = snapshot_from_text(&path, source);
    let file = snapshot
        .file_id(&PersistentFileKey::workspace(&path))
        .unwrap();
    let save = snapshot.symbol(snapshot.symbols_for_fqn("Repo::save")[0]).unwrap();
    let save_docblock = save.docblock_range.clone().unwrap();

    assert_eq!(
        snapshot.callable_for_docblock(file, &(save_docblock.start + 1..save_docblock.end)),
        None
    );
    assert_eq!(
        snapshot.callable_for_docblock(file, &(save_docblock.end + 1..save_docblock.end + 2)),
        None
    );
}

#[test]
fn derived_docblock_lookup_survives_snapshot_json_round_trip() {
    let path = std::env::temp_dir().join("axiom-resident-round-trip.php");
    let source = "<?php\n/** @throws \\App\\PersistenceException */\nfunction persist() {}\n";
    let snapshot = snapshot_from_text(&path, source);
    let cache = std::env::temp_dir().join("axiom-resident-snapshot.json");
    snapshot.save_json(&cache).unwrap();
    let restored = SemanticSnapshot::load_json(&cache).unwrap();
    let file = restored
        .file_id(&PersistentFileKey::workspace(&path))
        .unwrap();
    let symbol_id = restored.symbols_for_fqn("persist")[0];
    let docblock_range = restored.symbol(symbol_id).unwrap().docblock_range.clone().unwrap();

    assert_eq!(
        restored.callable_for_docblock(file, &docblock_range),
        Some(symbol_id)
    );
    std::fs::remove_file(cache).unwrap();
}
