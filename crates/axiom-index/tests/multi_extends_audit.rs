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

#[test]
fn direct_receiver_trait_namespace_alias_regression() {
    let text = "<?php namespace Shared; trait T { public function run(): void {} public string $name; public const VERSION = '1'; } namespace Model; use Shared\\T as Runnable; class C { use Runnable; } function test(C $c): void { $c->run(); $c->name; } $v = C::VERSION;";
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("direct-trait.php");
    fs::write(&path, text).unwrap();
    let mut index = ProjectSymbolIndex::new();
    index.index_project(dir.path()).unwrap();
    let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(5));
    let run = snapshot.definition_at_detailed(
        &path,
        text,
        text.find("$c->run").unwrap() + 4,
        context(&snapshot),
    );
    assert_eq!(run.outcome, SemanticDefinitionOutcome::Resolved);
    let name = snapshot.definition_at_detailed(
        &path,
        text,
        text.find("$c->name").unwrap() + 4,
        context(&snapshot),
    );
    let version = snapshot.definition_at_detailed(
        &path,
        text,
        text.rfind("VERSION").unwrap(),
        context(&snapshot),
    );
    assert_eq!(name.outcome, SemanticDefinitionOutcome::Resolved);
    assert_eq!(version.outcome, SemanticDefinitionOutcome::Resolved);
}

#[test]
fn incremental_replace_preserves_direct_trait_relation() {
    let dir = tempfile::tempdir().unwrap();
    let trait_path = dir.path().join("T.php");
    let class_path = dir.path().join("C.php");
    let test_path = dir.path().join("test.php");
    let trait_text = "<?php namespace N; trait T { public function run(): void {} }";
    let class_before = "<?php namespace N; class C {}";
    let class_after = "<?php namespace N; class C { use T; }";
    let use_text = "<?php namespace N; function test(C $c): void { $c->run(); }";
    fs::write(&trait_path, trait_text).unwrap();
    fs::write(&class_path, class_before).unwrap();
    fs::write(&test_path, use_text).unwrap();
    let mut index = ProjectSymbolIndex::new();
    index.index_project(dir.path()).unwrap();
    let base = SemanticSnapshot::from_project_index(&index, SemanticRevision(6));
    let mut builder = axiom_index::SnapshotBuilder::from_snapshot(&base);
    builder.replace_workspace_file(&class_path, class_after);
    let changed = builder.finish();
    let result = changed.definition_at_detailed(
        &test_path,
        use_text,
        use_text.find("run").unwrap(),
        context(&changed),
    );
    assert_eq!(result.outcome, SemanticDefinitionOutcome::Resolved);
    let mut remove_builder = axiom_index::SnapshotBuilder::from_snapshot(&changed);
    remove_builder.replace_workspace_file(&class_path, class_before);
    let removed = remove_builder.finish();
    let result = removed.definition_at_detailed(
        &test_path,
        use_text,
        use_text.find("run").unwrap(),
        context(&removed),
    );
    assert_eq!(result.outcome, SemanticDefinitionOutcome::MissingSymbol);
}

#[test]
fn enum_case_definition_is_owner_scoped() {
    let text = "<?php namespace N; enum Status { case Active; case Disabled; public const VERSION = '1'; } enum Feature { case Active; } $a = Status::Active; $b = Feature::Active; $v = Status::VERSION;";
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("enum.php");
    fs::write(&path, text).unwrap();
    let mut index = ProjectSymbolIndex::new();
    index.index_project(dir.path()).unwrap();
    let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(7));
    let status = snapshot.symbols_for_fqn("N\\Status");
    assert_eq!(status.len(), 1);
    let active = snapshot.members_named(
        status[0],
        "Active",
        axiom_index::ProjectSymbolKind::EnumCase,
    );
    assert_eq!(active.len(), 1);
    let first = snapshot.definition_at_detailed(
        &path,
        text,
        text.find("Status::Active").unwrap() + "Status::".len(),
        context(&snapshot),
    );
    let second = snapshot.definition_at_detailed(
        &path,
        text,
        text.find("Feature::Active").unwrap() + "Feature::".len(),
        context(&snapshot),
    );
    assert_eq!(first.outcome, SemanticDefinitionOutcome::Resolved);
    assert_eq!(second.outcome, SemanticDefinitionOutcome::Resolved);
}

#[test]
fn enum_case_incremental_add_remove_and_rename() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("status.php");
    let use_text = "<?php enum Status { case Pending; } $x = Status::Active;";
    fs::write(&path, use_text).unwrap();
    let mut index = ProjectSymbolIndex::new();
    index.index_project(dir.path()).unwrap();
    let base = SemanticSnapshot::from_project_index(&index, SemanticRevision(8));
    let add_text = "<?php enum Status { case Pending; case Active; } $x = Status::Active;";
    let mut add = axiom_index::SnapshotBuilder::from_snapshot(&base);
    add.replace_workspace_file(&path, add_text);
    let added = add.finish();
    assert_eq!(
        added
            .definition_at_detailed(
                &path,
                add_text,
                add_text.rfind("Active").unwrap(),
                context(&added)
            )
            .outcome,
        SemanticDefinitionOutcome::Resolved
    );
    let rename_text = "<?php enum Status { case Pending; case Enabled; } $x = Status::Active;";
    let mut rename = axiom_index::SnapshotBuilder::from_snapshot(&added);
    rename.replace_workspace_file(&path, rename_text);
    let renamed = rename.finish();
    assert_eq!(
        renamed
            .definition_at_detailed(
                &path,
                rename_text,
                rename_text.rfind("Active").unwrap(),
                context(&renamed)
            )
            .outcome,
        SemanticDefinitionOutcome::MissingSymbol
    );
}

#[test]
fn enum_implements_interface_relations_and_incremental_updates() {
    let dir = tempfile::tempdir().unwrap();
    let contracts = dir.path().join("contracts.php");
    let domain = dir.path().join("status.php");
    let child = dir.path().join("child.php");
    fs::write(
        &contracts,
        "<?php namespace App\\Contracts; interface Runnable { public function run(): void; } interface ParentContract extends Runnable {} interface Other { public function other(): void; }",
    )
    .unwrap();
    fs::write(
        &domain,
        "<?php namespace App\\Domain; use App\\Contracts\\Runnable as Contract; use App\\Contracts\\Other; enum Status implements Contract, Other { case Active; public function run(): void {} public function other(): void {} }",
    )
    .unwrap();
    fs::write(
        &child,
        "<?php namespace App\\Domain; class Base implements \\App\\Contracts\\ParentContract { public function run(): void {} } class Child extends Base {}",
    )
    .unwrap();

    let mut index = ProjectSymbolIndex::new();
    index.index_project(dir.path()).unwrap();
    let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(9));
    let runnable = snapshot.symbols_for_fqn("App\\Contracts\\Runnable")[0];
    let parent = snapshot.symbols_for_fqn("App\\Contracts\\ParentContract")[0];
    let other = snapshot.symbols_for_fqn("App\\Contracts\\Other")[0];
    let status = snapshot.symbols_for_fqn("App\\Domain\\Status")[0];

    assert_eq!(snapshot.direct_implementers_of(runnable), &[status]);
    let runnable_implementers = snapshot.implementers_of(runnable);
    assert!(runnable_implementers.contains(&status));
    assert_eq!(runnable_implementers.len(), 3);
    assert_eq!(snapshot.direct_implementers_of(other), &[status]);
    assert_eq!(snapshot.implementers_of(parent).len(), 2);
    let run = snapshot.members_named(runnable, "run", axiom_index::ProjectSymbolKind::Method)[0];
    let implementations = snapshot.implementations_of(run);
    assert!(implementations.iter().any(|id| *id
        == snapshot.members_named(status, "run", axiom_index::ProjectSymbolKind::Method)[0]));
    assert!(!implementations.iter().any(|id| {
        snapshot
            .symbol(*id)
            .is_some_and(|s| s.kind == axiom_index::ProjectSymbolKind::EnumCase)
    }));

    let changed_text = "<?php namespace App\\Domain; use App\\Contracts\\Runnable as Contract; enum Status implements Contract { case Active; public function run(): void {} }";
    let mut builder = axiom_index::SnapshotBuilder::from_snapshot(&snapshot);
    builder.replace_workspace_file(&domain, changed_text);
    let changed = builder.finish();
    assert!(changed.direct_implementers_of(other).is_empty());
    assert!(changed.implementers_of(runnable).contains(&status));
}
