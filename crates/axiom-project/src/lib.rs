//! Headless project discovery and Composer/PSR-4 modeling.

use std::{
    collections::BTreeMap,
    fmt, fs, io,
    path::{Component, Path, PathBuf},
};

use serde::Deserialize;

#[derive(Debug)]
pub enum ProjectError {
    Io(io::Error),
    NotDirectory(PathBuf),
    OutsideProject(PathBuf),
    InvalidName(String),
    AlreadyExists(PathBuf),
}

impl fmt::Display for ProjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::NotDirectory(path) => {
                write!(f, "project root is not a directory: {}", path.display())
            }
            Self::OutsideProject(path) => {
                write!(f, "path is outside the project: {}", path.display())
            }
            Self::InvalidName(name) => write!(f, "invalid file name: {name}"),
            Self::AlreadyExists(path) => write!(f, "path already exists: {}", path.display()),
        }
    }
}

impl std::error::Error for ProjectError {}

impl From<io::Error> for ProjectError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Directory,
    File,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectEntry {
    pub path: PathBuf,
    pub name: String,
    pub kind: EntryKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileContent {
    Text(String),
    Binary,
    UnsupportedEncoding,
}

impl ProjectEntry {
    pub fn is_directory(&self) -> bool {
        self.kind == EntryKind::Directory
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum ComposerPaths {
    One(String),
    Many(Vec<String>),
}

impl ComposerPaths {
    fn iter(&self) -> Box<dyn Iterator<Item = &str> + '_> {
        match self {
            Self::One(path) => Box::new(std::iter::once(path.as_str())),
            Self::Many(paths) => Box::new(paths.iter().map(String::as_str)),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ComposerAutoload {
    #[serde(default, rename = "psr-4")]
    psr4: BTreeMap<String, ComposerPaths>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ComposerProject {
    #[serde(default, rename = "name")]
    pub package_name: Option<String>,
    #[serde(default, rename = "type")]
    pub package_type: Option<String>,
    #[serde(default)]
    pub require: BTreeMap<String, String>,
    #[serde(default, rename = "require-dev")]
    pub require_dev: BTreeMap<String, String>,
    #[serde(default)]
    autoload: ComposerAutoload,
    #[serde(default, rename = "autoload-dev")]
    autoload_dev: ComposerAutoload,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Psr4Mapping {
    pub namespace_prefix: String,
    pub directory: PathBuf,
    pub dev: bool,
}

#[derive(Debug)]
pub struct Project {
    root_path: PathBuf,
    composer: Option<ComposerProject>,
    composer_error: Option<String>,
    psr4: Vec<Psr4Mapping>,
}

impl Project {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ProjectError> {
        let root_path = fs::canonicalize(path)?;
        if !root_path.is_dir() {
            return Err(ProjectError::NotDirectory(root_path));
        }
        let composer_path = root_path.join("composer.json");
        let (composer, composer_error) = if composer_path.is_file() {
            match fs::read_to_string(&composer_path)
                .map_err(|error| error.to_string())
                .and_then(|text| serde_json::from_str(&text).map_err(|error| error.to_string()))
            {
                Ok(composer) => (Some(composer), None),
                Err(error) => (None, Some(error)),
            }
        } else {
            (None, None)
        };
        let psr4 = composer
            .as_ref()
            .map(|composer| collect_mappings(&root_path, composer))
            .unwrap_or_default();
        Ok(Self {
            root_path,
            composer,
            composer_error,
            psr4,
        })
    }

    pub fn root_path(&self) -> &Path {
        &self.root_path
    }

    pub fn name(&self) -> &str {
        self.root_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("project")
    }

    pub fn composer(&self) -> Option<&ComposerProject> {
        self.composer.as_ref()
    }

    pub fn composer_error(&self) -> Option<&str> {
        self.composer_error.as_deref()
    }

    pub fn psr4_mappings(&self) -> &[Psr4Mapping] {
        &self.psr4
    }

    /// Reads one directory level. No filesystem work is performed by UI render code.
    pub fn read_directory(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<Vec<ProjectEntry>, ProjectError> {
        let path = fs::canonicalize(path)?;
        if !path.starts_with(&self.root_path) {
            return Err(ProjectError::OutsideProject(path));
        }
        let mut entries = Vec::new();
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let file_type = entry.file_type()?;
            if file_type.is_dir() && matches!(name.as_str(), ".git" | "target" | "node_modules") {
                continue;
            }
            let kind = if file_type.is_dir() {
                EntryKind::Directory
            } else if file_type.is_file() {
                EntryKind::File
            } else {
                continue;
            };
            entries.push(ProjectEntry {
                path: entry.path(),
                name,
                kind,
            });
        }
        entries.sort_by(|left, right| {
            right
                .is_directory()
                .cmp(&left.is_directory())
                .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
        });
        Ok(entries)
    }

    pub fn create_file(&self, directory: &Path, name: &str) -> Result<PathBuf, ProjectError> {
        let directory = self.existing_path_inside(directory)?;
        validate_name(name)?;
        let path = directory.join(name);
        if path.exists() {
            return Err(ProjectError::AlreadyExists(path));
        }
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        Ok(path)
    }

    pub fn create_directory(&self, directory: &Path, name: &str) -> Result<PathBuf, ProjectError> {
        let directory = self.existing_path_inside(directory)?;
        validate_name(name)?;
        let path = directory.join(name);
        if path.exists() {
            return Err(ProjectError::AlreadyExists(path));
        }
        fs::create_dir(&path)?;
        Ok(path)
    }

    pub fn rename(&self, path: &Path, new_name: &str) -> Result<PathBuf, ProjectError> {
        let path = self.existing_path_inside(path)?;
        if path == self.root_path {
            return Err(ProjectError::OutsideProject(path));
        }
        validate_name(new_name)?;
        let destination = path
            .parent()
            .expect("non-root project entry")
            .join(new_name);
        if destination.exists() {
            return Err(ProjectError::AlreadyExists(destination));
        }
        fs::rename(&path, &destination)?;
        Ok(destination)
    }

    pub fn delete(&self, path: &Path) -> Result<(), ProjectError> {
        let path = self.existing_path_inside(path)?;
        if path == self.root_path {
            return Err(ProjectError::OutsideProject(path));
        }
        if path.is_dir() {
            fs::remove_dir_all(path)?;
        } else {
            fs::remove_file(path)?;
        }
        Ok(())
    }

    fn existing_path_inside(&self, path: &Path) -> Result<PathBuf, ProjectError> {
        let path = fs::canonicalize(path)?;
        if !path.starts_with(&self.root_path) {
            return Err(ProjectError::OutsideProject(path));
        }
        Ok(path)
    }

    pub fn namespace_to_paths(&self, namespace_or_class: &str) -> Vec<PathBuf> {
        self.psr4
            .iter()
            .filter_map(|mapping| {
                let suffix = namespace_or_class.strip_prefix(&mapping.namespace_prefix)?;
                let mut path = mapping.directory.clone();
                for segment in suffix
                    .trim_matches('\\')
                    .split('\\')
                    .filter(|part| !part.is_empty())
                {
                    path.push(segment);
                }
                path.set_extension("php");
                Some(path)
            })
            .collect()
    }

    pub fn namespace_to_directories(&self, namespace: &str) -> Vec<PathBuf> {
        self.psr4
            .iter()
            .filter_map(|mapping| {
                let suffix = namespace.strip_prefix(&mapping.namespace_prefix)?;
                let mut path = mapping.directory.clone();
                for segment in suffix
                    .trim_matches('\\')
                    .split('\\')
                    .filter(|part| !part.is_empty())
                {
                    path.push(segment);
                }
                Some(path)
            })
            .collect()
    }

    pub fn path_to_namespace(&self, path: impl AsRef<Path>) -> Option<String> {
        let path = normalize_existing_or_lexical(path.as_ref());
        self.psr4.iter().find_map(|mapping| {
            let relative = path.strip_prefix(&mapping.directory).ok()?;
            let parent = relative.parent()?;
            let suffix = parent
                .components()
                .filter_map(|component| match component {
                    Component::Normal(value) => value.to_str(),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\\");
            Some(if suffix.is_empty() {
                mapping.namespace_prefix.trim_end_matches('\\').to_owned()
            } else {
                format!("{}{}", mapping.namespace_prefix, suffix)
            })
        })
    }
}

pub fn is_php_file(path: impl AsRef<Path>) -> bool {
    path.as_ref()
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| matches!(extension.to_ascii_lowercase().as_str(), "php" | "phtml"))
}

pub fn is_supported_text_file(path: impl AsRef<Path>) -> bool {
    matches!(read_file_content(path), Ok(FileContent::Text(_)))
}

pub fn read_file_content(path: impl AsRef<Path>) -> io::Result<FileContent> {
    let bytes = fs::read(path)?;
    if bytes.contains(&0) {
        return Ok(FileContent::Binary);
    }
    Ok(match String::from_utf8(bytes) {
        Ok(text) => FileContent::Text(text),
        Err(_) => FileContent::UnsupportedEncoding,
    })
}

fn validate_name(name: &str) -> Result<(), ProjectError> {
    let path = Path::new(name);
    let mut components = path.components();
    let single_normal =
        matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none();
    let invalid_windows_character = name.chars().any(|character| {
        character.is_control()
            || matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            )
    });
    let windows_stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let windows_reserved = matches!(windows_stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (windows_stem.len() == 4
            && matches!(&windows_stem[..3], "COM" | "LPT")
            && matches!(windows_stem.as_bytes()[3], b'1'..=b'9'));
    if name.is_empty()
        || name == "."
        || name == ".."
        || !single_normal
        || invalid_windows_character
        || name.ends_with(['.', ' '])
        || windows_reserved
    {
        return Err(ProjectError::InvalidName(name.to_owned()));
    }
    Ok(())
}

fn collect_mappings(root: &Path, composer: &ComposerProject) -> Vec<Psr4Mapping> {
    let mut mappings = Vec::new();
    for (autoload, dev) in [(&composer.autoload, false), (&composer.autoload_dev, true)] {
        for (prefix, paths) in &autoload.psr4 {
            for path in paths.iter() {
                mappings.push(Psr4Mapping {
                    namespace_prefix: prefix.clone(),
                    directory: normalize_existing_or_lexical(&root.join(path)),
                    dev,
                });
            }
        }
    }
    mappings
}

fn normalize_existing_or_lexical(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| {
        let mut normalized = PathBuf::new();
        for component in path.components() {
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    normalized.pop();
                }
                other => normalized.push(other.as_os_str()),
            }
        }
        normalized
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    const COMPOSER: &str = r#"{
        "name": "axiom/demo",
        "type": "project",
        "require": {"php": "^8.3"},
        "autoload": {"psr-4": {"App\\": "src/", "Domain\\": ["src/Domain/", "packages/domain/"]}},
        "autoload-dev": {"psr-4": {"Tests\\": "tests/"}}
    }"#;

    fn fixture() -> (tempfile::TempDir, Project) {
        let root = tempdir().unwrap();
        for directory in [
            "src/Service",
            "src/Repository",
            "src/Domain",
            "packages/domain",
            "tests",
            ".git",
            "vendor",
        ] {
            fs::create_dir_all(root.path().join(directory)).unwrap();
        }
        fs::write(root.path().join("composer.json"), COMPOSER).unwrap();
        fs::write(root.path().join("src/Service/UserService.php"), "<?php").unwrap();
        fs::write(
            root.path().join("src/Repository/UserRepository.php"),
            "<?php",
        )
        .unwrap();
        fs::write(root.path().join("tests/UserServiceTest.php"), "<?php").unwrap();
        let project = Project::open(root.path().join("./src/..")).unwrap();
        (root, project)
    }

    #[test]
    fn opens_normalized_project_and_detects_composer() {
        let (root, project) = fixture();
        assert_eq!(project.root_path(), fs::canonicalize(root.path()).unwrap());
        let composer = project.composer().unwrap();
        assert_eq!(composer.package_name.as_deref(), Some("axiom/demo"));
        assert_eq!(composer.package_type.as_deref(), Some("project"));
        assert!(composer.require.contains_key("php"));
        assert!(project.composer_error().is_none());
    }

    #[test]
    fn invalid_and_missing_composer_are_controlled() {
        let invalid = tempdir().unwrap();
        fs::write(invalid.path().join("composer.json"), "{").unwrap();
        let project = Project::open(invalid.path()).unwrap();
        assert!(project.composer().is_none());
        assert!(project.composer_error().is_some());
        let missing = tempdir().unwrap();
        let project = Project::open(missing.path()).unwrap();
        assert!(project.composer().is_none());
        assert!(project.composer_error().is_none());
    }

    #[test]
    fn resolves_psr4_in_both_directions() {
        let (root, project) = fixture();
        assert_eq!(
            project.namespace_to_paths("App\\Service\\UserService"),
            vec![root.path().join("src/Service/UserService.php")]
        );
        assert_eq!(
            project
                .path_to_namespace(root.path().join("src/Repository/UserRepository.php"))
                .as_deref(),
            Some("App\\Repository")
        );
        assert_eq!(
            project
                .path_to_namespace(root.path().join("tests/UserServiceTest.php"))
                .as_deref(),
            Some("Tests")
        );
        assert_eq!(
            project.namespace_to_directories("App\\Service"),
            vec![root.path().join("src/Service")]
        );
    }

    #[test]
    fn supports_multiple_paths_and_autoload_dev() {
        let (root, project) = fixture();
        let paths = project.namespace_to_paths("Domain\\Model\\Order");
        assert_eq!(
            paths,
            vec![
                root.path().join("src/Domain/Model/Order.php"),
                root.path().join("packages/domain/Model/Order.php")
            ]
        );
        assert!(
            project
                .psr4_mappings()
                .iter()
                .any(|mapping| mapping.dev && mapping.namespace_prefix == "Tests\\")
        );
    }

    #[test]
    fn discovers_one_level_and_applies_exclusions() {
        let (_root, project) = fixture();
        let entries = project.read_directory(project.root_path()).unwrap();
        assert!(
            entries
                .iter()
                .any(|entry| entry.name == "src" && entry.is_directory())
        );
        assert!(
            entries
                .iter()
                .any(|entry| entry.name == "vendor" && entry.is_directory())
        );
        assert!(!entries.iter().any(|entry| entry.name == ".git"));
        assert!(entries.iter().any(|entry| entry.name == "composer.json"));
    }

    #[test]
    fn detects_supported_file_types() {
        let root = tempdir().unwrap();
        for name in [
            "index.php",
            "composer.json",
            "config.yaml",
            "README.md",
            ".env",
            "Dockerfile",
            "LICENSE",
            "file.unknown",
        ] {
            let path = root.path().join(name);
            fs::write(&path, "plain text").unwrap();
            assert!(is_supported_text_file(path), "{name}");
        }
        assert!(is_php_file("view.PHTML"));
        let binary = root.path().join("image.png");
        fs::write(&binary, [0, 1, 2, 3]).unwrap();
        assert_eq!(read_file_content(&binary).unwrap(), FileContent::Binary);
        let invalid = root.path().join("invalid.txt");
        fs::write(&invalid, [0xff, 0xfe]).unwrap();
        assert_eq!(
            read_file_content(&invalid).unwrap(),
            FileContent::UnsupportedEncoding
        );
    }

    #[test]
    fn creates_files_directories_and_dotfiles_without_overwrite() {
        let (root, project) = fixture();
        let directory = project
            .create_directory(project.root_path(), "generated")
            .unwrap();
        assert!(directory.is_dir());
        let file = project.create_file(&directory, "Service.php").unwrap();
        assert!(file.is_file());
        assert!(matches!(
            project.create_file(&directory, "Service.php"),
            Err(ProjectError::AlreadyExists(_))
        ));
        assert!(
            project
                .create_file(root.path(), ".env.local")
                .unwrap()
                .is_file()
        );
        assert!(
            project
                .create_file(root.path(), "Dockerfile")
                .unwrap()
                .is_file()
        );
    }

    #[test]
    fn renames_and_deletes_files_and_directories() {
        let (_root, project) = fixture();
        let file = project
            .create_file(project.root_path(), "before.txt")
            .unwrap();
        let file = project.rename(&file, "after.txt").unwrap();
        assert!(file.is_file());
        project.delete(&file).unwrap();
        assert!(!file.exists());
        let directory = project
            .create_directory(project.root_path(), "old")
            .unwrap();
        project.create_file(&directory, "nested.txt").unwrap();
        let directory = project.rename(&directory, "new").unwrap();
        assert!(directory.join("nested.txt").is_file());
        project.delete(&directory).unwrap();
        assert!(!directory.exists());
    }

    #[test]
    fn rejects_traversal_invalid_names_and_outside_paths() {
        let (_root, project) = fixture();
        for name in [
            "",
            ".",
            "..",
            "../escape",
            "nested/file",
            "bad?.txt",
            "CON",
            "LPT1.log",
            "trailing.",
            "trailing ",
        ] {
            assert!(matches!(
                project.create_file(project.root_path(), name),
                Err(ProjectError::InvalidName(_))
            ));
        }
        let outside = tempdir().unwrap();
        assert!(matches!(
            project.delete(outside.path()),
            Err(ProjectError::OutsideProject(_))
        ));
    }

    #[test]
    fn opens_repository_fixture() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/php-project");
        let project = Project::open(&fixture).unwrap();
        assert_eq!(
            project
                .composer()
                .and_then(|composer| composer.package_name.as_deref()),
            Some("axiom/demo")
        );
        assert_eq!(
            project
                .path_to_namespace(fixture.join("src/Service/UserService.php"))
                .as_deref(),
            Some("App\\Service")
        );
    }
}
