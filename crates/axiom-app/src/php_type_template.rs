//! Pure preparation of a new PHP declaration; no symbol resolution or UI state.
use std::collections::{BTreeMap, BTreeSet};

pub(crate) struct GeneratedPhpType {
    pub contents: String,
    pub caret_offset: usize,
}

fn normalize(value: &str) -> String {
    value
        .trim()
        .split('\\')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\\")
}

fn types(value: &str) -> Vec<(String, bool)> {
    let mut seen = BTreeSet::new();
    value
        .split(',')
        .filter_map(|value| {
            let qualified = value.trim().contains('\\');
            let name = normalize(value);
            (!name.is_empty() && seen.insert(name.to_ascii_lowercase()))
                .then_some((name, qualified))
        })
        .collect()
}

#[cfg(test)]
fn render(keyword: &str, name: &str, namespace: &str, extends: &str, implements: &str) -> String {
    render_with_caret(keyword, name, namespace, extends, implements)
        .contents
        .replace("{\n    \n}", "{\n}")
}

pub(crate) fn render_with_caret(
    keyword: &str,
    name: &str,
    namespace: &str,
    extends: &str,
    implements: &str,
) -> GeneratedPhpType {
    let namespace = normalize(namespace);
    let parents = if matches!(keyword, "class" | "interface") {
        types(extends)
    } else {
        Vec::new()
    };
    let interfaces = if keyword == "class" {
        types(implements)
    } else {
        Vec::new()
    };
    // Reserve the declaration and manual short names. All colliding qualified
    // references remain absolute, rather than arbitrarily assigning an alias.
    let mut owners: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    owners
        .entry(name.to_ascii_lowercase())
        .or_default()
        .insert("<declaration>".into());
    for (fqn, qualified) in parents.iter().chain(&interfaces) {
        let basename = fqn.rsplit('\\').next().unwrap();
        let identity = if *qualified {
            fqn.to_ascii_lowercase()
        } else {
            format!("<short:{}>", fqn.to_ascii_lowercase())
        };
        owners
            .entry(basename.to_ascii_lowercase())
            .or_default()
            .insert(identity);
    }
    let mut imports: BTreeMap<String, String> = BTreeMap::new();
    let mut reference = |(fqn, qualified): &(String, bool)| {
        if !qualified {
            return fqn.clone();
        }
        let (owner, basename) = fqn.rsplit_once('\\').unwrap_or(("", fqn));
        if owners[&basename.to_ascii_lowercase()].len() > 1 {
            return format!("\\{fqn}");
        }
        if !owner.eq_ignore_ascii_case(&namespace) {
            imports
                .entry(fqn.to_ascii_lowercase())
                .or_insert_with(|| fqn.clone());
        }
        basename.to_owned()
    };
    let parents: Vec<_> = parents.iter().map(&mut reference).collect();
    let interfaces: Vec<_> = interfaces.iter().map(&mut reference).collect();
    let mut output = "<?php\n\n".to_owned();
    if !namespace.is_empty() {
        output.push_str(&format!("namespace {namespace};\n\n"));
    }
    let mut imports: Vec<_> = imports.into_values().collect();
    imports.sort();
    for fqn in &imports {
        output.push_str(&format!("use {fqn};\n"));
    }
    if !imports.is_empty() {
        output.push('\n');
    }
    output.push_str(&format!("{keyword} {name}"));
    if !parents.is_empty() {
        output.push_str(&format!(" extends {}", parents.join(", ")));
    }
    if !interfaces.is_empty() {
        output.push_str(&format!(" implements {}", interfaces.join(", ")));
    }
    output.push_str("\n{\n    \n}\n");
    let caret_offset = output.len() - 3;
    GeneratedPhpType {
        contents: output,
        caret_offset,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_caret_is_after_indented_body_line() {
        for keyword in ["class", "interface", "trait", "enum"] {
            let generated = render_with_caret(keyword, "Item", "App", "App\\Base", "App\\Contract");
            assert_eq!(
                &generated.contents[generated.caret_offset - 4..generated.caret_offset],
                "    "
            );
            assert_eq!(generated.contents.as_bytes()[generated.caret_offset], b'\n');
            assert!(generated.caret_offset < generated.contents.rfind('}').unwrap());
        }
        let generated = render_with_caret("class", "Item", "", "", "");
        assert!(generated.contents.starts_with("<?php\n\nclass Item"));
    }

    #[test]
    fn external_class_template_and_sorted_imports() {
        assert_eq!(
            render(
                "class",
                "FileStone",
                "App\\Feature",
                "App\\TestCompletion\\Service",
                "App\\TestCompletion\\LoggerInterface"
            ),
            "<?php\n\nnamespace App\\Feature;\n\nuse App\\TestCompletion\\LoggerInterface;\nuse App\\TestCompletion\\Service;\n\nclass FileStone extends Service implements LoggerInterface\n{\n}\n"
        );
    }

    #[test]
    fn same_namespace_and_external_mix() {
        assert_eq!(
            render(
                "class",
                "FileStone",
                "App\\Service",
                "App\\Service\\Base",
                "App\\Service\\LocalInterface, Psr\\Log\\LoggerInterface"
            ),
            "<?php\n\nnamespace App\\Service;\n\nuse Psr\\Log\\LoggerInterface;\n\nclass FileStone extends Base implements LocalInterface, LoggerInterface\n{\n}\n"
        );
    }

    #[test]
    fn multiple_interfaces_keep_order_and_share_imports() {
        let body = render(
            "class",
            "UserService",
            "App\\Service",
            "App\\Core\\BaseService",
            "Psr\\SimpleCache\\CacheInterface, Psr\\Log\\LoggerInterface",
        );
        assert_eq!(
            body,
            "<?php\n\nnamespace App\\Service;\n\nuse App\\Core\\BaseService;\nuse Psr\\Log\\LoggerInterface;\nuse Psr\\SimpleCache\\CacheInterface;\n\nclass UserService extends BaseService implements CacheInterface, LoggerInterface\n{\n}\n"
        );
        let shared = render(
            "class",
            "Item",
            "App",
            "Vendor\\Contract",
            "Vendor\\Contract",
        );
        assert_eq!(shared.matches("use Vendor\\Contract;").count(), 1);
        let collision = render("class", "service", "App", "Vendor\\Service", "");
        assert!(!collision.contains("use "));
        assert!(collision.contains("extends \\Vendor\\Service"));
        assert_eq!(
            render("class", "Item", "App", "App\\Base", ""),
            "<?php\n\nnamespace App;\n\nclass Item extends Base\n{\n}\n"
        );
    }

    #[test]
    fn root_namespace_normalization_and_duplicates() {
        assert_eq!(
            render(
                "class",
                "FileStone",
                "",
                "\\App\\\\Core\\Base",
                "\\Psr\\Log\\LoggerInterface, Psr\\Log\\LoggerInterface, psr\\log\\loggerinterface"
            ),
            "<?php\n\nuse App\\Core\\Base;\nuse Psr\\Log\\LoggerInterface;\n\nclass FileStone extends Base implements LoggerInterface\n{\n}\n"
        );
    }

    #[test]
    fn collisions_use_absolute_references() {
        let body = render(
            "class",
            "FileStone",
            "App",
            "App\\Service\\Logger",
            "Vendor\\Package\\Logger",
        );
        assert_eq!(
            body,
            "<?php\n\nnamespace App;\n\nclass FileStone extends \\App\\Service\\Logger implements \\Vendor\\Package\\Logger\n{\n}\n"
        );
        let body = render(
            "class",
            "Cache",
            "App",
            "Vendor\\Cache",
            "Vendor\\A\\CacheInterface, Vendor\\B\\CacheInterface",
        );
        assert!(!body.contains("use "));
        assert!(body.contains("extends \\Vendor\\Cache implements \\Vendor\\A\\CacheInterface, \\Vendor\\B\\CacheInterface"));
    }

    #[test]
    fn manual_short_names_reserve_alias_without_resolution() {
        let body = render("class", "Item", "App", "Base", "Logger, Vendor\\Logger");
        assert!(!body.contains("use "));
        assert!(body.contains("extends Base implements Logger, \\Vendor\\Logger"));
        assert!(render("class", "Item", "App", "\\Exception", "").contains("use Exception;"));
    }

    #[test]
    fn interface_template_and_trait_enum_unchanged() {
        assert_eq!(
            render(
                "interface",
                "ExtendedCache",
                "App\\Contracts",
                "Psr\\SimpleCache\\CacheInterface",
                "Ignored"
            ),
            "<?php\n\nnamespace App\\Contracts;\n\nuse Psr\\SimpleCache\\CacheInterface;\n\ninterface ExtendedCache extends CacheInterface\n{\n}\n"
        );
        for keyword in ["trait", "enum"] {
            assert_eq!(
                render(keyword, "Item", "App", "Vendor\\Base", "Vendor\\Contract"),
                format!("<?php\n\nnamespace App;\n\n{keyword} Item\n{{\n}}\n")
            );
        }
        assert_eq!(
            render("class", "Item", "", "", ""),
            "<?php\n\nclass Item\n{\n}\n"
        );
    }
}
