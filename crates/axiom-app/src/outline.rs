use axiom_index::{ProjectSymbol, ProjectSymbolKind};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutlineKind {
    Class,
    Interface,
    Trait,
    Enum,
    Function,
    Method,
    Property,
    Constant,
    EnumCase,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineItem {
    pub name: String,
    pub kind: OutlineKind,
    pub range: std::ops::Range<usize>,
    pub depth: usize,
    /// Index of the unambiguous declaring type in this outline.
    pub parent: Option<usize>,
}

fn kind(kind: ProjectSymbolKind) -> Option<OutlineKind> {
    Some(match kind {
        ProjectSymbolKind::Class => OutlineKind::Class,
        ProjectSymbolKind::Interface => OutlineKind::Interface,
        ProjectSymbolKind::Trait => OutlineKind::Trait,
        ProjectSymbolKind::Enum => OutlineKind::Enum,
        ProjectSymbolKind::Function => OutlineKind::Function,
        ProjectSymbolKind::Method => OutlineKind::Method,
        ProjectSymbolKind::Property => OutlineKind::Property,
        ProjectSymbolKind::ClassConstant | ProjectSymbolKind::Constant => OutlineKind::Constant,
        ProjectSymbolKind::EnumCase => OutlineKind::EnumCase,
    })
}

pub fn build_file_outline<'a>(
    symbols: impl IntoIterator<Item = &'a ProjectSymbol>,
    file: &Path,
) -> Vec<OutlineItem> {
    let mut all: Vec<_> = symbols
        .into_iter()
        .filter(|s| s.file == file)
        .filter_map(|s| Some((s, kind(s.kind)?)))
        .collect();
    all.sort_by_key(|(s, _)| s.range.start);
    // Index ranges cover names only. Ownership comes from the index's encoded
    // owner FQN, never containment or a short-name guess. Duplicate owners are
    // ambiguous and deliberately left unparented.
    let mut owners = HashMap::new();
    for (i, (s, k)) in all.iter().enumerate() {
        if matches!(
            k,
            OutlineKind::Class | OutlineKind::Interface | OutlineKind::Trait | OutlineKind::Enum
        ) {
            owners
                .entry(s.fully_qualified_name.as_str())
                .and_modify(|v| *v = None)
                .or_insert(Some(i));
        }
    }
    let mut out = Vec::new();
    for (s, k) in &all {
        let parent = match k {
            OutlineKind::Method
            | OutlineKind::Property
            | OutlineKind::Constant
            | OutlineKind::EnumCase => s
                .fully_qualified_name
                .rsplit_once("::")
                .and_then(|(owner, _)| owners.get(owner).copied().flatten()),
            _ => None,
        };
        out.push(OutlineItem {
            name: s.name.clone(),
            kind: *k,
            range: s.range.clone(),
            depth: usize::from(parent.is_some()),
            parent,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    // Parsing belongs only to fixture construction using the existing index.
    // The outline consumes resident symbols and performs no parsing or IO.
    fn check(source: &str, expected: &[(&str, OutlineKind, Option<usize>)]) {
        let mut index = axiom_index::ProjectSymbolIndex::new();
        let path = Path::new("outline-unsaved-fixture.php");
        index
            .index_file_text(path, format!("<?php {source}"))
            .unwrap();
        let mut symbols = index.symbols().to_vec();
        symbols.reverse(); // Input iteration order is not structural order.
        let items = build_file_outline(&symbols, path);
        assert_eq!(items.len(), expected.len());
        for (item, (name, kind, parent)) in items.iter().zip(expected) {
            assert_eq!(
                (&*item.name, item.kind, item.parent),
                (*name, *kind, *parent)
            );
            assert_eq!(item.depth, usize::from(parent.is_some()));
            let original = symbols.iter().find(|s| s.range == item.range).unwrap();
            assert_eq!(original.name, item.name);
        }
        assert!(
            items
                .windows(2)
                .all(|w| w[0].range.start <= w[1].range.start)
        );
    }

    #[test]
    fn interface_methods() {
        use OutlineKind::*;
        check(
            "interface RepositoryInterface { public function find(); public function save(); }",
            &[
                ("RepositoryInterface", Interface, None),
                ("find", Method, Some(0)),
                ("save", Method, Some(0)),
            ],
        );
    }
    #[test]
    fn trait_property_and_method() {
        use OutlineKind::*;
        check(
            "trait HasUuid { private string $uuid; public function uuid(): string {} }",
            &[
                ("HasUuid", Trait, None),
                ("$uuid", Property, Some(0)),
                ("uuid", Method, Some(0)),
            ],
        );
    }
    #[test]
    fn enum_cases_and_method() {
        use OutlineKind::*;
        check(
            "enum Status { case Pending; case Paid; case Cancelled; public function label(): string {} }",
            &[
                ("Status", Enum, None),
                ("Pending", EnumCase, Some(0)),
                ("Paid", EnumCase, Some(0)),
                ("Cancelled", EnumCase, Some(0)),
                ("label", Method, Some(0)),
            ],
        );
    }
    #[test]
    fn properties_source_order() {
        use OutlineKind::*;
        check(
            "class UserService { private Repository $repository; protected Logger $logger; public string $name; }",
            &[
                ("UserService", Class, None),
                ("$repository", Property, Some(0)),
                ("$logger", Property, Some(0)),
                ("$name", Property, Some(0)),
            ],
        );
    }
    #[test]
    fn class_constants() {
        use OutlineKind::*;
        check(
            "class Config { public const VERSION = 1; private const DEFAULT_TIMEOUT = 30; }",
            &[
                ("Config", Class, None),
                ("VERSION", Constant, Some(0)),
                ("DEFAULT_TIMEOUT", Constant, Some(0)),
            ],
        );
    }
    #[test]
    fn multiple_types_and_global_function() {
        use OutlineKind::*;
        check(
            "class A { public function one() {} } interface B { public function two(); } trait C { public function three() {} } enum D { case Four; } function helper() {}",
            &[
                ("A", Class, None),
                ("one", Method, Some(0)),
                ("B", Interface, None),
                ("two", Method, Some(2)),
                ("C", Trait, None),
                ("three", Method, Some(4)),
                ("D", Enum, None),
                ("Four", EnumCase, Some(6)),
                ("helper", Function, None),
            ],
        );
    }
    #[test]
    fn repeated_member_names_use_owner_fqn() {
        use OutlineKind::*;
        check(
            "namespace Z; class B { public function same() {} } class A { public function same() {} }",
            &[
                ("B", Class, None),
                ("same", Method, Some(0)),
                ("A", Class, None),
                ("same", Method, Some(2)),
            ],
        );
    }
    #[test]
    fn files_and_updated_in_memory_symbols_are_isolated() {
        let mut index = axiom_index::ProjectSymbolIndex::new();
        let a = Path::new("outline-unsaved-A.php");
        let b = Path::new("outline-unsaved-B.php");
        index.index_file_text(a, "<?php class A {}").unwrap();
        index.index_file_text(b, "<?php class B {}").unwrap();
        assert_eq!(build_file_outline(index.symbols(), a)[0].name, "A");
        assert_eq!(build_file_outline(index.symbols(), b)[0].name, "B");
        index
            .index_file_text(a, "<?php function unsaved() {}")
            .unwrap();
        let current = build_file_outline(index.symbols(), a);
        assert_eq!(current.len(), 1);
        assert_eq!(current[0].name, "unsaved");
        assert_eq!(build_file_outline(index.symbols(), b).len(), 1);
    }
    #[test]
    fn filters_file_and_preserves_source_order() {
        let file = std::path::PathBuf::from("A.php");
        let symbols = vec![
            ProjectSymbol {
                name: "save".into(),
                fully_qualified_name: "A::save".into(),
                kind: ProjectSymbolKind::Method,
                file: file.clone(),
                range: 40..44,
                namespace: "".into(),
                visibility: axiom_index::Visibility::Public,
                modifiers: vec![],
                parameters: None,
                return_type: None,
            },
            ProjectSymbol {
                name: "A".into(),
                fully_qualified_name: "A".into(),
                kind: ProjectSymbolKind::Class,
                file: file.clone(),
                range: 0..80,
                namespace: "".into(),
                visibility: axiom_index::Visibility::Public,
                modifiers: vec![],
                parameters: None,
                return_type: None,
            },
        ];
        let outline = build_file_outline(symbols.iter(), &file);
        assert_eq!(
            outline
                .iter()
                .map(|i| (i.name.as_str(), i.depth))
                .collect::<Vec<_>>(),
            vec![("A", 0), ("save", 1)]
        );
    }
}
