use axiom_index::{ProjectSymbolIndex, ProjectSymbolKind};
use serde_json::json;
use std::sync::{Arc, RwLock};

use super::{ToolError, ToolMetadata, ToolName, ToolOutput, ToolResult};

pub(crate) const FIND_SYMBOL_MAX_RESULTS: usize = 100;

#[derive(Clone, Debug)]
pub(crate) struct FindSymbolTool {
    index: Option<Arc<RwLock<ProjectSymbolIndex>>>,
    workspace_root: std::path::PathBuf,
}

impl FindSymbolTool {
    pub(crate) fn new(
        workspace_root: std::path::PathBuf,
        index: Option<Arc<RwLock<ProjectSymbolIndex>>>,
    ) -> Self {
        Self {
            index,
            workspace_root: std::fs::canonicalize(&workspace_root).unwrap_or(workspace_root),
        }
    }

    pub(crate) fn execute(
        &self,
        query: String,
        kind: Option<String>,
        limit: Option<usize>,
    ) -> ToolResult {
        let result = self
            .index
            .as_ref()
            .ok_or_else(|| ToolError::InvalidArguments("project symbol index unavailable".into()))
            .and_then(|index| {
                let index = index.read().map_err(|_| ToolError::Io {
                    path: ".".into(),
                    message: "project symbol index unavailable".into(),
                })?;
                find_symbols(&index, &self.workspace_root, &query, kind.as_deref(), limit)
            });
        ToolResult {
            tool: ToolName::FindSymbol,
            result,
        }
    }
}

fn find_symbols(
    index: &ProjectSymbolIndex,
    workspace_root: &std::path::Path,
    query: &str,
    kind: Option<&str>,
    limit: Option<usize>,
) -> Result<ToolOutput, ToolError> {
    if query.trim().is_empty() {
        return Err(ToolError::InvalidArguments("query must be a string".into()));
    }
    let limit = limit
        .unwrap_or(FIND_SYMBOL_MAX_RESULTS)
        .clamp(1, FIND_SYMBOL_MAX_RESULTS);
    let kind = kind
        .map(|value| {
            parse_kind(value).ok_or_else(|| {
                ToolError::InvalidArguments(format!("unsupported symbol kind: {value}"))
            })
        })
        .transpose()?;
    let query = query.trim();
    let mut matches = index
        .search_prefix(query)
        .into_iter()
        .filter(|symbol| kind.is_none_or(|kind| symbol.kind == kind))
        .collect::<Vec<_>>();
    matches.sort_by_key(|symbol| {
        (
            !(symbol.name == query || symbol.fully_qualified_name == query),
            !symbol.name.starts_with(query),
            symbol.name.to_ascii_lowercase(),
            symbol.file.clone(),
            symbol.range.start,
        )
    });
    let truncated = matches.len() > limit;
    let matches = matches
        .into_iter()
        .take(limit)
        .map(|symbol| {
            let (line, column) = index.symbol_location(symbol).unwrap_or((1, 1));
            let path = symbol
                .file
                .strip_prefix(workspace_root)
                .unwrap_or(&symbol.file)
                .to_string_lossy()
                .replace('\\', "/");
            json!({
                "name": symbol.name,
                "kind": symbol_kind_name(symbol.kind),
                "path": path,
                "line": line,
                "column": column,
            })
        })
        .collect::<Vec<_>>();
    Ok(ToolOutput {
        content: json!({"matches": matches, "truncated": truncated}).to_string(),
        metadata: ToolMetadata {
            path: ".".into(),
            bytes: 0,
            range: None,
            source_bytes: None,
            truncated,
            fingerprint: None,
        },
    })
}

fn parse_kind(value: &str) -> Option<ProjectSymbolKind> {
    Some(match value.to_ascii_lowercase().as_str() {
        "class" => ProjectSymbolKind::Class,
        "interface" => ProjectSymbolKind::Interface,
        "trait" => ProjectSymbolKind::Trait,
        "enum" => ProjectSymbolKind::Enum,
        "function" => ProjectSymbolKind::Function,
        "method" => ProjectSymbolKind::Method,
        "property" => ProjectSymbolKind::Property,
        "constant" => ProjectSymbolKind::Constant,
        "class_constant" | "classconstant" => ProjectSymbolKind::ClassConstant,
        "enum_case" | "enumcase" => ProjectSymbolKind::EnumCase,
        _ => return None,
    })
}

fn symbol_kind_name(kind: ProjectSymbolKind) -> &'static str {
    match kind {
        ProjectSymbolKind::Class => "class",
        ProjectSymbolKind::Interface => "interface",
        ProjectSymbolKind::Trait => "trait",
        ProjectSymbolKind::Enum => "enum",
        ProjectSymbolKind::Function => "function",
        ProjectSymbolKind::Method => "method",
        ProjectSymbolKind::Property => "property",
        ProjectSymbolKind::Constant => "constant",
        ProjectSymbolKind::ClassConstant => "class_constant",
        ProjectSymbolKind::EnumCase => "enum_case",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn tool() -> (tempfile::TempDir, FindSymbolTool) {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("App")).unwrap();
        fs::write(
            dir.path().join("App/ProductService.php"),
            "<?php\nclass ProductService {\n    public function calculateTotal() {}\n}\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("App/OtherService.php"),
            "<?php\nclass ProductService {\n}\nfunction calculateTotal() {}\n",
        )
        .unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let index = Arc::new(RwLock::new(index));
        let tool = FindSymbolTool::new(dir.path().to_path_buf(), Some(index));
        (dir, tool)
    }

    #[test]
    fn returns_exact_and_method_matches_from_the_existing_index() {
        let (_dir, tool) = tool();
        let result = tool.execute("ProductService".into(), Some("class".into()), None);
        let output = result.result.unwrap();
        let value: serde_json::Value = serde_json::from_str(&output.content).unwrap();
        assert_eq!(value["matches"][0]["name"], "ProductService");
        assert_eq!(value["matches"][0]["path"], "App/OtherService.php");
        assert_eq!(value["matches"][0]["line"], 2);
        assert_eq!(value["matches"][0]["column"], 1);
        assert_eq!(value["truncated"], false);

        let result = tool.execute("calculateTotal".into(), Some("method".into()), None);
        let value: serde_json::Value =
            serde_json::from_str(&result.result.unwrap().content).unwrap();
        assert_eq!(value["matches"][0]["kind"], "method");
        assert_eq!(value["matches"][0]["line"], 3);
        assert_eq!(value["matches"][0]["column"], 21);
    }

    #[test]
    fn returns_deterministic_duplicate_matches_and_truncates() {
        let (_dir, tool) = tool();
        let result = tool.execute("ProductService".into(), None, Some(1));
        let value: serde_json::Value =
            serde_json::from_str(&result.result.unwrap().content).unwrap();
        assert_eq!(value["matches"].as_array().unwrap().len(), 1);
        assert_eq!(value["truncated"], true);
        assert_eq!(value["matches"][0]["path"], "App/OtherService.php");
    }

    #[test]
    fn no_match_and_invalid_kind_fail_safely() {
        let (_dir, tool) = tool();
        let result = tool.execute("MissingSymbol".into(), None, None);
        let value: serde_json::Value =
            serde_json::from_str(&result.result.unwrap().content).unwrap();
        assert!(value["matches"].as_array().unwrap().is_empty());
        assert!(
            tool.execute("ProductService".into(), Some("module".into()), None)
                .result
                .is_err()
        );
    }
}
