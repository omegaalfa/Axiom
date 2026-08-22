use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandDescriptor {
    pub id: String,
    pub title: String,
    pub description: String,
    pub category: String,
    pub default_shortcut: Option<String>,
    pub context: String,
}

pub fn registry() -> Vec<CommandDescriptor> {
    vec![
        command(
            "editor.undo",
            "Undo",
            "Undoes the last editor change.",
            "Editor",
            "ctrl-z",
        ),
        command(
            "editor.redo",
            "Redo",
            "Redoes the last undone editor change.",
            "Editor",
            "ctrl-y",
        ),
        command(
            "editor.select_all",
            "Select All",
            "Selects the entire document.",
            "Editor",
            "ctrl-a",
        ),
        command(
            "editor.save",
            "Save",
            "Saves the active document.",
            "Editor",
            "ctrl-s",
        ),
        command(
            "editor.find",
            "Find",
            "Finds text in the active document.",
            "Editor",
            "ctrl-f",
        ),
        command(
            "editor.copy",
            "Copy",
            "Copies the selected text.",
            "Editor",
            "ctrl-c",
        ),
        command(
            "editor.cut",
            "Cut",
            "Cuts the selected text.",
            "Editor",
            "ctrl-x",
        ),
        command(
            "editor.paste",
            "Paste",
            "Pastes clipboard text.",
            "Editor",
            "ctrl-v",
        ),
        command(
            "editor.reformat",
            "Reformat Code",
            "Reformats the current PHP document.",
            "Code",
            "ctrl-alt-l",
        ),
        command(
            "navigate.definition",
            "Go to Definition",
            "Navigates to the declaration under the caret.",
            "Navigate",
            "ctrl-b",
        ),
        command(
            "navigate.class",
            "Go to Class",
            "Searches indexed PHP classes and opens their declaration.",
            "Navigate",
            "ctrl-n",
        ),
        command(
            "navigate.symbol",
            "Go to Symbol",
            "Searches classes, methods and functions in the project.",
            "Navigate",
            "ctrl-shift-alt-o",
        ),
        command(
            "navigate.back",
            "Back",
            "Returns to the previous navigation location.",
            "Navigate",
            "alt-left",
        ),
        command(
            "navigate.forward",
            "Forward",
            "Moves to the next navigation location.",
            "Navigate",
            "alt-right",
        ),
        command(
            "code.completion",
            "Completion",
            "Shows PHP completion proposals.",
            "Code",
            "ctrl-space",
        ),
        command(
            "terminal.toggle",
            "Terminal",
            "Shows or hides the integrated terminal.",
            "Tool Windows",
            "ctrl-`",
        ),
        command(
            "workspace.commands",
            "Command Palette",
            "Searches and runs Axiom commands.",
            "Help",
            "ctrl-shift-p",
        ),
        command(
            "help.features",
            "Axiom Features",
            "Shows implemented features and their current shortcuts.",
            "Help",
            "",
        ),
        command(
            "settings.open",
            "Settings",
            "Opens Axiom settings and the configurable keymap.",
            "Help",
            "",
        ),
        command(
            "project.open_project",
            "Open Project",
            "Opens a project directory.",
            "Project",
            "ctrl-shift-o",
        ),
        command(
            "project.open_file",
            "Open File",
            "Opens a file in the editor.",
            "Project",
            "ctrl-o",
        ),
    ]
}

fn command(
    id: &str,
    title: &str,
    description: &str,
    category: &str,
    shortcut: &str,
) -> CommandDescriptor {
    CommandDescriptor {
        id: id.into(),
        title: title.into(),
        description: description.into(),
        category: category.into(),
        default_shortcut: Some(shortcut.into()),
        context: "global".into(),
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KeymapFile {
    #[serde(default)]
    pub bindings: HashMap<String, Option<String>>,
}

#[derive(Debug, Clone)]
pub struct Keymap {
    descriptors: Vec<CommandDescriptor>,
    overrides: HashMap<String, Option<String>>,
}

impl Default for Keymap {
    fn default() -> Self {
        Self {
            descriptors: registry(),
            overrides: HashMap::new(),
        }
    }
}

impl Keymap {
    pub fn user_path() -> Option<PathBuf> {
        let current = directories::ProjectDirs::from("org", "Axiom", "Axiom")
            .map(|dirs| dirs.config_dir().join("keymap.json"))?;
        if !current.exists()
            && let Some(legacy) = directories::ProjectDirs::from("org", "RustStorm", "RustStorm")
                .map(|dirs| dirs.config_dir().join("keymap.json"))
            && legacy.is_file()
        {
            if let Some(parent) = current.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::copy(legacy, &current);
        }
        Some(current)
    }

    pub fn load_user() -> Self {
        Self::user_path().map(Self::load).unwrap_or_default()
    }

    pub fn persist_user(&self) -> std::io::Result<()> {
        Self::user_path()
            .map(|path| self.save(path))
            .unwrap_or(Ok(()))
    }

    pub fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let overrides = fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<KeymapFile>(&text).ok())
            .map(|file| file.bindings)
            .unwrap_or_default();
        Self {
            descriptors: registry(),
            overrides,
        }
    }

    pub fn save(&self, path: impl Into<PathBuf>) -> std::io::Result<()> {
        let path = path.into();
        let file = KeymapFile {
            bindings: self.overrides.clone(),
        };
        let text = serde_json::to_string_pretty(&file).expect("keymap serialization");
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, text)
    }

    pub fn commands(&self) -> &[CommandDescriptor] {
        &self.descriptors
    }

    pub fn search(&self, query: &str) -> Vec<&CommandDescriptor> {
        let query = query.to_ascii_lowercase();
        self.descriptors
            .iter()
            .filter(|command| {
                command.title.to_ascii_lowercase().contains(&query)
                    || command.description.to_ascii_lowercase().contains(&query)
                    || command.category.to_ascii_lowercase().contains(&query)
            })
            .collect()
    }

    pub fn shortcut(&self, id: &str) -> Option<&str> {
        self.overrides
            .get(id)
            .and_then(|value| value.as_deref())
            .or_else(|| {
                self.descriptors
                    .iter()
                    .find(|command| command.id == id)
                    .and_then(|command| command.default_shortcut.as_deref())
            })
    }

    pub fn set_shortcut(&mut self, id: &str, shortcut: Option<String>) -> Result<(), String> {
        if let Some(other) = self
            .descriptors
            .iter()
            .find(|command| command.id != id && self.shortcut(&command.id) == shortcut.as_deref())
        {
            return Err(format!(
                "{} is already assigned to {}",
                shortcut.unwrap_or_default(),
                other.title
            ));
        }
        self.overrides.insert(id.into(), shortcut);
        Ok(())
    }

    pub fn replace_shortcut(&mut self, id: &str, shortcut: Option<String>) {
        if let Some(value) = shortcut.as_deref() {
            let conflicts: Vec<String> = self
                .descriptors
                .iter()
                .filter(|command| command.id != id && self.shortcut(&command.id) == Some(value))
                .map(|command| command.id.clone())
                .collect();
            for conflict in conflicts {
                self.overrides.insert(conflict, None);
            }
        }
        self.overrides.insert(id.to_owned(), shortcut);
    }

    pub fn reset(&mut self, id: &str) {
        self.overrides.remove(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registry_ids_are_unique_and_search_uses_description() {
        let keymap = Keymap::default();
        let mut ids = std::collections::HashSet::new();
        assert!(
            keymap
                .commands()
                .iter()
                .all(|command| ids.insert(&command.id))
        );
        assert!(
            keymap
                .search("declaration")
                .iter()
                .any(|command| command.id == "navigate.definition")
        );
    }
    #[test]
    fn overrides_conflicts_and_reset_are_deterministic() {
        let mut keymap = Keymap::default();
        assert!(
            keymap
                .set_shortcut("editor.reformat", Some("ctrl-shift-l".into()))
                .is_ok()
        );
        assert!(
            keymap
                .set_shortcut("editor.copy", Some("ctrl-shift-l".into()))
                .is_err()
        );
        keymap.reset("editor.reformat");
        assert_eq!(keymap.shortcut("editor.reformat"), Some("ctrl-alt-l"));
    }

    #[test]
    fn project_open_commands_are_distinct_and_explicit() {
        let keymap = Keymap::default();
        let project = keymap
            .commands()
            .iter()
            .find(|command| command.id == "project.open_project")
            .expect("project command");
        let file = keymap
            .commands()
            .iter()
            .find(|command| command.id == "project.open_file")
            .expect("file command");
        assert_ne!(project.id, file.id);
        assert!(
            project
                .description
                .to_ascii_lowercase()
                .contains("directory")
        );
        assert!(file.description.to_ascii_lowercase().contains("file"));
    }
}
