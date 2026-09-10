//! Bounded presentation adapter over the editor's existing semantic filters.
use crate::editor_view::{
    TypeCompletionContext, rank_new_completion_items, runtime_type_kind_allowed, type_kind_allowed,
};
use axiom_index::{ProjectSymbolIndex, VendorSymbolIndex};
use axiom_php::RuntimeSymbolIndex;
use lsp_types::CompletionItem;
use std::ops::Range;

pub(crate) const LIMIT: usize = 32;
const CANDIDATE_BUDGET: usize = 96;

/// Byte range of the comma-separated value at the caret, retaining surrounding whitespace.
pub(crate) fn active_token(text: &str, caret: usize) -> (Range<usize>, &str) {
    let caret = caret.min(text.len());
    let start = text[..caret].rfind(',').map_or(0, |i| i + 1);
    let end = text[caret..].find(',').map_or(text.len(), |i| caret + i);
    let segment = &text[start..end];
    let trimmed = segment.trim();
    let start = start + segment.len() - segment.trim_start().len();
    let end = start + trimmed.len();
    let prefix_end = caret.clamp(start, end);
    (start..end, &text[start..prefix_end])
}

pub(crate) fn lookup(
    context: TypeCompletionContext,
    prefix: &str,
    project: Option<&ProjectSymbolIndex>,
    vendor: Option<&VendorSymbolIndex>,
    runtime: Option<&RuntimeSymbolIndex>,
) -> Vec<CompletionItem> {
    let prefix = prefix.trim_start_matches('\\');
    if prefix.is_empty() {
        return Vec::new();
    }
    let mut items = Vec::new();
    let mut add = |name: &str, fqn: &str, source: &str| {
        items.push(CompletionItem {
            label: name.to_owned(),
            detail: Some(format!("{fqn} - {source}")),
            insert_text: Some(fqn.trim_start_matches('\\').to_owned()),
            ..Default::default()
        });
    };
    if let Some(index) = project {
        for symbol in index.search_prefix_limited(prefix, CANDIDATE_BUDGET) {
            if type_kind_allowed(context, symbol.kind) {
                add(&symbol.name, &symbol.fully_qualified_name, "Project");
            }
        }
    }
    if let Some(index) = vendor {
        for symbol in index.types_matching_limited(prefix, CANDIDATE_BUDGET) {
            if type_kind_allowed(context, symbol.kind) {
                add(&symbol.short_name, &symbol.fqn, "Vendor");
            }
        }
    }
    if let Some(index) = runtime {
        for symbol in index.search_prefix_limited(prefix, CANDIDATE_BUDGET) {
            if runtime_type_kind_allowed(context, symbol.kind) {
                add(&symbol.name, &symbol.fqn, "Runtime");
            }
        }
    }
    rank_new_completion_items(&mut items, prefix);
    let mut seen = std::collections::HashSet::new();
    items.retain(|item| seen.insert(item.insert_text.as_ref().unwrap().to_ascii_lowercase()));
    items.truncate(LIMIT);
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor_view::TypeCompletionContext::*;

    #[test]
    fn modal_types_project_contexts_and_bounded_candidates() {
        let mut project = ProjectSymbolIndex::new();
        let root = tempfile::tempdir().unwrap();
        project.index_project(root.path()).unwrap();
        let mut text = "<?php namespace App; class ServiceBase {} interface ServiceContract {} trait ServiceTrait {} enum ServiceEnum {} function ServiceFunction() {}".to_owned();
        for i in 0..200 {
            text.push_str(&format!(" class Service{i:03} {{}}"));
        }
        project
            .index_file_text("modal-completion.php", text)
            .unwrap();
        for context in [ClassImplements, InterfaceExtends] {
            let items = lookup(context, "ServiceC", Some(&project), None, None);
            assert_eq!(items.len(), 1);
            assert_eq!(
                items[0].insert_text.as_deref(),
                Some("App\\ServiceContract")
            );
        }
        let items = lookup(ClassExtends, "ServiceB", Some(&project), None, None);
        assert_eq!(items[0].insert_text.as_deref(), Some("App\\ServiceBase"));
        assert!(lookup(ClassExtends, "ServiceC", Some(&project), None, None).is_empty());
        assert!(lookup(ClassExtends, "", Some(&project), None, None).is_empty());
        assert_eq!(
            lookup(ClassExtends, "Service", Some(&project), None, None).len(),
            LIMIT
        );
        assert_eq!(project.search_prefix_limited("Service", 3).len(), 3);
        assert_eq!(
            lookup(ClassExtends, "App\\ServiceBase", Some(&project), None, None).len(),
            1
        );
    }

    #[test]
    fn modal_types_runtime_uses_same_kind_filter_and_deduplicates() {
        use axiom_php::{SourceLocation, Symbol, SymbolKind, SymbolOrigin};
        let mut runtime = RuntimeSymbolIndex::default();
        for (name, kind) in [
            ("CacheBase", SymbolKind::Class),
            ("CacheContract", SymbolKind::Interface),
            ("CacheTrait", SymbolKind::Trait),
            ("CacheEnum", SymbolKind::Enum),
        ] {
            runtime.insert(Symbol {
                name: name.into(),
                fqn: format!("Runtime\\{name}"),
                kind,
                origin: SymbolOrigin::PhpRuntime,
                extension: "test".into(),
                location: SourceLocation {
                    file: "fixture.php".into(),
                    range: 0..0,
                },
                declared_type: None,
                signature: None,
                documentation: None,
                availability: Default::default(),
                is_static: false,
            });
        }
        assert_eq!(
            lookup(ClassExtends, "Cache", None, None, Some(&runtime))[0].label,
            "CacheBase"
        );
        for context in [ClassImplements, InterfaceExtends] {
            let items = lookup(context, "Cache", None, None, Some(&runtime));
            assert_eq!(items.len(), 1);
            assert_eq!(items[0].label, "CacheContract");
        }
        assert_eq!(runtime.search_prefix_limited("Cache", 2).len(), 2);
    }

    #[test]
    fn modal_types_active_token_preserves_other_values_and_unicode() {
        for (text, caret, expected, prefix) in [
            (
                "CacheInterface, Log",
                19,
                "CacheInterface, Psr\\Log\\LoggerInterface",
                "Log",
            ),
            (
                "Árvore, Log, Other",
                12,
                "Árvore, Psr\\Log\\LoggerInterface, Other",
                "Log",
            ),
        ] {
            let (range, actual_prefix) = active_token(text, caret);
            assert_eq!(actual_prefix, prefix);
            let mut output = text.to_owned();
            output.replace_range(range, "Psr\\Log\\LoggerInterface");
            assert_eq!(output, expected);
        }
    }

    #[test]
    fn modal_types_vendor_only_resident_declarations() {
        let root = tempfile::tempdir().unwrap();
        let vendor = root.path().join("vendor/pkg");
        std::fs::create_dir_all(&vendor).unwrap();
        std::fs::write(
            root.path().join("composer.json"),
            r#"{"autoload":{"psr-4":{"Pkg\\":"vendor/pkg/"}}}"#,
        )
        .unwrap();
        for (name, kind) in [
            ("CacheBase", "class"),
            ("CacheContract", "interface"),
            ("CacheTrait", "trait"),
            ("CacheEnum", "enum"),
        ] {
            std::fs::write(
                vendor.join(format!("{name}.php")),
                format!("<?php namespace Pkg; {kind} {name} {{}}"),
            )
            .unwrap();
        }
        let mut index = VendorSymbolIndex::load(root.path()).unwrap();
        assert!(lookup(ClassImplements, "Cache", None, Some(&index), None).is_empty());
        for name in ["CacheBase", "CacheContract", "CacheTrait", "CacheEnum"] {
            index.symbols_of(&format!("Pkg\\{name}"));
            std::fs::remove_file(vendor.join(format!("{name}.php"))).unwrap();
        }
        for context in [ClassImplements, InterfaceExtends] {
            let items = lookup(context, "Cache", None, Some(&index), None);
            assert_eq!(items.len(), 1);
            assert_eq!(items[0].insert_text.as_deref(), Some("Pkg\\CacheContract"));
        }
        assert_eq!(
            lookup(ClassExtends, "Cache", None, Some(&index), None)[0].label,
            "CacheBase"
        );
        assert_eq!(index.types_matching_limited("Cache", 2).len(), 2);
    }
}
