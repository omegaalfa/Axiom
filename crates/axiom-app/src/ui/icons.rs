use std::{borrow::Cow, path::Path};

use gpui::{AssetSource, Rgba, Styled, Svg, svg};

use super::metrics;

const PROJECT_ICON: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M3.5 6.5h6l2 2h9v9.5a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2z"/><path d="M3.5 9h17"/></svg>"#;
const SEARCH_ICON: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"><circle cx="10.5" cy="10.5" r="6.5"/><path d="m15.5 15.5 4.5 4.5"/></svg>"#;
const PROBLEMS_ICON: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3.5 21 20H3z"/><path d="M12 9v5"/><path d="M12 17.2v.1"/></svg>"#;
const TERMINAL_ICON: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="m4 6 6 6-6 6"/><path d="M13 18h7"/></svg>"#;

pub struct AxiomAssets;

impl AssetSource for AxiomAssets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        let source = match path {
            "icons/activity-project.svg" => PROJECT_ICON,
            "icons/activity-search.svg" => SEARCH_ICON,
            "icons/activity-problems.svg" => PROBLEMS_ICON,
            "icons/activity-terminal.svg" => TERMINAL_ICON,
            _ => return Ok(None),
        };
        Ok(Some(Cow::Borrowed(source.as_bytes())))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<gpui::SharedString>> {
        Ok(if path == "icons" {
            vec![
                "activity-project.svg".into(),
                "activity-search.svg".into(),
                "activity-problems.svg".into(),
                "activity-terminal.svg".into(),
            ]
        } else {
            Vec::new()
        })
    }
}

#[derive(Clone, Copy)]
pub enum ActivityIcon {
    Project,
    Search,
    Problems,
    Terminal,
}

pub fn activity_icon(icon: ActivityIcon, color: Rgba) -> Svg {
    let path = match icon {
        ActivityIcon::Project => "icons/activity-project.svg",
        ActivityIcon::Search => "icons/activity-search.svg",
        ActivityIcon::Problems => "icons/activity-problems.svg",
        ActivityIcon::Terminal => "icons/activity-terminal.svg",
    };
    svg().path(path).size(metrics().icon_size).text_color(color)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileIcon {
    Folder,
    FolderOpen,
    Php,
    Json,
    Yaml,
    Markdown,
    Env,
    Html,
    Css,
    JavaScript,
    TypeScript,
    Text,
    Unknown,
}

impl FileIcon {
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Folder => "▸",
            Self::FolderOpen => "▾",
            Self::Php => "φ",
            Self::Json => "{}",
            Self::Yaml => "Y",
            Self::Markdown => "M",
            Self::Env => "E",
            Self::Html => "<>",
            Self::Css => "#",
            Self::JavaScript => "J",
            Self::TypeScript => "T",
            Self::Text => "≡",
            Self::Unknown => "·",
        }
    }
}

pub fn file_icon(path: &Path, directory: bool, expanded: bool) -> FileIcon {
    if directory {
        return if expanded {
            FileIcon::FolderOpen
        } else {
            FileIcon::Folder
        };
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if name.starts_with(".env") {
        return FileIcon::Env;
    }
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("php" | "phtml" | "inc") => FileIcon::Php,
        Some("json" | "jsonc") => FileIcon::Json,
        Some("yaml" | "yml") => FileIcon::Yaml,
        Some("md" | "markdown") => FileIcon::Markdown,
        Some("html" | "htm") => FileIcon::Html,
        Some("css" | "scss" | "sass" | "less") => FileIcon::Css,
        Some("js" | "mjs" | "cjs" | "jsx") => FileIcon::JavaScript,
        Some("ts" | "tsx") => FileIcon::TypeScript,
        Some("txt" | "log" | "ini" | "conf") => FileIcon::Text,
        _ => FileIcon::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_core_project_file_types() {
        assert_eq!(file_icon(Path::new("a.php"), false, false), FileIcon::Php);
        assert_eq!(
            file_icon(Path::new(".env.local"), false, false),
            FileIcon::Env
        );
        assert_eq!(file_icon(Path::new("a.yaml"), false, false), FileIcon::Yaml);
        assert_eq!(
            file_icon(Path::new("a.tsx"), false, false),
            FileIcon::TypeScript
        );
        assert_eq!(
            file_icon(Path::new("src"), true, true),
            FileIcon::FolderOpen
        );
    }
}
