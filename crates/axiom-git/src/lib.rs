//! Headless Git repository intelligence for Axiom.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};
use gix::bstr::ByteSlice;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitError {
    NotRepository,
    RepositoryError(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitRepositoryState {
    pub repository_root: PathBuf,
    pub worktree_root: PathBuf,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub detached: bool,
    pub files: Vec<GitFileStatus>,
    pub remotes: Vec<GitRemote>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitFileStatus {
    pub path: PathBuf,
    pub old_path: Option<PathBuf>,
    pub index: GitChangeState,
    pub worktree: GitChangeState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitFileDiff {
    pub path: PathBuf,
    pub old_path: Option<PathBuf>,
    pub target: GitDiffTarget,
    pub index: GitChangeState,
    pub worktree: GitChangeState,
    pub hunks: Vec<GitDiffHunk>,
    pub binary: bool,
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitDiffTarget {
    /// The working-tree Changes view: index blob to current worktree bytes.
    IndexToWorktree,
    /// The staged Changes view: HEAD blob to index blob.
    HeadToIndex,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitDiffHunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub lines: Vec<GitDiffLine>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitDiffLineKind {
    Context,
    Addition,
    Deletion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitDiffLine {
    pub kind: GitDiffLineKind,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
    pub text: String,
}

pub const MAX_DIFF_BYTES: usize = 512 * 1024;
pub const MAX_DIFF_LINES: usize = 10_000;
pub const MAX_DIFF_HUNKS: usize = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitChangeState {
    Unmodified,
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Conflicted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitRemote {
    pub name: String,
    pub url: String,
    pub provider: GitRemoteProvider,
    pub owner: Option<String>,
    pub repository: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitRemoteProvider {
    GitHub,
    Other,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct GitRepositoryService;

impl GitRepositoryService {
    pub fn load(&self, workspace: impl AsRef<Path>) -> Result<GitRepositoryState, GitError> {
        let repo = gix::discover(workspace.as_ref()).map_err(|error| {
            if error.is_not_found() && !has_git_marker(workspace.as_ref()) {
                GitError::NotRepository
            } else {
                GitError::RepositoryError(error.to_string())
            }
        })?;
        let worktree_root = repo
            .workdir()
            .ok_or_else(|| GitError::RepositoryError("repository has no worktree".into()))?
            .to_path_buf();
        let head = repo
            .head()
            .map_err(|error| GitError::RepositoryError(error.to_string()))?;
        let detached = head.is_detached();
        let branch = repo
            .head_name()
            .map_err(|error| GitError::RepositoryError(error.to_string()))?
            .map(|name| name.shorten().to_string());
        let head = head.id().map(|id| id.to_string());
        let files = status(&repo)?;
        let remotes = remotes(&repo);
        Ok(GitRepositoryState {
            repository_root: repo
                .common_dir()
                .parent()
                .unwrap_or(&worktree_root)
                .to_path_buf(),
            worktree_root,
            head,
            branch,
            detached,
            files,
            remotes,
        })
    }

    pub fn diff_file(
        &self,
        workspace: impl AsRef<Path>,
        path: impl AsRef<Path>,
    ) -> Result<GitFileDiff, GitError> {
        self.diff_file_with_target(workspace, path, GitDiffTarget::IndexToWorktree)
    }

    pub fn diff_file_with_target(
        &self,
        workspace: impl AsRef<Path>,
        path: impl AsRef<Path>,
        target: GitDiffTarget,
    ) -> Result<GitFileDiff, GitError> {
        let repo = gix::discover(workspace.as_ref()).map_err(|error| {
            if error.is_not_found() && !has_git_marker(workspace.as_ref()) {
                GitError::NotRepository
            } else {
                GitError::RepositoryError(error.to_string())
            }
        })?;
        let worktree = repo
            .workdir()
            .ok_or_else(|| GitError::RepositoryError("repository has no worktree".into()))?;
        let relative = path.as_ref();
        let state = status(&repo)?;
        let status = state
            .into_iter()
            .find(|file| file.path == relative)
            .ok_or_else(|| GitError::RepositoryError("file is no longer changed".into()))?;
        let index = index_bytes(&repo, relative)?;
        let head = head_bytes(&repo, relative)?;
        let disk_path = worktree.join(relative);
        let disk = std::fs::read(&disk_path).ok();
        let head_present = head.is_some();
        let index_present = index.is_some();
        let worktree_present = disk.is_some();

        let (old, new) = match target {
            GitDiffTarget::IndexToWorktree => {
                let old = index.unwrap_or_default();
                let new = disk.unwrap_or_default();
                (old, new)
            }
            GitDiffTarget::HeadToIndex => {
                let old = head.unwrap_or_default();
                let new = index.unwrap_or_default();
                (old, new)
            }
        };
        let binary = is_binary(&old) || is_binary(&new);
        let (hunks, truncated) = if binary {
            (Vec::new(), false)
        } else {
            diff_text(&old, &new)
        };
        let lines = hunks
            .iter()
            .map(|hunk| hunk.lines.len())
            .sum::<usize>();
        tracing::debug!(
            target: "axiom.git_diag",
            path = %relative.display(),
            target = match target {
                GitDiffTarget::IndexToWorktree => "IndexToWorktree",
                GitDiffTarget::HeadToIndex => "HeadToIndex",
            },
            index_status = ?status.index,
            worktree_status = ?status.worktree,
            head_present,
            index_present,
            worktree_present,
            old_bytes = old.len(),
            new_bytes = new.len(),
            hunks = hunks.len(),
            lines,
            "[GIT-DIFF]"
        );
        Ok(GitFileDiff {
            path: status.path,
            old_path: status.old_path,
            target,
            index: status.index,
            worktree: status.worktree,
            hunks,
            binary,
            truncated,
        })
    }

    /// Stage exactly one repository-relative path by changing only the index.
    pub fn stage_file(
        &self,
        workspace: impl AsRef<Path>,
        path: impl AsRef<Path>,
    ) -> Result<(), GitError> {
        let repo = discover_repository(workspace.as_ref())?;
        let path = repository_relative_path(path.as_ref())?;
        let mutation_lock = index_mutation_lock(&repo);
        let _guard = mutation_lock.lock().map_err(|_| {
            GitError::RepositoryError("index mutation lock poisoned".into())
        })?;
        let worktree = repo
            .workdir()
            .ok_or_else(|| GitError::RepositoryError("repository has no worktree".into()))?;
        let mut index = mutable_index(&repo)?;
        let disk_path = worktree.join(&path);
        remove_index_path(&mut index, &path);
        if disk_path.is_file() {
            let bytes = std::fs::read(&disk_path)
                .map_err(|error| GitError::RepositoryError(error.to_string()))?;
            let blob = repo
                .write_blob(&bytes)
                .map_err(|error| GitError::RepositoryError(error.to_string()))?
                .detach();
            index.dangerously_push_entry(
                Default::default(),
                blob,
                gix::index::entry::Flags::empty(),
                gix::index::entry::Mode::FILE,
                path.to_string_lossy().replace('\\', "/").as_bytes().as_bstr(),
            );
        }
        index.sort_entries();
        index
            .write(Default::default())
            .map_err(|error| GitError::RepositoryError(error.to_string()))
    }

    /// Unstage exactly one repository-relative path without changing the worktree.
    pub fn unstage_file(
        &self,
        workspace: impl AsRef<Path>,
        path: impl AsRef<Path>,
    ) -> Result<(), GitError> {
        let repo = discover_repository(workspace.as_ref())?;
        let path = repository_relative_path(path.as_ref())?;
        tracing::debug!(
            target: "axiom.git_ui_diag",
            event = "unstage_backend_entered",
            path = %path.display(),
        );
        let mutation_lock = index_mutation_lock(&repo);
        let _guard = mutation_lock.lock().map_err(|_| {
            GitError::RepositoryError("index mutation lock poisoned".into())
        })?;
        tracing::debug!(
            target: "axiom.git_ui_diag",
            event = "unstage_index_lock_acquired",
            path = %path.display(),
        );
        let mut index = mutable_index(&repo)?;
        tracing::debug!(
            target: "axiom.git_ui_diag",
            event = "unstage_index_opened",
            path = %path.display(),
        );
        remove_index_path(&mut index, &path);
        let head_entry = head_entry(&repo, &path)?;
        tracing::debug!(
            target: "axiom.git_ui_diag",
            event = "unstage_head_entry_found",
            path = %path.display(),
            found = head_entry.is_some(),
        );
        if let Some((blob, mode)) = head_entry {
            index.dangerously_push_entry(
                Default::default(),
                blob,
                gix::index::entry::Flags::empty(),
                mode,
                path.to_string_lossy().replace('\\', "/").as_bytes().as_bstr(),
            );
        }
        index.sort_entries();
        let result = index
            .write(Default::default())
            .map_err(|error| GitError::RepositoryError(error.to_string()));
        tracing::debug!(
            target: "axiom.git_ui_diag",
            event = "unstage_index_write_result",
            path = %path.display(),
            result = if result.is_ok() { "ok" } else { "err" },
        );
        result
    }

}

fn discover_repository(workspace: &Path) -> Result<gix::Repository, GitError> {
    gix::discover(workspace).map_err(|error| {
        if error.is_not_found() && !has_git_marker(workspace) {
            GitError::NotRepository
        } else {
            GitError::RepositoryError(error.to_string())
        }
    })
}

fn repository_relative_path(path: &Path) -> Result<PathBuf, GitError> {
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(component, std::path::Component::ParentDir | std::path::Component::RootDir)
        })
        || path.as_os_str().is_empty()
    {
        return Err(GitError::RepositoryError("path must be repository-relative".into()));
    }
    Ok(path.to_path_buf())
}

fn mutable_index(repo: &gix::Repository) -> Result<gix::index::File, GitError> {
    if repo.index_path().exists() {
        return repo
            .open_index()
            .map_err(|error| GitError::RepositoryError(error.to_string()));
    }
    match repo
        .index_or_load_from_head_or_empty()
        .map_err(|error| GitError::RepositoryError(error.to_string()))?
    {
        gix::worktree::IndexPersistedOrInMemory::Persisted(_) => repo
            .open_index()
            .map_err(|error| GitError::RepositoryError(error.to_string())),
        gix::worktree::IndexPersistedOrInMemory::InMemory(index) => Ok(index),
    }
}

fn remove_index_path(index: &mut gix::index::File, path: &Path) {
    let path = path.to_string_lossy().replace('\\', "/");
    index.remove_entries(|_, entry_path, _| entry_path.to_str() == Ok(path.as_str()));
}

static INDEX_MUTATION_LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>>= OnceLock::new();

fn index_mutation_lock(repo: &gix::Repository) -> Arc<Mutex<()>> {
    let key = repo.common_dir().to_path_buf();
    let locks = INDEX_MUTATION_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut locks = locks.lock().expect("index mutation lock map poisoned");
    locks
        .entry(key)
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

fn head_bytes(repo: &gix::Repository, path: &Path) -> Result<Option<Vec<u8>>, GitError> {
    let Some((blob, _mode)) = head_entry(repo, path)? else {
        return Ok(None);
    };
    let blob = repo
        .find_blob(blob)
        .map_err(|error| GitError::RepositoryError(error.to_string()))?;
    Ok(Some(blob.data.to_vec()))
}

fn head_entry(
    repo: &gix::Repository,
    path: &Path,
) -> Result<Option<(gix::hash::ObjectId, gix::index::entry::Mode)>, GitError> {
    let tree_id = repo
        .head_tree_id_or_empty()
        .map_err(|error| GitError::RepositoryError(error.to_string()))?;
    let Some(entry) = repo
        .find_tree(tree_id)
        .map_err(|error| GitError::RepositoryError(error.to_string()))?
        .lookup_entry(path_components(path))
        .map_err(|error| GitError::RepositoryError(error.to_string()))?
    else {
        return Ok(None);
    };
    Ok(Some((
        entry.object_id(),
        gix::index::entry::Mode::from(entry.mode()),
    )))
}

fn index_bytes(repo: &gix::Repository, path: &Path) -> Result<Option<Vec<u8>>, GitError> {
    let index = repo
        .index_or_load_from_head_or_empty()
        .map_err(|error| GitError::RepositoryError(error.to_string()))?;
    let path = path.to_string_lossy().replace('\\', "/");
    let id = match &index {
        gix::worktree::IndexPersistedOrInMemory::Persisted(index) => index
            .entries_with_paths_by_filter_map(|entry_path, entry| {
                (entry_path.to_str() == Ok(path.as_str())).then_some(entry.id)
            })
            .next()
            .map(|(_, id)| id),
        gix::worktree::IndexPersistedOrInMemory::InMemory(index) => index
            .entries_with_paths_by_filter_map(|entry_path, entry| {
                (entry_path.to_str() == Ok(path.as_str())).then_some(entry.id)
            })
            .next()
            .map(|(_, id)| id),
    };
    id.map(|id| {
        repo.find_blob(id)
            .map(|blob| blob.data.to_vec())
            .map_err(|error| GitError::RepositoryError(error.to_string()))
    })
    .transpose()
}

fn path_components(path: &Path) -> Vec<String> {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect()
}

fn is_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8 * 1024).any(|byte| *byte == 0) || std::str::from_utf8(bytes).is_err()
}

struct HunkCollector {
    hunks: Vec<GitDiffHunk>,
    lines: usize,
    truncated: bool,
}

impl gix::diff::blob::unified_diff::ConsumeHunk for HunkCollector {
    type Out = Self;

    fn consume_hunk(
        &mut self,
        header: gix::diff::blob::unified_diff::HunkHeader,
        lines: &[(gix::diff::blob::unified_diff::DiffLineKind, &[u8])],
    ) -> std::io::Result<()> {
        if self.hunks.len() >= MAX_DIFF_HUNKS || self.lines >= MAX_DIFF_LINES {
            self.truncated = true;
            return Ok(());
        }
        let mut old_line = header.before_hunk_start;
        let mut new_line = header.after_hunk_start;
        let mut output = Vec::new();
        for (kind, bytes) in lines {
            if self.lines + output.len() >= MAX_DIFF_LINES {
                self.truncated = true;
                break;
            }
            let (kind, old, new) = match kind {
                gix::diff::blob::unified_diff::DiffLineKind::Context => {
                    let old = old_line;
                    let new = new_line;
                    old_line += 1;
                    new_line += 1;
                    (GitDiffLineKind::Context, Some(old), Some(new))
                }
                gix::diff::blob::unified_diff::DiffLineKind::Add => {
                    let new = new_line;
                    new_line += 1;
                    (GitDiffLineKind::Addition, None, Some(new))
                }
                gix::diff::blob::unified_diff::DiffLineKind::Remove => {
                    let old = old_line;
                    old_line += 1;
                    (GitDiffLineKind::Deletion, Some(old), None)
                }
            };
            output.push(GitDiffLine {
                kind,
                old_line: old,
                new_line: new,
                text: String::from_utf8_lossy(bytes)
                    .trim_end_matches(['\r', '\n'])
                    .to_owned(),
            });
        }
        self.lines += output.len();
        self.hunks.push(GitDiffHunk {
            old_start: header.before_hunk_start,
            old_lines: header.before_hunk_len,
            new_start: header.after_hunk_start,
            new_lines: header.after_hunk_len,
            lines: output,
        });
        Ok(())
    }

    fn finish(self) -> Self {
        self
    }
}

fn diff_text(old: &[u8], new: &[u8]) -> (Vec<GitDiffHunk>, bool) {
    if old.len().saturating_add(new.len()) > MAX_DIFF_BYTES {
        return (Vec::new(), true);
    }
    let input = gix::diff::blob::InternedInput::new(old, new);
    let diff = gix::diff::blob::diff_with_slider_heuristics(
        gix::diff::blob::Algorithm::Histogram,
        &input,
    );
    let result = gix::diff::blob::UnifiedDiff::new(
        &diff,
        &input,
        HunkCollector {
            hunks: Vec::new(),
            lines: 0,
            truncated: false,
        },
        gix::diff::blob::unified_diff::ContextSize::symmetrical(3),
    )
    .consume()
    .unwrap_or(HunkCollector {
        hunks: Vec::new(),
        lines: 0,
        truncated: true,
    });
    (result.hunks, result.truncated)
}

fn status(repo: &gix::Repository) -> Result<Vec<GitFileStatus>, GitError> {
    let iter = repo
        .status(gix::progress::Discard)
        .map_err(|error| GitError::RepositoryError(error.to_string()))?
        .untracked_files(gix::status::UntrackedFiles::Files)
        .into_iter(Vec::new())
        .map_err(|error| GitError::RepositoryError(error.to_string()))?;
    let mut files = std::collections::BTreeMap::<PathBuf, GitFileStatus>::new();
    for item in iter {
        let item = item.map_err(|error| GitError::RepositoryError(error.to_string()))?;
        match item {
            gix::status::Item::IndexWorktree(item) => {
                let path = PathBuf::from(item.rela_path().to_string());
                let state = match item.summary() {
                    Some(gix::status::index_worktree::iter::Summary::Added) => {
                        GitChangeState::Untracked
                    }
                    Some(gix::status::index_worktree::iter::Summary::Removed) => {
                        GitChangeState::Deleted
                    }
                    Some(gix::status::index_worktree::iter::Summary::Renamed) => {
                        GitChangeState::Renamed
                    }
                    Some(gix::status::index_worktree::iter::Summary::Conflict) => {
                        GitChangeState::Conflicted
                    }
                    Some(_) => GitChangeState::Modified,
                    None => continue,
                };
                files
                    .entry(path.clone())
                    .or_insert(GitFileStatus {
                        path,
                        old_path: None,
                        index: GitChangeState::Unmodified,
                        worktree: GitChangeState::Unmodified,
                    })
                    .worktree = state;
            }
            gix::status::Item::TreeIndex(change) => {
                let path = PathBuf::from(change.location().to_string());
                let old_path = match &change {
                    gix::diff::index::ChangeRef::Rewrite {
                        source_location, copy: false, ..
                    } => Some(PathBuf::from(source_location.to_string())),
                    _ => None,
                };
                let state = match change {
                    gix::diff::index::ChangeRef::Addition { .. } => GitChangeState::Added,
                    gix::diff::index::ChangeRef::Deletion { .. } => GitChangeState::Deleted,
                    gix::diff::index::ChangeRef::Modification { .. } => GitChangeState::Modified,
                    gix::diff::index::ChangeRef::Rewrite { copy, .. } => {
                        if copy { GitChangeState::Modified } else { GitChangeState::Renamed }
                    }
                };
                let file = files.entry(path.clone()).or_insert(GitFileStatus {
                    path,
                    old_path: old_path.clone(),
                    index: GitChangeState::Unmodified,
                    worktree: GitChangeState::Unmodified,
                });
                file.old_path = old_path;
                file.index = state;
            }
        }
    }
    Ok(files.into_values().collect())
}

fn remotes(repo: &gix::Repository) -> Vec<GitRemote> {
    repo.remote_names()
        .into_iter()
        .filter_map(|name| {
            let name = name.to_string();
            let remote = repo.find_remote(&name).ok()?;
            let raw_url = remote.url(gix::remote::Direction::Fetch)?.to_string();
            let url = sanitize_remote_url(&raw_url);
            let parsed = parse_remote_url(&url);
            Some(GitRemote {
                name,
                url,
                provider: parsed.0,
                owner: parsed.1,
                repository: parsed.2,
            })
        })
        .collect()
}

fn parse_remote_url(url: &str) -> (GitRemoteProvider, Option<String>, Option<String>) {
    let normalized = url.trim_end_matches('/').trim_end_matches(".git");
    let github_path = normalized
        .strip_prefix("https://github.com/")
        .or_else(|| normalized.strip_prefix("http://github.com/"))
        .or_else(|| normalized.strip_prefix("ssh://git@github.com/"))
        .or_else(|| normalized.strip_prefix("git@github.com:"));
    let Some(path) = github_path else {
        return (GitRemoteProvider::Other, None, None);
    };
    let mut parts = path.split('/');
    let owner = parts.next().filter(|part| !part.is_empty()).map(str::to_owned);
    let repository = parts.next().filter(|part| !part.is_empty()).map(str::to_owned);
    if owner.is_some() && repository.is_some() {
        (GitRemoteProvider::GitHub, owner, repository)
    } else {
        (GitRemoteProvider::Other, None, None)
    }
}

fn sanitize_remote_url(url: &str) -> String {
    if let Ok(mut parsed) = url::Url::parse(url) {
        let _ = parsed.set_username("");
        let _ = parsed.set_password(None);
        return parsed.to_string();
    }
    if let Some(at) = url.find('@')
        && url[..at].contains(':')
    {
        return url[at + 1..].to_owned();
    }
    url.to_owned()
}

fn has_git_marker(start: &Path) -> bool {
    let mut current = Some(start);
    while let Some(path) = current {
        if path.join(".git").exists() {
            return true;
        }
        current = path.parent();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use gix::bstr::ByteSlice;
    use std::{fs, io::Write};
    use tempfile::tempdir;

    fn initialized_repo() -> (tempfile::TempDir, gix::Repository) {
        let dir = tempdir().unwrap();
        let repo = gix::init(dir.path()).unwrap();
        (dir, repo)
    }

    fn tracked_repo() -> tempfile::TempDir {
        let (dir, repo) = initialized_repo();
        fs::write(dir.path().join("tracked.txt"), "one\n").unwrap();
        let blob = repo.write_blob(b"one\n").unwrap().detach();
        let mut state = gix::index::State::new(gix::hash::Kind::Sha1);
        state.dangerously_push_entry(
            Default::default(),
            blob,
            gix::index::entry::Flags::empty(),
            gix::index::entry::Mode::FILE,
            b"tracked.txt".as_bstr(),
        );
        state.sort_entries();
        let mut index = gix::index::File::from_state(state, repo.index_path());
        index.write(Default::default()).unwrap();
        dir
    }

    fn committed_tracked_repo() -> tempfile::TempDir {
        let (dir, repo) = initialized_repo();
        fs::write(dir.path().join("tracked.txt"), "one\n").unwrap();
        commit_entries(&repo, &[("tracked.txt", b"one\n")]);
        add_index_entry(&repo, b"tracked.txt", b"one\n");
        dir
    }

    fn add_index_entry(repo: &gix::Repository, path: &[u8], content: &[u8]) {
        let blob = repo.write_blob(content).unwrap().detach();
        if repo.index_path().exists() {
            let mut index = repo.open_index().unwrap();
            index.dangerously_push_entry(
                Default::default(),
                blob,
                gix::index::entry::Flags::empty(),
                gix::index::entry::Mode::FILE,
                path.as_bstr(),
            );
            index.sort_entries();
            index.write(Default::default()).unwrap();
        } else {
            let mut state = gix::index::State::new(gix::hash::Kind::Sha1);
            state.dangerously_push_entry(
                Default::default(),
                blob,
                gix::index::entry::Flags::empty(),
                gix::index::entry::Mode::FILE,
                path.as_bstr(),
            );
            state.sort_entries();
            let mut index = gix::index::File::from_state(state, repo.index_path());
            index.write(Default::default()).unwrap();
        }
    }

    fn write_index_entries(repo: &gix::Repository, entries: &[(&[u8], &[u8])]) {
        let mut state = gix::index::State::new(gix::hash::Kind::Sha1);
        for (path, content) in entries {
            let blob = repo.write_blob(content).unwrap().detach();
            state.dangerously_push_entry(
                Default::default(),
                blob,
                gix::index::entry::Flags::empty(),
                gix::index::entry::Mode::FILE,
                path.as_bstr(),
            );
        }
        state.sort_entries();
        let mut index = gix::index::File::from_state(state, repo.index_path());
        index.write(Default::default()).unwrap();
    }

    fn commit_entries(repo: &gix::Repository, entries: &[(&str, &[u8])]) {
        let mut editor = repo.empty_tree().edit().unwrap();
        for (path, content) in entries {
            let blob = repo.write_blob(content).unwrap().detach();
            editor
                .upsert(*path, gix::object::tree::EntryKind::Blob, blob)
                .unwrap();
        }
        let tree = editor.write().unwrap().detach();
        let commit = repo
            .new_commit_as(
                gix::actor::SignatureRef {
                    name: b"Axiom Test".as_bstr(),
                    email: b"axiom@example.test".as_bstr(),
                    time: "1 +0000",
                },
                gix::actor::SignatureRef {
                    name: b"Axiom Test".as_bstr(),
                    email: b"axiom@example.test".as_bstr(),
                    time: "1 +0000",
                },
                "fixture",
                tree,
                std::iter::empty::<gix::hash::ObjectId>(),
            )
            .unwrap()
            .id;
        fs::write(
            repo.workdir().unwrap().join(".git/HEAD"),
            format!("{commit}\n"),
        )
        .unwrap();
    }

    #[test]
    fn discovers_repository_from_root_and_nested_directory() {
        let dir = tracked_repo();
        let nested = dir.path().join("src/deep");
        fs::create_dir_all(&nested).unwrap();
        let service = GitRepositoryService;
        let root = service.load(dir.path()).unwrap();
        let nested_state = service.load(&nested).unwrap();
        assert_eq!(root.worktree_root, dir.path());
        assert_eq!(nested_state.repository_root, root.repository_root);
        assert_eq!(nested_state.worktree_root, root.worktree_root);
    }

    #[test]
    fn non_git_directory_is_not_repository() {
        let dir = tempdir().unwrap();
        assert_eq!(GitRepositoryService.load(dir.path()), Err(GitError::NotRepository));
    }

    #[test]
    fn reports_branch_and_detached_head() {
        let dir = tracked_repo();
        let service = GitRepositoryService;
        let state = service.load(dir.path()).unwrap();
        assert!(state.branch.is_some());
        assert!(!state.detached);
        let head = gix::open(dir.path())
            .unwrap()
            .new_commit_as(
                gix::actor::SignatureRef {
                    name: b"Axiom Test".as_bstr(),
                    email: b"axiom@example.test".as_bstr(),
                    time: "1 +0000",
                },
                gix::actor::SignatureRef {
                    name: b"Axiom Test".as_bstr(),
                    email: b"axiom@example.test".as_bstr(),
                    time: "1 +0000",
                },
                "fixture",
                gix::hash::ObjectId::empty_tree(gix::hash::Kind::Sha1),
                std::iter::empty::<gix::hash::ObjectId>(),
            )
            .unwrap()
            .id
            .to_string();
        fs::write(dir.path().join(".git/HEAD"), format!("{head}\n")).unwrap();
        let detached = service.load(dir.path()).unwrap();
        assert!(detached.detached);
        assert_eq!(detached.branch, None);
        assert_eq!(detached.head.as_deref(), Some(head.as_str()));
    }

    #[test]
    fn reports_structured_worktree_and_index_states() {
        let dir = tracked_repo();
        fs::write(dir.path().join("tracked.txt"), "two\n").unwrap();
        fs::write(dir.path().join("untracked.txt"), "new\n").unwrap();
        fs::write(dir.path().join("staged.txt"), "staged\n").unwrap();
        fs::remove_file(dir.path().join("tracked.txt")).unwrap();
        let repo = gix::open(dir.path()).unwrap();
        let blob = repo.write_blob(b"staged\n").unwrap().detach();
        let mut index = repo.open_index().unwrap();
        index.dangerously_push_entry(
            Default::default(),
            blob,
            gix::index::entry::Flags::empty(),
            gix::index::entry::Mode::FILE,
            b"staged.txt".as_bstr(),
        );
        index.sort_entries();
        index.write(Default::default()).unwrap();
        let files = GitRepositoryService.load(dir.path()).unwrap().files;
        let by_path = |name: &str| files.iter().find(|file| file.path == Path::new(name));
        assert_eq!(
            by_path("tracked.txt").map(|file| file.worktree),
            Some(GitChangeState::Deleted)
        );
        assert_eq!(
            by_path("untracked.txt").map(|file| file.worktree),
            Some(GitChangeState::Untracked)
        );
        assert_eq!(
            by_path("staged.txt").map(|file| file.index),
            Some(GitChangeState::Added)
        );
    }

    #[test]
    fn stage_changes_only_the_index_for_modified_and_untracked_files() {
        let dir = committed_tracked_repo();
        fs::write(dir.path().join("tracked.txt"), "two\n").unwrap();
        fs::write(dir.path().join("new.txt"), "new\n").unwrap();
        let service = GitRepositoryService;
        service.stage_file(dir.path(), "tracked.txt").unwrap();
        service.stage_file(dir.path(), "new.txt").unwrap();
        let files = service.load(dir.path()).unwrap().files;
        let tracked = files.iter().find(|file| file.path == Path::new("tracked.txt")).unwrap();
        let new = files.iter().find(|file| file.path == Path::new("new.txt")).unwrap();
        assert_eq!(tracked.index, GitChangeState::Modified);
        assert_eq!(tracked.worktree, GitChangeState::Unmodified);
        assert_eq!(new.index, GitChangeState::Added);
        assert_eq!(new.worktree, GitChangeState::Unmodified);
        assert_eq!(fs::read_to_string(dir.path().join("tracked.txt")).unwrap(), "two\n");
    }

    #[test]
    fn unstage_restores_head_index_and_preserves_worktree() {
        let dir = committed_tracked_repo();
        fs::write(dir.path().join("tracked.txt"), "local\n").unwrap();
        let service = GitRepositoryService;
        service.stage_file(dir.path(), "tracked.txt").unwrap();
        service.unstage_file(dir.path(), "tracked.txt").unwrap();
        let file = service
            .load(dir.path())
            .unwrap()
            .files
            .into_iter()
            .find(|file| file.path == Path::new("tracked.txt"))
            .unwrap();
        assert_eq!(file.index, GitChangeState::Unmodified);
        assert_eq!(file.worktree, GitChangeState::Modified);
        assert_eq!(fs::read_to_string(dir.path().join("tracked.txt")).unwrap(), "local\n");
        let index = gix::open(dir.path()).unwrap().open_index().unwrap();
        let count = index
            .entries_with_paths_by_filter_map(|entry_path, _| {
                (entry_path.to_str() == Ok("tracked.txt")).then_some(())
            })
            .count();
        assert_eq!(count, 1);
    }

    #[test]
    fn unstage_new_file_removes_index_entry_and_preserves_worktree() {
        let dir = committed_tracked_repo();
        fs::write(dir.path().join("new.txt"), "new\n").unwrap();
        let service = GitRepositoryService;
        service.stage_file(dir.path(), "new.txt").unwrap();
        service.unstage_file(dir.path(), "new.txt").unwrap();
        let files = service.load(dir.path()).unwrap().files;
        let new = files.iter().find(|file| file.path == Path::new("new.txt")).unwrap();
        assert_eq!(new.index, GitChangeState::Unmodified);
        assert_eq!(new.worktree, GitChangeState::Untracked);
        assert_eq!(fs::read_to_string(dir.path().join("new.txt")).unwrap(), "new\n");
    }

    #[test]
    fn unstage_deletion_restores_head_index_and_preserves_deleted_worktree() {
        let dir = committed_tracked_repo();
        fs::remove_file(dir.path().join("tracked.txt")).unwrap();
        let service = GitRepositoryService;
        service.stage_file(dir.path(), "tracked.txt").unwrap();
        service.unstage_file(dir.path(), "tracked.txt").unwrap();
        let file = service
            .load(dir.path())
            .unwrap()
            .files
            .into_iter()
            .find(|file| file.path == Path::new("tracked.txt"))
            .unwrap();
        assert_eq!(file.index, GitChangeState::Unmodified);
        assert_eq!(file.worktree, GitChangeState::Deleted);
        assert!(!dir.path().join("tracked.txt").exists());
    }

    #[test]
    fn stage_deleted_file_does_not_modify_worktree() {
        let dir = committed_tracked_repo();
        fs::remove_file(dir.path().join("tracked.txt")).unwrap();
        let service = GitRepositoryService;
        service.stage_file(dir.path(), "tracked.txt").unwrap();
        let file = service
            .load(dir.path())
            .unwrap()
            .files
            .into_iter()
            .find(|file| file.path == Path::new("tracked.txt"))
            .unwrap();
        assert_eq!(file.index, GitChangeState::Deleted);
        assert_eq!(file.worktree, GitChangeState::Unmodified);
        assert!(!dir.path().join("tracked.txt").exists());
    }

    #[test]
    fn computes_bounded_structured_diff_for_modified_and_untracked_files() {
        let dir = tracked_repo();
        fs::write(dir.path().join("tracked.txt"), "one\ntwo\n").unwrap();
        fs::write(dir.path().join("new.txt"), "new\n").unwrap();
        let service = GitRepositoryService;
        let modified = service.diff_file(dir.path(), "tracked.txt").unwrap();
        assert_eq!(modified.target, GitDiffTarget::IndexToWorktree);
        assert!(!modified.binary);
        assert!(modified
            .hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .any(|line| line.kind == GitDiffLineKind::Addition && line.text == "two"));
        let untracked = service.diff_file(dir.path(), "new.txt").unwrap();
        assert_eq!(untracked.target, GitDiffTarget::IndexToWorktree);
        assert_eq!(untracked.worktree, GitChangeState::Untracked);
        assert!(untracked
            .hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .all(|line| line.kind == GitDiffLineKind::Addition));
    }

    #[test]
    fn whitespace_only_index_to_worktree_change_is_non_empty() {
        let (dir, repo) = initialized_repo();
        fs::write(dir.path().join("tracked.txt"), "foo  bar\n").unwrap();
        add_index_entry(&repo, b"tracked.txt", b"foo bar\n");
        let diff = GitRepositoryService
            .diff_file_with_target(
                dir.path(),
                "tracked.txt",
                GitDiffTarget::IndexToWorktree,
            )
            .unwrap();
        let lines = diff.hunks.iter().flat_map(|hunk| &hunk.lines).collect::<Vec<_>>();
        assert!(lines.iter().any(|line| {
            line.kind == GitDiffLineKind::Deletion && line.text == "foo bar"
        }));
        assert!(lines.iter().any(|line| {
            line.kind == GitDiffLineKind::Addition && line.text == "foo  bar"
        }));
    }

    #[test]
    fn staged_addition_has_head_to_index_diff_but_clean_index_to_worktree() {
        let dir = tracked_repo();
        fs::write(dir.path().join("staged.php"), "<?php\nnew\n").unwrap();
        let repo = gix::open(dir.path()).unwrap();
        add_index_entry(&repo, b"staged.php", b"<?php\nnew\n");
        let head = repo
            .new_commit_as(
                gix::actor::SignatureRef {
                    name: b"Axiom Test".as_bstr(),
                    email: b"axiom@example.test".as_bstr(),
                    time: "1 +0000",
                },
                gix::actor::SignatureRef {
                    name: b"Axiom Test".as_bstr(),
                    email: b"axiom@example.test".as_bstr(),
                    time: "1 +0000",
                },
                "empty head",
                gix::hash::ObjectId::empty_tree(gix::hash::Kind::Sha1),
                std::iter::empty::<gix::hash::ObjectId>(),
            )
            .unwrap()
            .id;
        fs::write(dir.path().join(".git/HEAD"), format!("{head}\n")).unwrap();
        let service = GitRepositoryService;
        let staged = service
            .diff_file_with_target(dir.path(), "staged.php", GitDiffTarget::HeadToIndex)
            .unwrap();
        assert!(!staged.hunks.is_empty());
        assert_eq!(staged.target, GitDiffTarget::HeadToIndex);
        let unstaged = service
            .diff_file_with_target(
                dir.path(),
                "staged.php",
                GitDiffTarget::IndexToWorktree,
            )
            .unwrap();
        assert!(unstaged.hunks.is_empty());
        assert_eq!(unstaged.target, GitDiffTarget::IndexToWorktree);
    }

    #[test]
    fn staged_and_unstaged_changes_use_independent_targets() {
        let (dir, repo) = initialized_repo();
        fs::write(dir.path().join("tracked.txt"), "head\n").unwrap();
        commit_entries(&repo, &[("tracked.txt", b"head\n")]);
        write_index_entries(&repo, &[(b"tracked.txt", b"staged\n")]);
        fs::write(dir.path().join("tracked.txt"), "worktree\n").unwrap();

        let staged = GitRepositoryService
            .diff_file_with_target(
                dir.path(),
                "tracked.txt",
                GitDiffTarget::HeadToIndex,
            )
            .unwrap();
        let unstaged = GitRepositoryService
            .diff_file_with_target(
                dir.path(),
                "tracked.txt",
                GitDiffTarget::IndexToWorktree,
            )
            .unwrap();

        assert_eq!(staged.target, GitDiffTarget::HeadToIndex);
        assert_eq!(unstaged.target, GitDiffTarget::IndexToWorktree);
        assert!(!staged.hunks.is_empty());
        assert!(!unstaged.hunks.is_empty());
        assert!(staged
            .hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .any(|line| line.kind == GitDiffLineKind::Deletion && line.text == "head"));
        assert!(staged
            .hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .any(|line| line.kind == GitDiffLineKind::Addition && line.text == "staged"));
        assert!(unstaged
            .hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .any(|line| line.kind == GitDiffLineKind::Deletion && line.text == "staged"));
        assert!(unstaged
            .hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .any(|line| line.kind == GitDiffLineKind::Addition && line.text == "worktree"));
    }

    #[test]
    fn unstaged_deletion_diffs_index_to_missing_worktree() {
        let (dir, repo) = initialized_repo();
        write_index_entries(&repo, &[(b"deleted.txt", b"old\n")]);
        let diff = GitRepositoryService
            .diff_file_with_target(
                dir.path(),
                "deleted.txt",
                GitDiffTarget::IndexToWorktree,
            )
            .unwrap();

        assert_eq!(diff.worktree, GitChangeState::Deleted);
        assert_eq!(diff.target, GitDiffTarget::IndexToWorktree);
        assert!(diff
            .hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .any(|line| line.kind == GitDiffLineKind::Deletion && line.text == "old"));
        assert!(diff
            .hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .all(|line| line.kind != GitDiffLineKind::Addition));
    }

    #[test]
    fn staged_deletion_diffs_head_to_missing_index() {
        let (dir, repo) = initialized_repo();
        commit_entries(&repo, &[("deleted.txt", b"old\n")]);
        write_index_entries(&repo, &[]);
        let diff = GitRepositoryService
            .diff_file_with_target(dir.path(), "deleted.txt", GitDiffTarget::HeadToIndex)
            .unwrap();

        assert_eq!(diff.index, GitChangeState::Deleted);
        assert_eq!(diff.target, GitDiffTarget::HeadToIndex);
        assert!(diff
            .hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .any(|line| line.kind == GitDiffLineKind::Deletion && line.text == "old"));
        assert!(diff
            .hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .all(|line| line.kind != GitDiffLineKind::Addition));
    }

    #[test]
    fn staged_addition_and_deletion_are_independent() {
        let (dir, repo) = initialized_repo();
        commit_entries(&repo, &[("deleted.txt", b"old\n")]);
        fs::write(dir.path().join("added.txt"), "new\n").unwrap();
        write_index_entries(&repo, &[(b"added.txt", b"new\n")]);

        let added = GitRepositoryService
            .diff_file_with_target(dir.path(), "added.txt", GitDiffTarget::HeadToIndex)
            .unwrap();
        let deleted = GitRepositoryService
            .diff_file_with_target(dir.path(), "deleted.txt", GitDiffTarget::HeadToIndex)
            .unwrap();

        assert_eq!(added.index, GitChangeState::Added);
        assert!(added
            .hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .all(|line| line.kind == GitDiffLineKind::Addition));
        assert_eq!(deleted.index, GitChangeState::Deleted);
        assert!(deleted
            .hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .all(|line| line.kind == GitDiffLineKind::Deletion));
    }

    #[test]
    fn root_and_nested_repository_relative_paths_reach_terminal_diff_states() {
        let dir = tracked_repo();
        fs::create_dir_all(dir.path().join("crates/axiom-app/src")).unwrap();
        fs::write(dir.path().join("test.php"), "<?php\nold\n").unwrap();
        fs::write(
            dir.path().join("crates/axiom-app/src/workspace_view.rs"),
            "old\n",
        )
        .unwrap();
        let repo = gix::open(dir.path()).unwrap();
        add_index_entry(&repo, b"test.php", b"<?php\nold\n");
        add_index_entry(
            &repo,
            b"crates/axiom-app/src/workspace_view.rs",
            b"old\n",
        );
        fs::write(dir.path().join("test.php"), "<?php\nnew\n").unwrap();
        fs::write(
            dir.path().join("crates/axiom-app/src/workspace_view.rs"),
            "new\n",
        )
        .unwrap();
        let service = GitRepositoryService;
        for path in ["test.php", "crates/axiom-app/src/workspace_view.rs"] {
            let diff = service.diff_file(dir.path(), path).unwrap();
            assert_eq!(diff.path, Path::new(path));
            assert_eq!(diff.target, GitDiffTarget::IndexToWorktree);
            assert!(!diff.hunks.is_empty());
            assert!(!diff.truncated);
            assert!(diff.hunks.iter().flat_map(|hunk| &hunk.lines).any(|line| {
                line.kind == GitDiffLineKind::Deletion
            }));
            assert!(diff.hunks.iter().flat_map(|hunk| &hunk.lines).any(|line| {
                line.kind == GitDiffLineKind::Addition
            }));
        }
    }

    #[test]
    fn binary_diff_is_structured_without_text_lines() {
        let dir = tracked_repo();
        fs::write(dir.path().join("tracked.txt"), [0_u8, 1, 2]).unwrap();
        let diff = GitRepositoryService.diff_file(dir.path(), "tracked.txt").unwrap();
        assert!(diff.binary);
        assert!(diff.hunks.is_empty());
    }

    #[test]
    fn parses_and_sanitizes_github_remotes() {
        let cases = [
            ("https://github.com/owner/repo.git", true),
            ("https://github.com/owner/repo", true),
            ("git@github.com:owner/repo.git", true),
            ("ssh://git@github.com/owner/repo.git", true),
            ("https://gitlab.com/owner/repo.git", false),
        ];
        for (url, github) in cases {
            let (provider, owner, repository) = parse_remote_url(url);
            assert_eq!(provider == GitRemoteProvider::GitHub, github);
            if github {
                assert_eq!(owner.as_deref(), Some("owner"));
                assert_eq!(repository.as_deref(), Some("repo"));
            }
        }
        let sanitized = sanitize_remote_url("https://user:secret@github.com/owner/repo.git");
        assert!(!sanitized.contains("secret"));
        assert!(!sanitized.contains("user@"));

        let (dir, _) = initialized_repo();
        fs::OpenOptions::new()
            .append(true)
            .open(dir.path().join(".git/config"))
            .unwrap()
            .write_all(
                b"\n[remote \"origin\"]\n\turl = https://user:secret@github.com/owner/repo.git\n",
            )
            .unwrap();
        let remote = GitRepositoryService
            .load(dir.path())
            .unwrap()
            .remotes
            .into_iter()
            .next()
            .unwrap();
        assert_eq!(remote.provider, GitRemoteProvider::GitHub);
        assert!(!remote.url.contains("secret"));
        assert!(!remote.url.contains("user@"));
    }
}
