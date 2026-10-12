use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::io::Read;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use axiom_agent::{
    AxiomMemoryScopeIds, BackgroundMemoryService, SharedMemoryService, default_data_directory,
};

pub(crate) fn initialize_workspace_memory(root: &Path) -> Option<SharedMemoryService> {
    initialize_workspace_memory_at(default_data_directory()?, root)
}

fn initialize_workspace_memory_at(
    data_directory: PathBuf,
    root: &Path,
) -> Option<SharedMemoryService> {
    let scope_ids = scope_ids_for_root(root)?;
    let service = BackgroundMemoryService::start_ready(
        data_directory,
        scope_ids,
        std::time::Duration::from_secs(2),
    )
    .ok()?;
    Some(SharedMemoryService::new(Box::new(service)))
}

fn scope_ids_for_root(root: &Path) -> Option<AxiomMemoryScopeIds> {
    let root = std::fs::canonicalize(root).ok()?;
    let checkout_id = checkout_memory_id(&root)?;
    // The data directory is already OS-user-scoped, so a constant local user
    // identity avoids persisting the account name or profile path.
    Some(AxiomMemoryScopeIds::new(
        "local-user-v1",
        format!("workspace-{checkout_id}"),
        format!("project-{checkout_id}"),
    ))
}

#[derive(Deserialize, Serialize)]
struct CheckoutMemoryIdentity {
    id: String,
    checkout_root: String,
}

fn checkout_memory_id(root: &Path) -> Option<String> {
    let metadata_dir = root.join(".axiom").join("memory-identities");
    fs::create_dir_all(&metadata_dir).ok()?;
    let _lock = CheckoutIdentityLock::acquire(root)?;
    let checkout_root = normalized_checkout_path(root);
    let mut matches = Vec::new();
    for entry in fs::read_dir(&metadata_dir).ok()? {
        let entry = entry.ok()?;
        if !entry.file_type().ok()?.is_file() || entry.path().extension()? != "json" {
            continue;
        }
        let Ok(bytes) = fs::read(entry.path()) else {
            continue;
        };
        let Ok(identity) = serde_json::from_slice::<CheckoutMemoryIdentity>(&bytes) else {
            continue;
        };
        if identity.checkout_root == checkout_root && valid_checkout_id(&identity.id) {
            matches.push(identity.id);
        }
    }
    if matches.len() == 1 {
        return matches.pop();
    }
    // Multiple valid identities for one checkout are ambiguous legacy state.
    // Keep every file untouched and refuse to select one implicitly.
    if matches.len() > 1 {
        return None;
    }

    // The recorded checkout path is provenance used only to reject copied
    // metadata. A checkout copied or moved elsewhere receives a fresh ID;
    // existing memory files are deliberately not migrated.
    let id = random_checkout_id()?;
    let identity = CheckoutMemoryIdentity {
        id: id.clone(),
        checkout_root,
    };
    let bytes = serde_json::to_vec_pretty(&identity).ok()?;
    let final_path = metadata_dir.join(format!("{id}.json"));
    let temp_path = metadata_dir.join(format!(".{id}.tmp"));
    let mut temp = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)
        .ok()?;
    if temp.write_all(&bytes).is_err() || temp.sync_all().is_err() {
        let _ = fs::remove_file(&temp_path);
        return None;
    }
    drop(temp);
    publish_identity(temp_path, final_path).ok()?;
    Some(id)
}

fn publish_identity(temp_path: PathBuf, final_path: PathBuf) -> std::io::Result<()> {
    match fs::hard_link(&temp_path, &final_path) {
        Ok(()) => {
            let _ = fs::remove_file(temp_path);
            Ok(())
        }
        Err(error) => {
            let _ = fs::remove_file(temp_path);
            Err(error)
        }
    }
}

struct CheckoutIdentityLock {
    #[cfg(windows)]
    handle: *mut std::ffi::c_void,
    #[cfg(unix)]
    file: std::fs::File,
}

impl CheckoutIdentityLock {
    fn acquire(root: &Path) -> Option<Self> {
        #[cfg(windows)]
        {
            let name = format!("Local\\Axiom-Memory-Identity-{}", lock_path_id(root));
            let wide = name.encode_utf16().chain(std::iter::once(0)).collect::<Vec<_>>();
            let handle = unsafe { create_mutex_w(wide.as_ptr()) };
            if handle.is_null() {
                return None;
            }
            let result = unsafe { wait_for_single_object(handle, INFINITE) };
            if result != WAIT_OBJECT_0 && result != WAIT_ABANDONED {
                unsafe {
                    close_handle(handle);
                }
                return None;
            }
            return Some(Self { handle });
        }

        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let path = root.join(".axiom").join("memory-identities.lock");
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .open(path)
                .ok()?;
            if unsafe { flock(file.as_raw_fd(), LOCK_EX) } != 0 {
                return None;
            }
            return Some(Self { file });
        }

        #[cfg(not(any(windows, unix)))]
        {
            let _ = root;
            None
        }
    }
}

#[cfg(windows)]
impl Drop for CheckoutIdentityLock {
    fn drop(&mut self) {
        unsafe {
            let _ = release_mutex(self.handle);
            let _ = close_handle(self.handle);
        }
    }
}

fn lock_path_id(path: &Path) -> String {
    let normalized = normalized_checkout_path(path);
    let hash = normalized
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        });
    format!("{hash:016x}")
}

#[cfg(windows)]
const INFINITE: u32 = 0xffff_ffff;
#[cfg(windows)]
const WAIT_OBJECT_0: u32 = 0;
#[cfg(windows)]
const WAIT_ABANDONED: u32 = 0x80;

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateMutexW(
        mutex_attributes: *const std::ffi::c_void,
        initial_owner: i32,
        name: *const u16,
    ) -> *mut std::ffi::c_void;
    fn WaitForSingleObject(handle: *mut std::ffi::c_void, milliseconds: u32) -> u32;
    fn ReleaseMutex(handle: *mut std::ffi::c_void) -> i32;
    fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
}

#[cfg(windows)]
unsafe fn create_mutex_w(name: *const u16) -> *mut std::ffi::c_void {
    unsafe { CreateMutexW(std::ptr::null(), 0, name) }
}

#[cfg(windows)]
unsafe fn wait_for_single_object(handle: *mut std::ffi::c_void, milliseconds: u32) -> u32 {
    unsafe { WaitForSingleObject(handle, milliseconds) }
}

#[cfg(windows)]
unsafe fn release_mutex(handle: *mut std::ffi::c_void) -> i32 {
    unsafe { ReleaseMutex(handle) }
}

#[cfg(windows)]
unsafe fn close_handle(handle: *mut std::ffi::c_void) -> i32 {
    unsafe { CloseHandle(handle) }
}

#[cfg(unix)]
const LOCK_EX: i32 = 2;
#[cfg(unix)]
unsafe extern "C" {
    fn flock(fd: std::os::fd::RawFd, operation: i32) -> i32;
}

fn normalized_checkout_path(path: &Path) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    #[cfg(windows)]
    let path = path.to_lowercase();
    path
}

fn valid_checkout_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn random_checkout_id() -> Option<String> {
    let mut bytes = [0_u8; 16];
    #[cfg(unix)]
    {
        std::fs::File::open("/dev/urandom")
            .ok()?
            .read_exact(&mut bytes)
            .ok()?;
    }
    #[cfg(windows)]
    {
        #[link(name = "ole32")]
        unsafe extern "system" {
            fn CoCreateGuid(guid: *mut [u8; 16]) -> i32;
        }
        // CoCreateGuid supplies the OS-generated 128-bit checkout identifier.
        if unsafe { CoCreateGuid(&mut bytes) } < 0 {
            return None;
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        return None;
    }
    Some(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axiom_agent::{
        AxiomMemoryBackend, MemoryObservation, MemoryReadError, MemoryScope, MemoryService,
    };
    use std::sync::{Arc, Barrier, Mutex};

    fn temp_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "axiom-memory-runtime-{label}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn active_workspace_initializes_available_persistent_backend() {
        let root = temp_root("available");
        let data = temp_root("data");
        let memory = initialize_workspace_memory_at(data.clone(), &root).unwrap();
        assert!(memory.is_available());
        assert!(memory.history(MemoryScope::Workspace, 10).is_empty());
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(data);
    }

    #[test]
    fn workspace_memory_persists_across_construction_and_isolates_distinct_roots() {
        let first = temp_root("identity-a");
        let second = temp_root("identity-b");
        let data = temp_root("identity-data");
        let first_memory = initialize_workspace_memory_at(data.clone(), &first).unwrap();
        let mut agent_memory = first_memory.boxed();
        let session = agent_memory.session_start(MemoryScope::Workspace);
        agent_memory.observe(
            session,
            MemoryObservation::Discovery("first workspace fact".into()),
        );
        agent_memory.session_end(session);

        let reopened_first = initialize_workspace_memory_at(data.clone(), &first).unwrap();
        assert_eq!(
            reopened_first.history(MemoryScope::Workspace, 10)[0].content,
            "first workspace fact"
        );
        let second_memory = initialize_workspace_memory_at(data.clone(), &second).unwrap();
        assert!(second_memory.history(MemoryScope::Workspace, 10).is_empty());
        assert_ne!(
            scope_ids_for_test(&first).workspace,
            scope_ids_for_test(&second).workspace
        );
        let _ = std::fs::remove_dir_all(first);
        let _ = std::fs::remove_dir_all(second);
        let _ = std::fs::remove_dir_all(data);
    }

    #[test]
    fn checkout_identity_is_persisted_and_copied_metadata_gets_a_new_identity() {
        let original = temp_root("identity-copy-source");
        let copy = temp_root("identity-copy-destination");
        let original_ids = scope_ids_for_root(&original).unwrap();
        assert_eq!(scope_ids_for_root(&original).unwrap(), original_ids);

        let source_metadata = original.join(".axiom").join("memory-identities");
        let copied_metadata = copy.join(".axiom").join("memory-identities");
        std::fs::create_dir_all(&copied_metadata).unwrap();
        for entry in std::fs::read_dir(source_metadata).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), copied_metadata.join(entry.file_name())).unwrap();
        }
        let copied_ids = scope_ids_for_root(&copy).unwrap();
        assert_ne!(copied_ids.workspace, original_ids.workspace);
        assert_ne!(copied_ids.project, original_ids.project);
        assert_eq!(scope_ids_for_root(&copy).unwrap(), copied_ids);

        let _ = std::fs::remove_dir_all(original);
        let _ = std::fs::remove_dir_all(copy);
    }

    #[test]
    fn concurrent_initializations_share_one_identity_and_scope() {
        let root = temp_root("identity-concurrent");
        let data = temp_root("identity-concurrent-data");
        let participants = 8;
        let barrier = Arc::new(Barrier::new(participants));
        let ids = Arc::new(Mutex::new(Vec::new()));
        let mut workers = Vec::new();
        for _ in 0..participants {
            let root = root.clone();
            let barrier = barrier.clone();
            let ids = ids.clone();
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                let scope_ids = scope_ids_for_root(&root).unwrap();
                ids.lock().unwrap().push(scope_ids);
            }));
        }
        for worker in workers {
            worker.join().unwrap();
        }
        let ids = ids.lock().unwrap();
        assert_eq!(ids.len(), participants);
        assert!(ids.windows(2).all(|pair| pair[0] == pair[1]));
        let shared_ids = ids[0].clone();
        drop(ids);

        for index in 0..participants {
            let backend = AxiomMemoryBackend::new(&data, shared_ids.clone());
            backend.initialize_storage().unwrap();
            backend
                .remember(
                    MemoryScope::Workspace,
                    &format!("concurrent participant {index}"),
                )
                .unwrap();
        }
        let reopened = AxiomMemoryBackend::new(&data, shared_ids);
        assert_eq!(
            reopened.history(MemoryScope::Workspace, participants).len(),
            participants
        );

        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(data);
    }

    #[test]
    fn failed_metadata_publication_cleans_temporary_file_and_can_recover() {
        let root = temp_root("identity-publication-failure");
        let metadata_dir = root.join(".axiom").join("memory-identities");
        std::fs::create_dir_all(&metadata_dir).unwrap();
        let id = "0123456789abcdef0123456789abcdef";
        let final_path = metadata_dir.join(format!("{id}.json"));
        std::fs::create_dir(&final_path).unwrap();
        let temp_path = metadata_dir.join(format!(".{id}.tmp"));
        std::fs::write(&temp_path, b"partial metadata").unwrap();
        assert!(publish_identity(temp_path.clone(), final_path.clone()).is_err());
        assert!(!temp_path.exists());
        assert!(final_path.is_dir());

        std::fs::remove_dir_all(final_path).unwrap();
        let scope_ids = scope_ids_for_root(&root).unwrap();
        assert!(valid_checkout_id(scope_ids.workspace.strip_prefix("workspace-").unwrap()));
        assert_eq!(scope_ids_for_root(&root).unwrap(), scope_ids);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn legacy_path_scoped_memory_is_preserved_without_automatic_migration() {
        let root = temp_root("legacy-root");
        let data = temp_root("legacy-data");
        let canonical = std::fs::canonicalize(&root).unwrap();
        let old_id = legacy_path_id_for_test(&canonical);
        let legacy_file = data
            .join("workspace")
            .join(format!("workspace-{old_id}.json"));
        std::fs::create_dir_all(legacy_file.parent().unwrap()).unwrap();
        let legacy_bytes = br#"{"schema_version":1,"scope":"workspace","scope_id":"workspace-legacy","next_record_id":1,"records":[]}"#;
        std::fs::write(&legacy_file, legacy_bytes).unwrap();

        let memory = initialize_workspace_memory_at(data.clone(), &root).unwrap();
        assert!(memory.history(MemoryScope::Workspace, 10).is_empty());
        assert_eq!(std::fs::read(&legacy_file).unwrap(), legacy_bytes);

        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(data);
    }

    #[test]
    fn agent_and_ui_clones_observe_the_same_workspace_memory() {
        use crate::ai::tools::{ToolArguments, ToolKind, ToolName, ToolRegistry, ToolRequest};
        use axiom_project::{
            project_directory::ProjectDirectoryCapability, project_read::ProjectReadCapability,
        };

        let root = temp_root("shared-root");
        let data = temp_root("shared-data");
        let ui_memory = initialize_workspace_memory_at(data.clone(), &root).unwrap();
        let mut agent_memory = ui_memory.boxed();
        let session = agent_memory.session_start(MemoryScope::Workspace);
        agent_memory.observe(
            session,
            MemoryObservation::Discovery("shared workspace fact".into()),
        );
        agent_memory.session_end(session);

        let snapshot = ui_memory.snapshot(MemoryScope::Workspace, 10).unwrap();
        assert_eq!(snapshot.history.len(), 1);
        assert_eq!(snapshot.history[0].content, "shared workspace fact");

        let registry = ToolRegistry::new(
            ProjectReadCapability::new(&root).unwrap(),
            ProjectDirectoryCapability::new(&root).unwrap(),
        )
        .with_memory_service_in_scope(ui_memory.boxed(), MemoryScope::Workspace);
        assert_eq!(
            registry.kind(&ToolName::MemorySearch),
            Some(ToolKind::ReadOnly)
        );
        assert_eq!(
            registry.kind(&ToolName::MemoryGet),
            Some(ToolKind::ReadOnly)
        );
        let search = registry.execute(ToolRequest {
            name: ToolName::MemorySearch,
            arguments: ToolArguments::MemorySearch {
                query: "shared workspace fact".into(),
                limit: Some(5),
            },
        });
        assert!(
            matches!(search.result, Ok(output) if output.content.contains("shared workspace fact"))
        );
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(data);
    }

    #[test]
    fn memory_remember_dispatch_persists_deduplicates_and_recovers_after_restart() {
        use crate::ai::tools::{
            ToolArguments, ToolError, ToolKind, ToolName, ToolRegistry, ToolRequest, ToolResult,
        };
        use axiom_project::{
            project_directory::ProjectDirectoryCapability, project_read::ProjectReadCapability,
        };

        let first_root = temp_root("remember-root-a");
        let other_root = temp_root("remember-root-b");
        let data = temp_root("remember-data");
        let memory = initialize_workspace_memory_at(data.clone(), &first_root).unwrap();
        let registry = ToolRegistry::new(
            ProjectReadCapability::new(&first_root).unwrap(),
            ProjectDirectoryCapability::new(&first_root).unwrap(),
        )
        .with_memory_service_in_scope(memory.boxed(), MemoryScope::Workspace);
        assert_eq!(
            registry.kind(&ToolName::MemoryRemember),
            Some(ToolKind::Mutating)
        );

        let rejected = registry.execute(ToolRequest {
            name: ToolName::MemoryRemember,
            arguments: ToolArguments::MemoryRemember {
                content: "Authorization: Bearer secret-value".into(),
            },
        });
        assert!(matches!(
            rejected.result,
            Err(ToolError::MemoryWriteRejected)
        ));

        let remember = || ToolRequest {
            name: ToolName::MemoryRemember,
            arguments: ToolArguments::MemoryRemember {
                content: "The workspace uses a persistent Axiom memory store.".into(),
            },
        };
        for _ in 0..2 {
            let ToolResult {
                result: Ok(output), ..
            } = registry.execute(remember())
            else {
                panic!("memory_remember must persist the observation");
            };
            let response: serde_json::Value = serde_json::from_str(&output.content).unwrap();
            assert_eq!(response["persisted"], true);
            assert_eq!(response["scope"], "workspace");
        }
        assert_eq!(memory.history(MemoryScope::Workspace, 10).len(), 1);

        let restarted = initialize_workspace_memory_at(data.clone(), &first_root).unwrap();
        let restarted_registry = ToolRegistry::new(
            ProjectReadCapability::new(&first_root).unwrap(),
            ProjectDirectoryCapability::new(&first_root).unwrap(),
        )
        .with_memory_service_in_scope(restarted.boxed(), MemoryScope::Workspace);
        let search = restarted_registry.execute(ToolRequest {
            name: ToolName::MemorySearch,
            arguments: ToolArguments::MemorySearch {
                query: "persistent Axiom memory store".into(),
                limit: Some(1),
            },
        });
        let ToolResult {
            result: Ok(search_output), ..
        } = search
        else {
            panic!("memory_search must recover the persisted observation");
        };
        let search_response: serde_json::Value =
            serde_json::from_str(&search_output.content).unwrap();
        assert_eq!(
            search_response["results"][0]["content"],
            "The workspace uses a persistent Axiom memory store."
        );
        let id = search_response["results"][0]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let get = restarted_registry.execute(ToolRequest {
            name: ToolName::MemoryGet,
            arguments: ToolArguments::MemoryGet { id },
        });
        assert!(matches!(get.result, Ok(output) if output.content.contains(
            "The workspace uses a persistent Axiom memory store."
        )));

        let isolated = initialize_workspace_memory_at(data.clone(), &other_root).unwrap();
        assert!(isolated
            .query(
                MemoryScope::Workspace,
                "persistent Axiom memory store",
                10
            )
            .is_empty());

        let _ = std::fs::remove_dir_all(first_root);
        let _ = std::fs::remove_dir_all(other_root);
        let _ = std::fs::remove_dir_all(data);
    }

    #[test]
    fn memory_remember_reports_storage_failure_without_success_payload() {
        use crate::ai::tools::{
            ToolArguments, ToolError, ToolName, ToolRegistry, ToolRequest,
        };
        use axiom_project::{
            project_directory::ProjectDirectoryCapability, project_read::ProjectReadCapability,
        };

        let root = temp_root("remember-storage-root");
        let data = temp_root("remember-storage-data");
        let memory = initialize_workspace_memory_at(data.clone(), &root).unwrap();
        let workspace_directory = data.join("workspace");
        std::fs::remove_dir_all(&workspace_directory).unwrap();
        std::fs::write(&workspace_directory, b"block workspace directory").unwrap();
        let registry = ToolRegistry::new(
            ProjectReadCapability::new(&root).unwrap(),
            ProjectDirectoryCapability::new(&root).unwrap(),
        )
        .with_memory_service_in_scope(memory.boxed(), MemoryScope::Workspace);

        let result = registry.execute(ToolRequest {
            name: ToolName::MemoryRemember,
            arguments: ToolArguments::MemoryRemember {
                content: "valid but not writable".into(),
            },
        });
        assert!(matches!(result.result, Err(ToolError::MemoryWriteFailed)));

        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(data);
    }

    #[test]
    fn unavailable_initialization_returns_none_without_panicking() {
        let root = temp_root("failure-root");
        let data_file = temp_root("failure-parent").join("not-a-directory");
        std::fs::write(&data_file, b"block directory creation").unwrap();
        assert!(initialize_workspace_memory_at(data_file.clone(), &root).is_none());
        let _ = std::fs::remove_dir_all(root);
        if let Some(parent) = data_file.parent() {
            let _ = std::fs::remove_dir_all(parent);
        }
    }

    #[test]
    fn snapshot_reports_corrupt_storage_as_unavailable_not_empty() {
        let root = temp_root("corrupt-root");
        let data = temp_root("corrupt-data");
        let memory = initialize_workspace_memory_at(data.clone(), &root).unwrap();
        let workspace_file = data
            .join("workspace")
            .join(format!("{}.json", scope_ids_for_test(&root).workspace));
        std::fs::write(workspace_file, b"invalid json").unwrap();
        assert!(memory.snapshot(MemoryScope::Workspace, 10).is_err());
        assert_eq!(
            memory.query_checked(MemoryScope::Workspace, "anything", 10),
            Err(MemoryReadError::CorruptStore)
        );
        assert_eq!(
            memory.briefing_checked(MemoryScope::Workspace, 10),
            Err(MemoryReadError::CorruptStore)
        );
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(data);
    }

    fn scope_ids_for_test(root: &Path) -> AxiomMemoryScopeIds {
        scope_ids_for_root(root).unwrap()
    }

    fn legacy_path_id_for_test(path: &Path) -> String {
        let normalized = path.to_string_lossy().replace('\\', "/");
        #[cfg(windows)]
        let normalized = normalized.to_lowercase();
        let hash = normalized
            .as_bytes()
            .iter()
            .fold(0xcbf29ce484222325_u64, |hash, byte| {
                (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
            });
        format!("{hash:016x}")
    }
}
