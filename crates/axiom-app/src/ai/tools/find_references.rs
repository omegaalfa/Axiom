use axiom_index::{
    FindUsagesOptions, PersistentFileKey, ProjectSymbolIndex, ProjectSymbolKind, SemanticEngine,
};
use serde_json::json;
use std::{path::PathBuf, sync::{Arc, RwLock}};

use super::{ToolError, ToolMetadata, ToolName, ToolOutput, ToolResult};

pub(crate) const FIND_REFERENCES_MAX_RESULTS: usize = 100;

#[derive(Clone, Debug)]
pub(crate) struct FindReferencesTool {
    index: Option<Arc<RwLock<ProjectSymbolIndex>>>,
    semantic_engine: Option<Arc<SemanticEngine>>,
    workspace_root: PathBuf,
}

impl FindReferencesTool {
    pub(crate) fn new(
        workspace_root: PathBuf,
        index: Option<Arc<RwLock<ProjectSymbolIndex>>>,
        semantic_engine: Option<Arc<SemanticEngine>>,
    ) -> Self {
        Self {
            index,
            semantic_engine,
            workspace_root: std::fs::canonicalize(&workspace_root).unwrap_or(workspace_root),
        }
    }

    pub(crate) fn with_semantic_engine(mut self, engine: Option<Arc<SemanticEngine>>) -> Self {
        self.semantic_engine = engine;
        self
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
                let engine = self.semantic_engine.as_ref().ok_or_else(|| {
                    ToolError::InvalidArguments("semantic reference index unavailable".into())
                })?;
                find_references(
                    &index,
                    &engine.snapshot(),
                    &self.workspace_root,
                    &query,
                    kind.as_deref(),
                    limit,
                )
            });
        ToolResult {
            tool: ToolName::FindReferences,
            result,
        }
    }
}

fn find_references(
    index: &ProjectSymbolIndex,
    snapshot: &axiom_index::SemanticSnapshot,
    workspace_root: &std::path::Path,
    query: &str,
    kind: Option<&str>,
    limit: Option<usize>,
) -> Result<ToolOutput, ToolError> {
    if query.trim().is_empty() {
        return Err(ToolError::InvalidArguments("query must be a string".into()));
    }
    let limit = limit
        .unwrap_or(FIND_REFERENCES_MAX_RESULTS)
        .clamp(1, FIND_REFERENCES_MAX_RESULTS);
    let kind = kind
        .map(|value| {
            parse_kind(value).ok_or_else(|| {
                ToolError::InvalidArguments(format!("unsupported symbol kind: {value}"))
            })
        })
        .transpose()?;
    let query = query.trim();
    let mut symbols = index
        .search_prefix(query)
        .into_iter()
        .filter(|symbol| kind.is_none_or(|kind| symbol.kind == kind))
        .collect::<Vec<_>>();
    symbols.sort_by_key(|symbol| {
        (
            !(symbol.name == query || symbol.fully_qualified_name == query),
            !symbol.name.starts_with(query),
            symbol.name.to_ascii_lowercase(),
            symbol.file.clone(),
            symbol.range.start,
        )
    });
    let Some(project_symbol) = symbols.first() else {
        return Ok(output(Vec::new(), false));
    };
    let file_key = PersistentFileKey::workspace(&project_symbol.file);
    let semantic_symbol = snapshot
        .symbols_for_fqn(&project_symbol.fully_qualified_name)
        .iter()
        .filter_map(|id| snapshot.symbol(*id))
        .find(|symbol| {
            symbol.kind == project_symbol.kind
                && symbol.name == project_symbol.name
                && symbol.key.file == file_key
        })
        .ok_or_else(|| ToolError::InvalidArguments("semantic symbol is unavailable".into()))?;
    let mut usages = snapshot
        .find_usages_by_key(&semantic_symbol.key, FindUsagesOptions::default())
        .usages;
    usages.sort_by_key(|usage| (usage.file.normalized_path.clone(), usage.span.start));
    let truncated = usages.len() > limit;
    let references = usages
        .into_iter()
        .take(limit)
        .filter_map(|usage| {
            let path = PathBuf::from(&usage.file.normalized_path);
            let path = if path.is_absolute() {
                path
            } else {
                workspace_root.join(path)
            };
            let text = std::fs::read_to_string(&path).ok()?;
            let (line, column) = one_based_position(&text, usage.span.start);
            let relative = path
                .strip_prefix(workspace_root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            Some(json!({"path": relative, "line": line, "column": column}))
        })
        .collect::<Vec<_>>();
    Ok(output(references, truncated))
}

fn output(references: Vec<serde_json::Value>, truncated: bool) -> ToolOutput {
    ToolOutput {
        content: json!({"references": references, "truncated": truncated}).to_string(),
        metadata: ToolMetadata {
            path: ".".into(),
            bytes: 0,
            range: None,
            source_bytes: None,
            truncated,
            fingerprint: None,
        },
    }
}

fn one_based_position(text: &str, offset: usize) -> (usize, usize) {
    let prefix = &text[..offset.min(text.len())];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rsplit_once('\n')
        .map_or(prefix, |(_, line)| line)
        .chars()
        .count()
        + 1;
    (line, column)
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

#[cfg(test)]
mod tests {
    use super::*;
    use axiom_index::{SemanticRevision, SemanticSnapshot};
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn returns_indexed_class_references_with_bounded_locations() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("Product.php"),
            "<?php\nclass Product {}\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("Use.php"),
            "<?php\nfunction build() { return new Product(); }\n",
        )
        .unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(1));
        let tool = FindReferencesTool::new(
            dir.path().to_path_buf(),
            Some(Arc::new(RwLock::new(index))),
            Some(Arc::new(SemanticEngine::from_snapshot(snapshot))),
        );

        let result = tool.execute("Product".into(), Some("class".into()), Some(1));
        let output: serde_json::Value = serde_json::from_str(&result.result.unwrap().content).unwrap();
        assert_eq!(output["references"].as_array().unwrap().len(), 1);
        assert_eq!(output["references"][0]["path"], "Use.php");
        assert_eq!(output["references"][0]["line"], 2);
        assert_eq!(output["references"][0]["column"], 31);
        assert_eq!(output["truncated"], false);
    }
}
