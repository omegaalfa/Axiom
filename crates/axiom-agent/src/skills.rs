//! Workspace-local, human-readable Skills for contextual Agent procedures.

use std::{
    fs,
    path::{Path, PathBuf},
};

const MAX_SKILLS: usize = 8;
const MAX_SKILL_NAME_CHARS: usize = 80;
const MAX_SKILL_DESCRIPTION_CHARS: usize = 240;
const MAX_SKILL_INSTRUCTION_CHARS: usize = 2_000;
const MAX_SKILL_CONTEXT_CHARS: usize = 6_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub triggers: Vec<String>,
    pub instructions: String,
    pub parameter_constraints: Vec<String>,
    pub source: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillLoadError {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SkillCatalog {
    skills: Vec<Skill>,
    errors: Vec<SkillLoadError>,
}

impl SkillCatalog {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn discover(workspace_root: &Path) -> Self {
        let directory = workspace_root.join(".axiom").join("skills");
        let canonical_root = match fs::canonicalize(workspace_root) {
            Ok(path) => path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(error) => return Self::with_error(workspace_root.to_path_buf(), error.to_string()),
        };
        let canonical_directory = match fs::canonicalize(&directory) {
            Ok(path) if is_within(&path, &canonical_root) => path,
            Ok(path) => {
                return Self::with_error(
                    directory,
                    format!("skills directory escapes workspace: {}", path.display()),
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(error) => return Self::with_error(directory, error.to_string()),
        };

        let mut catalog = Self::default();
        let mut paths = Vec::new();
        let entries = match fs::read_dir(&canonical_directory) {
            Ok(entries) => entries,
            Err(error) => {
                catalog.errors.push(SkillLoadError {
                    path: directory,
                    message: error.to_string(),
                });
                return catalog;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    catalog.errors.push(SkillLoadError {
                        path: canonical_directory.clone(),
                        message: error.to_string(),
                    });
                    continue;
                }
            };
            let entry_path = entry.path();
            let canonical_entry = match fs::canonicalize(&entry_path) {
                Ok(path) => path,
                Err(error) => {
                    catalog.errors.push(SkillLoadError {
                        path: entry_path,
                        message: error.to_string(),
                    });
                    continue;
                }
            };
            if !is_within(&canonical_entry, &canonical_root)
                || !is_within(&canonical_entry, &canonical_directory)
            {
                catalog.errors.push(SkillLoadError {
                    path: entry_path,
                    message: "skill path escapes workspace skill directory".into(),
                });
                continue;
            }
            if canonical_entry.is_file()
                && canonical_entry
                    .extension()
                    .and_then(|extension| extension.to_str())
                    == Some("md")
            {
                paths.push(canonical_entry);
            } else if canonical_entry.is_dir() {
                let skill_path = canonical_entry.join("SKILL.md");
                if let Ok(canonical_skill_path) = fs::canonicalize(&skill_path) {
                    if is_within(&canonical_skill_path, &canonical_root)
                        && is_within(&canonical_skill_path, &canonical_directory)
                        && canonical_skill_path.is_file()
                    {
                        paths.push(canonical_skill_path);
                    } else {
                        catalog.errors.push(SkillLoadError {
                            path: skill_path,
                            message: "skill path escapes workspace skill directory".into(),
                        });
                    }
                }
            }
        }
        paths.sort();
        for path in paths {
            match fs::read_to_string(&path).and_then(|content| {
                Skill::parse(&content, path.clone())
                    .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
            }) {
                Ok(skill) => {
                    if catalog.skills.len() < MAX_SKILLS {
                        catalog.skills.push(skill);
                    }
                }
                Err(error) => catalog.errors.push(SkillLoadError {
                    path,
                    message: error.to_string(),
                }),
            }
        }
        catalog
    }

    fn with_error(path: PathBuf, message: String) -> Self {
        Self {
            skills: Vec::new(),
            errors: vec![SkillLoadError { path, message }],
        }
    }

    pub fn skills(&self) -> &[Skill] {
        &self.skills
    }

    pub fn errors(&self) -> &[SkillLoadError] {
        &self.errors
    }

    pub fn active_for(&self, task: &str) -> Vec<&Skill> {
        let task = task.to_ascii_lowercase();
        let mut matched = self
            .skills
            .iter()
            .filter_map(|skill| {
                let score = skill
                    .triggers
                    .iter()
                    .filter(|trigger| {
                        !trigger.trim().is_empty() && task.contains(&trigger.to_ascii_lowercase())
                    })
                    .count();
                (score > 0).then_some((score, skill))
            })
            .collect::<Vec<_>>();
        matched.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| left.1.name.cmp(&right.1.name))
        });
        matched.into_iter().map(|(_, skill)| skill).collect()
    }

    pub fn render_active_context(&self, task: &str) -> Option<String> {
        let mut context =
            String::from("Active Skills (workspace-local procedures; use only when relevant):\n");
        let mut included = 0;
        for skill in self.active_for(task) {
            let block = format!(
                "- {}: {}\n  Instructions: {}\n",
                skill.name, skill.description, skill.instructions
            );
            if context.chars().count() + block.chars().count() > MAX_SKILL_CONTEXT_CHARS {
                break;
            }
            context.push_str(&block);
            included += 1;
        }
        (included > 0).then_some(context)
    }
}

fn is_within(path: &Path, root: &Path) -> bool {
    path == root || path.starts_with(root)
}

impl Skill {
    pub fn parse(content: &str, source: PathBuf) -> Result<Self, String> {
        let mut lines = content.lines();
        if lines.next().map(str::trim) != Some("---") {
            return Err("skill must start with YAML-like front matter".into());
        }
        let mut fields = std::collections::BTreeMap::<String, Vec<String>>::new();
        let mut list_field: Option<String> = None;
        let mut front_matter_closed = false;
        for line in &mut lines {
            let line = line.trim();
            if line == "---" {
                front_matter_closed = true;
                break;
            }
            if let Some(item) = line.strip_prefix("- ") {
                let item = item.trim().trim_matches(['"', '\'']).trim();
                let Some(key) = list_field.as_ref() else {
                    return Err("list item must follow a supported list field".into());
                };
                if item.is_empty() {
                    return Err("front matter list items must not be empty".into());
                }
                fields.entry(key.clone()).or_default().push(item.to_owned());
                continue;
            }
            let Some((key, value)) = line.split_once(':') else {
                return Err("invalid front matter line".into());
            };
            let key = key.trim().to_ascii_lowercase();
            let value = value.trim().trim_matches(['"', '\'']).trim().to_string();
            let supports_multiline_list = matches!(key.as_str(), "triggers" | "parameter_constraints");
            if key.is_empty() || (value.is_empty() && !supports_multiline_list) {
                return Err("front matter fields must not be empty".into());
            }
            if value.is_empty() {
                list_field = Some(key);
            } else {
                list_field = None;
                fields.entry(key).or_default().push(value);
            }
        }
        if !front_matter_closed {
            return Err("skill front matter is not closed".into());
        }
        let name = required_field(&fields, "name")?;
        let description = required_field(&fields, "description")?;
        let triggers = split_list_fields(fields.get("triggers"));
        let parameter_constraints = split_list_fields(fields.get("parameter_constraints"));
        let instructions = lines
            .collect::<Vec<_>>()
            .join(" ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if instructions.is_empty() {
            return Err("skill instructions must not be empty".into());
        }
        if name.chars().count() > MAX_SKILL_NAME_CHARS
            || description.chars().count() > MAX_SKILL_DESCRIPTION_CHARS
            || instructions.chars().count() > MAX_SKILL_INSTRUCTION_CHARS
        {
            return Err("skill field exceeds its size limit".into());
        }
        if triggers.is_empty() {
            return Err("skill must define at least one trigger".into());
        }
        Ok(Self {
            name,
            description,
            triggers,
            instructions,
            parameter_constraints,
            source,
        })
    }
}

fn required_field(
    fields: &std::collections::BTreeMap<String, Vec<String>>,
    key: &str,
) -> Result<String, String> {
    fields
        .get(key)
        .and_then(|values| values.first())
        .cloned()
        .ok_or_else(|| format!("missing required field: {key}"))
}

fn split_list_fields(values: Option<&Vec<String>>) -> Vec<String> {
    values
        .into_iter()
        .flat_map(|values| values.iter())
        .flat_map(|value| value.trim_matches(['[', ']']).split(','))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(name: &str, trigger: &str) -> String {
        format!(
            "---\nname: {name}\ndescription: procedure\ntriggers: {trigger}\n---\nInspect the relevant files before changing code."
        )
    }

    #[test]
    fn parses_valid_and_rejects_invalid_skill_files() {
        let parsed =
            Skill::parse(&skill("Rust review", "review"), PathBuf::from("review.md")).unwrap();
        assert_eq!(parsed.name, "Rust review");
        let listed = Skill::parse(
            "---\nname: Listed\ndescription: procedure\ntriggers: [review, tests]\n---\nInspect files.",
            PathBuf::from("listed.md"),
        )
        .unwrap();
        assert_eq!(listed.triggers, vec!["review", "tests"]);
        assert!(Skill::parse("name: broken", PathBuf::from("broken.md")).is_err());
        assert!(
            Skill::parse(
                &format!(
                    "---\nname: broken\ndescription: no triggers\n---\n{}",
                    "instructions"
                ),
                PathBuf::from("broken.md")
            )
            .is_err()
        );
    }

    #[test]
    fn parses_multiline_yaml_lists_in_front_matter() {
        let parsed = Skill::parse(
            "---\nname: Listed\ndescription: procedure\ntriggers:\n  - review\n  - tests\nparameter_constraints:\n  - php\n---\nInspect files.",
            PathBuf::from("listed.md"),
        )
        .unwrap();
        assert_eq!(parsed.triggers, vec!["review", "tests"]);
        assert_eq!(parsed.parameter_constraints, vec!["php"]);
    }

    #[test]
    fn discovers_skills_and_keeps_malformed_files_isolated() {
        let root = std::env::temp_dir().join(format!("axiom-skills-{}", std::process::id()));
        let directory = root.join(".axiom").join("skills");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("valid.md"), skill("Review", "review")).unwrap();
        fs::create_dir_all(directory.join("nested")).unwrap();
        fs::write(
            directory.join("nested").join("SKILL.md"),
            skill("Nested", "nested"),
        )
        .unwrap();
        fs::write(directory.join("broken.md"), "not a skill").unwrap();
        let catalog = SkillCatalog::discover(&root);
        assert_eq!(catalog.skills().len(), 2);
        assert_eq!(catalog.errors().len(), 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn activation_is_contextual_and_deterministic() {
        let root =
            std::env::temp_dir().join(format!("axiom-skills-activation-{}", std::process::id()));
        let directory = root.join(".axiom").join("skills");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("review.md"), skill("Review", "review")).unwrap();
        fs::write(directory.join("test.md"), skill("Tests", "tests")).unwrap();
        let catalog = SkillCatalog::discover(&root);
        assert_eq!(catalog.active_for("Please review the tests").len(), 2);
        assert!(
            catalog
                .render_active_context("Please review the tests")
                .unwrap()
                .contains("Active Skills")
        );
        assert!(catalog.active_for("general question").is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn matching_skills_are_bounded_and_rendered_deterministically() {
        let root =
            std::env::temp_dir().join(format!("axiom-skills-bounded-{}", std::process::id()));
        let directory = root.join(".axiom").join("skills");
        fs::create_dir_all(&directory).unwrap();
        for index in 0..8 {
            fs::write(
                directory.join(format!("skill-{index}.md")),
                format!(
                    "---\nname: Skill {index}\ndescription: bounded\ntriggers: shared\n---\n{}",
                    "Use this procedure safely. ".repeat(50)
                ),
            )
            .unwrap();
        }
        let catalog = SkillCatalog::discover(&root);
        let first = catalog.render_active_context("shared task").unwrap();
        assert!(catalog.active_for("shared task").len() >= 5);
        assert!(first.chars().count() <= MAX_SKILL_CONTEXT_CHARS);
        assert_eq!(first, catalog.render_active_context("shared task").unwrap());
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn discovery_rejects_skill_symlinks_outside_workspace() {
        let root =
            std::env::temp_dir().join(format!("axiom-skills-boundary-{}", std::process::id()));
        let external =
            std::env::temp_dir().join(format!("axiom-skills-external-{}", std::process::id()));
        let directory = root.join(".axiom").join("skills");
        fs::create_dir_all(&directory).unwrap();
        fs::write(&external, skill("External", "escape")).unwrap();
        let link = directory.join("escape.md");
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(&external, &link);
        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_file(&external, &link);
        if linked.is_err() {
            let _ = fs::remove_file(&external);
            let _ = fs::remove_dir_all(root);
            return;
        }
        let catalog = SkillCatalog::discover(&root);
        assert!(catalog.skills().is_empty());
        assert!(
            catalog
                .errors()
                .iter()
                .any(|error| error.message.contains("escapes"))
        );
        let _ = fs::remove_file(&link);
        let _ = fs::remove_file(external);
        let _ = fs::remove_dir_all(root);
    }
}
