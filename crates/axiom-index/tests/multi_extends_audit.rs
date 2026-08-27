use axiom_index::{
    DefinitionQueryContext, ProjectSymbolIndex, SemanticDefinitionOutcome, SemanticRevision,
    SemanticSnapshot,
};
use std::fs;

fn context(snapshot: &SemanticSnapshot) -> DefinitionQueryContext {
    DefinitionQueryContext {
        document_version: None,
        semantic_revision: snapshot.revision,
    }
}

#[test]
fn multi_extends_definition_audit() {
    let text = "<?php interface A { public function a(): void; } interface B { public function b(): void; } interface C extends A, B {} function test(C $c): void { $c->a(); $c->b(); }";
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("multi-extends-audit.php");
    fs::write(&path, text).unwrap();
    let mut index = ProjectSymbolIndex::new();
    index.index_project(dir.path()).unwrap();
    let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(1));
    let a = snapshot.definition_at_detailed(
        &path,
        text,
        text.find("$c->a").unwrap() + 4,
        context(&snapshot),
    );
    let b = snapshot.definition_at_detailed(
        &path,
        text,
        text.find("$c->b").unwrap() + 4,
        context(&snapshot),
    );
    assert_eq!(a.outcome, SemanticDefinitionOutcome::Resolved);
    assert_eq!(b.outcome, SemanticDefinitionOutcome::Resolved);
}

#[test]
fn interface_diamond_deduplicates_root() {
    let text = "<?php interface Root { public function run(): void; } interface Left extends Root {} interface Right extends Root {} interface FinalContract extends Left, Right {} function test(FinalContract $x): void { $x->run(); }";
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("diamond.php");
    fs::write(&path, text).unwrap();
    let mut index = ProjectSymbolIndex::new();
    index.index_project(dir.path()).unwrap();
    let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(2));
    let result = snapshot.definition_at_detailed(
        &path,
        text,
        text.find("$x->run").unwrap() + 4,
        context(&snapshot),
    );
    assert_eq!(result.outcome, SemanticDefinitionOutcome::Resolved);
}

#[test]
fn trait_method_definition_uses_original_trait_symbol() {
    let text = "<?php trait T { public function run(): void {} } class C { use T; } function test(C $c): void { $c->run(); }";
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trait.php");
    fs::write(&path, text).unwrap();
    let mut index = ProjectSymbolIndex::new();
    index.index_project(dir.path()).unwrap();
    let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(3));
    let result = snapshot.definition_at_detailed(
        &path,
        text,
        text.find("$c->run").unwrap() + 4,
        context(&snapshot),
    );
    assert_eq!(result.outcome, SemanticDefinitionOutcome::Resolved);
    let trait_span = text.find("function run").unwrap() + "function ".len();
    assert!(
        format!("{:?}", result.result).contains(&format!("span: {trait_span}..{}", trait_span + 3))
    );
}

#[test]
fn inherited_class_trait_supplies_method_property_and_constant() {
    let text = "<?php trait T { public string $name; public const VERSION = '1'; public function run(): void {} } class Base { use T; } class Child extends Base {} function test(Child $c): void { $c->name; $c->run(); } $value = Child::VERSION;";
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("inherited-trait.php");
    fs::write(&path, text).unwrap();
    let mut index = ProjectSymbolIndex::new();
    index.index_project(dir.path()).unwrap();
    let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(4));
    let ctx = context(&snapshot);
    let property =
        snapshot.definition_at_detailed(&path, text, text.find("$c->name").unwrap() + 4, ctx);
    let method = snapshot.definition_at_detailed(
        &path,
        text,
        text.find("$c->run").unwrap() + 4,
        context(&snapshot),
    );
    let constant = snapshot.definition_at_detailed(
        &path,
        text,
        text.rfind("VERSION").unwrap(),
        context(&snapshot),
    );
    assert_eq!(property.outcome, SemanticDefinitionOutcome::Resolved);
    assert_eq!(method.outcome, SemanticDefinitionOutcome::Resolved);
    assert_eq!(constant.outcome, SemanticDefinitionOutcome::Resolved);
}
