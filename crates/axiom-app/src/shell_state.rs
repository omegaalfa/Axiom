use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

pub const MAX_RECENT_PROJECTS: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupTarget {
    Project {
        root: PathBuf,
        initial_file: Option<PathBuf>,
    },
    Welcome,
}

pub fn resolve_startup_target(
    args: &[OsString],
    project_override: Option<&Path>,
    cwd: &Path,
) -> StartupTarget {
    if let Some(path) = args.get(1).map(PathBuf::from) {
        return target_for_explicit_path(path);
    }
    if let Some(path) = project_override {
        return target_for_explicit_path(path.to_path_buf());
    }
    if is_project_directory(cwd) {
        target_for_explicit_path(cwd.to_path_buf())
    } else {
        StartupTarget::Welcome
    }
}

fn target_for_explicit_path(path: PathBuf) -> StartupTarget {
    let path = normalize_path(fs::canonicalize(&path).unwrap_or(path));
    if path.is_file() {
        let Some(parent) = path.parent().map(Path::to_path_buf) else {
            return StartupTarget::Welcome;
        };
        return StartupTarget::Project {
            root: parent,
            initial_file: Some(path),
        };
    }
    if path.is_dir() {
        StartupTarget::Project {
            root: path,
            initial_file: None,
        }
    } else {
        StartupTarget::Welcome
    }
}

pub fn is_project_directory(path: &Path) -> bool {
    path.is_dir()
        && ["composer.json", ".git", "src"]
            .iter()
            .any(|marker| path.join(marker).exists())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecentProject {
    pub path: PathBuf,
    pub last_opened: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecentProjects {
    #[serde(default)]
    pub projects: Vec<RecentProject>,
}

fn normalize_path(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(path) = path.to_str().and_then(|path| path.strip_prefix("\\\\?\\")) {
            return PathBuf::from(path);
        }
    }
    path
}

impl RecentProjects {
    pub fn load(path: &Path) -> Self {
        fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_vec_pretty(self)?)
    }

    pub fn add(&mut self, path: &Path, last_opened: u64) {
        let normalized =
            normalize_path(fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()));
        self.projects.retain(|entry| entry.path != normalized);
        self.projects.push(RecentProject {
            path: normalized,
            last_opened,
        });
        self.projects
            .sort_by(|left, right| right.last_opened.cmp(&left.last_opened));
        self.projects.truncate(MAX_RECENT_PROJECTS);
    }

    pub fn existing(&self) -> impl Iterator<Item = &RecentProject> {
        self.projects.iter().filter(|entry| entry.path.is_dir())
    }
}

pub fn recent_projects_path() -> Option<PathBuf> {
    let current = ProjectDirs::from("dev", "Axiom", "Axiom")
        .map(|directories| directories.config_dir().join("recent-projects.json"))?;
    if !current.exists()
        && let Some(legacy) = ProjectDirs::from("dev", "RustStorm", "RustStorm")
            .map(|directories| directories.config_dir().join("recent-projects.json"))
        && legacy.is_file()
    {
        if let Some(parent) = current.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::copy(legacy, &current);
    }
    Some(current)
}

pub fn unix_timestamp_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn project(marker: &str) -> tempfile::TempDir {
        let directory = tempdir().unwrap();
        if marker.ends_with('/') {
            fs::create_dir(directory.path().join(marker)).unwrap();
        } else {
            fs::write(directory.path().join(marker), "{}").unwrap();
        }
        directory
    }

    #[test]
    fn startup_precedence_and_welcome_fallback() {
        let cli = project("composer.json");
        let override_project = project(".git/");
        let cwd = project("src/");
        let args = vec![OsString::from("axiom"), cli.path().into()];
        assert!(matches!(
            resolve_startup_target(&args, Some(override_project.path()), cwd.path()),
            StartupTarget::Project { root, .. } if root == cli.path()
        ));
        assert!(matches!(
            resolve_startup_target(&["axiom".into()], Some(override_project.path()), cwd.path()),
            StartupTarget::Project { root, .. } if root == override_project.path()
        ));
        assert!(matches!(
            resolve_startup_target(&["axiom".into()], None, cwd.path()),
            StartupTarget::Project { root, .. } if root == cwd.path()
        ));
        let empty = tempdir().unwrap();
        assert_eq!(
            resolve_startup_target(&["axiom".into()], None, empty.path()),
            StartupTarget::Welcome
        );
    }

    #[test]
    fn explicit_file_uses_parent_and_opens_file() {
        let directory = tempdir().unwrap();
        let file = directory.path().join("index.php");
        fs::write(&file, "<?php").unwrap();
        let target = resolve_startup_target(
            &["axiom".into(), file.clone().into_os_string()],
            None,
            directory.path(),
        );
        assert_eq!(
            target,
            StartupTarget::Project {
                root: directory.path().to_path_buf(),
                initial_file: Some(file)
            }
        );
    }

    #[test]
    fn recent_projects_deduplicate_order_limit_and_ignore_missing() {
        let root = tempdir().unwrap();
        let mut recent = RecentProjects::default();
        for index in 0..12 {
            let path = root.path().join(index.to_string());
            fs::create_dir(&path).unwrap();
            recent.add(&path, index);
        }
        recent.add(&root.path().join("5"), 99);
        assert_eq!(recent.projects.len(), MAX_RECENT_PROJECTS);
        assert_eq!(recent.projects[0].path, root.path().join("5"));
        recent.projects.push(RecentProject {
            path: root.path().join("missing"),
            last_opened: 100,
        });
        assert_eq!(recent.existing().count(), MAX_RECENT_PROJECTS);
    }

    #[test]
    fn recent_projects_round_trip_and_invalid_config_is_safe() {
        let root = tempdir().unwrap();
        let config = root.path().join("recent.json");
        let mut recent = RecentProjects::default();
        recent.add(root.path(), 42);
        recent.save(&config).unwrap();
        assert_eq!(RecentProjects::load(&config), recent);
        fs::write(&config, "not json").unwrap();
        assert_eq!(RecentProjects::load(&config), RecentProjects::default());
    }
}
