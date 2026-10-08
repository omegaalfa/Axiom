//! Cross-platform PTY session and VT screen state for Axiom.

use std::{
    collections::HashMap,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{Receiver, SyncSender, TrySendError, sync_channel},
    },
    thread,
};

#[cfg(unix)]
use std::env;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalLinkKind {
    File,
    FileLine { line: u32, column: Option<u32> },
    Url,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalLink {
    pub range: std::ops::Range<usize>,
    pub target: String,
    pub kind: TerminalLinkKind,
    pub path: Option<PathBuf>,
}

/// Lightweight, headless detector for shell/compiler output. Resolution is
/// relative to the session cwd and only existing files become file links.
pub fn detect_links(text: &str, cwd: &Path) -> Vec<TerminalLink> {
    let mut links = Vec::new();
    for (start, raw) in text.split_whitespace().scan(0usize, |offset, token| {
        let start = text[*offset..]
            .find(token)
            .map(|i| *offset + i)
            .unwrap_or(*offset);
        *offset = start + token.len();
        Some((start, token))
    }) {
        let token = raw.trim_matches(|c: char| "()[]{}<>,;\"'".contains(c));
        if token.is_empty() {
            continue;
        }
        let token_start = start + raw.find(token).unwrap_or(0);
        if token.starts_with("http://") || token.starts_with("https://") {
            links.push(TerminalLink {
                range: token_start..token_start + token.len(),
                target: token.to_owned(),
                kind: TerminalLinkKind::Url,
                path: None,
            });
            continue;
        }
        let (path_text, line, column) = split_location(token);
        let candidate = PathBuf::from(path_text);
        let path = if candidate.is_absolute() {
            candidate
        } else {
            cwd.join(candidate)
        };
        if path.is_file() {
            links.push(TerminalLink {
                range: token_start..token_start + token.len(),
                target: token.to_owned(),
                kind: line.map_or(TerminalLinkKind::File, |line| TerminalLinkKind::FileLine {
                    line,
                    column,
                }),
                path: Some(path),
            });
        }
    }
    links
}

fn split_location(token: &str) -> (&str, Option<u32>, Option<u32>) {
    let mut parts = token.rsplitn(3, ':');
    let last = parts.next();
    let second = parts.next();
    let prefix = parts.next();
    let parse = |value: Option<&str>| value.and_then(|v| v.parse::<u32>().ok());
    if let (Some(a), Some(b), Some(path)) = (parse(last), parse(second), prefix) {
        return (path, Some(b), Some(a));
    }
    if let (Some(a), Some(path)) = (parse(last), second) {
        // Preserve the drive colon in Windows paths (C:\foo.php:42).
        return (path, Some(a), None);
    }
    (token, None, None)
}

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

const DEFAULT_ROWS: u16 = 24;
const DEFAULT_COLS: u16 = 80;
const DEFAULT_SCROLLBACK: usize = 10_000;
const INPUT_QUEUE_CAPACITY: usize = 64;
const MAX_INPUT_BYTES: usize = 64 * 1024;

pub fn clamp_terminal_size(rows: u16, cols: u16) -> (u16, u16) {
    (rows.max(1), cols.max(1))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalProfile {
    Unix { program: PathBuf },
    PowerShell { program: String },
    Cmd,
    Wsl,
    Ubuntu,
}

impl TerminalProfile {
    pub fn platform_default() -> Self {
        #[cfg(windows)]
        {
            Self::PowerShell {
                program: "powershell.exe".to_owned(),
            }
        }
        #[cfg(not(windows))]
        {
            Self::Unix {
                program: env::var_os("SHELL")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("/bin/bash")),
            }
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Unix { program } => program
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("shell"),
            Self::PowerShell { .. } => "PowerShell",
            Self::Cmd => "CMD",
            Self::Wsl => "WSL",
            Self::Ubuntu => "Ubuntu",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::Unix { .. } => "unix",
            Self::PowerShell { .. } => "powershell",
            Self::Cmd => "cmd",
            Self::Wsl => "wsl",
            Self::Ubuntu => "ubuntu",
        }
    }

    pub fn built_in_profiles() -> Vec<Self> {
        #[cfg(windows)]
        {
            vec![
                Self::PowerShell {
                    program: "powershell.exe".to_owned(),
                },
                Self::Cmd,
                Self::Ubuntu,
            ]
        }
        #[cfg(not(windows))]
        {
            vec![Self::Unix {
                program: env::var_os("SHELL")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("/bin/bash")),
            }]
        }
    }

    pub fn wsl_available() -> bool {
        #[cfg(windows)]
        {
            let Ok(output) = std::process::Command::new("wsl.exe")
                .args(["--list", "--quiet"])
                .output()
            else {
                return false;
            };
            let utf16 = output
                .stdout
                .chunks_exact(2)
                .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]));
            let listed = String::from_utf16_lossy(&utf16.collect::<Vec<_>>());
            listed
                .lines()
                .map(str::trim)
                .any(|distro| distro.to_ascii_lowercase().starts_with("ubuntu"))
        }
        #[cfg(not(windows))]
        {
            false
        }
    }

    fn command(&self) -> CommandBuilder {
        match self {
            Self::Unix { program } => CommandBuilder::new(program),
            Self::PowerShell { program } => {
                let mut command = CommandBuilder::new(program);
                command.args(["-NoLogo", "-NoProfile", "-NoExit"]);
                command
            }
            Self::Cmd => {
                let mut command = CommandBuilder::new("cmd.exe");
                command.args(["/D", "/Q"]);
                command
            }
            Self::Wsl => CommandBuilder::new("wsl.exe"),
            Self::Ubuntu => {
                let mut command = CommandBuilder::new("wsl.exe");
                command.args(["-d", "Ubuntu"]);
                command
            }
        }
    }
}

pub fn windows_path_to_wsl(path: impl AsRef<Path>) -> Option<String> {
    let path = path.as_ref().to_str()?.replace('\\', "/");
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return Some(format!(
            "/mnt/{}/{}",
            (bytes[0] as char).to_ascii_lowercase(),
            path[2..].trim_start_matches('/')
        ));
    }
    path.starts_with('/').then_some(path)
}

pub type TerminalSessionId = u64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSessionInfo {
    pub id: TerminalSessionId,
    pub label: String,
    pub profile_id: &'static str,
}

struct ManagedTerminal {
    info: TerminalSessionInfo,
    session: Arc<TerminalSession>,
}

pub struct TerminalManager {
    sessions: HashMap<TerminalSessionId, ManagedTerminal>,
    active_session: Option<TerminalSessionId>,
    next_id: TerminalSessionId,
    max_sessions: usize,
    profile_counts: HashMap<&'static str, usize>,
}

impl TerminalManager {
    pub fn new(max_sessions: usize) -> Self {
        Self {
            sessions: HashMap::new(),
            active_session: None,
            next_id: 1,
            max_sessions: max_sessions.max(1),
            profile_counts: HashMap::new(),
        }
    }

    pub fn create_session(
        &mut self,
        cwd: impl AsRef<Path>,
        profile: TerminalProfile,
    ) -> io::Result<TerminalSessionId> {
        if self.sessions.len() >= self.max_sessions {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "maximum terminal session count reached",
            ));
        }
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        let profile_id = profile.id();
        let count = self.profile_counts.entry(profile_id).or_insert(0);
        *count += 1;
        let label = format!("{} {}", profile.label(), *count);
        let session = Arc::new(TerminalSession::spawn(cwd, profile)?);
        self.sessions.insert(
            id,
            ManagedTerminal {
                info: TerminalSessionInfo {
                    id,
                    label,
                    profile_id,
                },
                session,
            },
        );
        self.active_session = Some(id);
        Ok(id)
    }

    pub fn close_session(&mut self, id: TerminalSessionId) -> bool {
        let Some(terminal) = self.sessions.remove(&id) else {
            return false;
        };
        let _ = terminal.session.terminate();
        if self.active_session == Some(id) {
            self.active_session = self.sessions.keys().max().copied();
        }
        true
    }

    pub fn activate_session(&mut self, id: TerminalSessionId) -> bool {
        if self.sessions.contains_key(&id) {
            self.active_session = Some(id);
            true
        } else {
            false
        }
    }

    pub fn active_session(&self) -> Option<TerminalSessionId> {
        self.active_session
    }

    pub fn session(&self, id: TerminalSessionId) -> Option<Arc<TerminalSession>> {
        self.sessions.get(&id).map(|terminal| terminal.session.clone())
    }

    pub fn sessions(&self) -> Vec<TerminalSessionInfo> {
        let mut sessions = self
            .sessions
            .values()
            .map(|terminal| terminal.info.clone())
            .collect::<Vec<_>>();
        sessions.sort_by_key(|session| session.id);
        sessions
    }

    pub fn len(&self) -> usize {
        self.sessions.len()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalState {
    Running,
    Exited,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalEvent {
    pub generation: u64,
}

struct ScreenState {
    parser: Mutex<vt100::Parser>,
    revision: AtomicU64,
    exited: AtomicBool,
    state: AtomicU64,
    notifications: Mutex<Vec<SyncSender<TerminalEvent>>>,
    notification_pending: AtomicBool,
}

#[derive(Default)]
struct TerminalProtocolHandler {
    csi_candidate: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct DsrReplyStats {
    requests_seen: usize,
    replies_sent: usize,
}

impl TerminalProtocolHandler {
    fn handle<F, G>(
        &mut self,
        bytes: &[u8],
        mut cursor_position: F,
        mut enqueue_reply: G,
    ) -> DsrReplyStats
    where
        F: FnMut() -> (u16, u16),
        G: FnMut(Vec<u8>) -> bool,
    {
        const DSR_CURSOR_POSITION: &[u8] = b"\x1b[6n";
        let mut stats = DsrReplyStats::default();
        for &byte in bytes {
            if self.csi_candidate.is_empty() {
                if byte == 0x1b {
                    self.csi_candidate.push(byte);
                }
                continue;
            }
            self.csi_candidate.push(byte);
            if self.csi_candidate == DSR_CURSOR_POSITION {
                stats.requests_seen += 1;
                if terminal_debug_enabled() {
                    eprintln!("terminal_dsr_cursor_position_requested");
                }
                let (row, column) = cursor_position();
                let reply = format!("\x1b[{};{}R", row + 1, column + 1).into_bytes();
                if enqueue_reply(reply) {
                    stats.replies_sent += 1;
                    if terminal_debug_enabled() {
                        eprintln!(
                            "terminal_dsr_cursor_position_replied row={} column={}",
                            row + 1,
                            column + 1
                        );
                    }
                } else if terminal_debug_enabled() {
                    eprintln!("terminal_dsr_cursor_position_reply_dropped");
                }
                self.csi_candidate.clear();
            } else if !DSR_CURSOR_POSITION.starts_with(&self.csi_candidate) {
                self.csi_candidate.clear();
                if byte == 0x1b {
                    self.csi_candidate.push(byte);
                }
            }
        }
        stats
    }
}

impl ScreenState {
    fn feed(&self, bytes: &[u8]) {
        self.parser
            .lock()
            .expect("terminal parser lock poisoned")
            .process(bytes);
        self.revision.fetch_add(1, Ordering::Release);
        self.signal_update();
    }

    fn state(&self) -> TerminalState {
        match self.state.load(Ordering::Acquire) {
            1 => TerminalState::Exited,
            2 => TerminalState::Failed,
            _ => TerminalState::Running,
        }
    }

    fn cursor_position(&self) -> (u16, u16) {
        self.parser
            .lock()
            .expect("terminal parser lock poisoned")
            .screen()
            .cursor_position()
    }

    fn signal_update(&self) {
        if self.notification_pending.swap(true, Ordering::AcqRel) {
            return;
        }
        let event = TerminalEvent {
            generation: self.revision.load(Ordering::Acquire),
        };
        if let Ok(mut notifications) = self.notifications.lock() {
            notifications.retain(|sender| match sender.try_send(event) {
                Ok(()) | Err(TrySendError::Full(_)) => true,
                Err(TrySendError::Disconnected(_)) => false,
            });
        }
    }

    fn mark_state(&self, state: TerminalState) {
        self.state.store(
            match state {
                TerminalState::Running => 0,
                TerminalState::Exited => 1,
                TerminalState::Failed => 2,
            },
            Ordering::Release,
        );
        self.exited
            .store(state == TerminalState::Exited, Ordering::Release);
        self.revision.fetch_add(1, Ordering::Release);
        self.signal_update();
    }

    fn subscribe(&self) -> Receiver<TerminalEvent> {
        let (sender, receiver) = sync_channel(1);
        let pending_event = self.notification_pending.load(Ordering::Acquire).then(|| {
            TerminalEvent {
                generation: self.revision.load(Ordering::Acquire),
            }
        });
        if let Ok(mut notifications) = self.notifications.lock() {
            notifications.push(sender.clone());
        }
        if let Some(event) = pending_event {
            let _ = sender.try_send(event);
        }
        receiver
    }
}

pub struct TerminalSession {
    profile: TerminalProfile,
    cwd: PathBuf,
    screen: Arc<ScreenState>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    input_tx: SyncSender<Vec<u8>>,
    child: Mutex<Option<Box<dyn Child + Send + Sync>>>,
    size: Mutex<(u16, u16)>,
}

impl TerminalSession {
    pub fn spawn(cwd: impl AsRef<Path>, profile: TerminalProfile) -> io::Result<Self> {
        tracing::info!("terminal_spawn_started");
        if terminal_debug_enabled() {
            eprintln!("terminal_session_created");
        }
        let cwd = cwd.as_ref().to_path_buf();
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: DEFAULT_ROWS,
                cols: DEFAULT_COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(io::Error::other)?;
        if terminal_debug_enabled() {
            eprintln!("terminal_pty_pair_created");
        }
        let mut command = profile.command();
        #[cfg(windows)]
        if matches!(&profile, TerminalProfile::Wsl | TerminalProfile::Ubuntu) {
            if let Some(wsl_cwd) = windows_path_to_wsl(&cwd) {
                command.arg("--cd");
                command.arg(wsl_cwd);
            }
        } else {
            command.cwd(&cwd);
        }
        #[cfg(not(windows))]
        command.cwd(&cwd);
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(io::Error::other)?;
        if terminal_debug_enabled() {
            eprintln!("terminal_child_spawned");
        }
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().map_err(io::Error::other)?;
        if terminal_debug_enabled() {
            eprintln!("terminal_master_reader_ready");
        }
        let writer = pair.master.take_writer().map_err(io::Error::other)?;
        if terminal_debug_enabled() {
            eprintln!("terminal_master_writer_ready");
        }
        let screen = Arc::new(ScreenState {
            parser: Mutex::new(vt100::Parser::new(
                DEFAULT_ROWS,
                DEFAULT_COLS,
                DEFAULT_SCROLLBACK,
            )),
            revision: AtomicU64::new(0),
            exited: AtomicBool::new(false),
            state: AtomicU64::new(0),
            notifications: Mutex::new(Vec::new()),
            notification_pending: AtomicBool::new(false),
        });
        let (input_tx, input_rx) = sync_channel(INPUT_QUEUE_CAPACITY);
        let protocol_tx = input_tx.clone();
        let writer_screen = screen.clone();
        thread::Builder::new()
            .name("axiom-terminal-writer".to_owned())
            .spawn(move || {
                tracing::info!("terminal_writer_started");
                writer_loop(writer, input_rx, writer_screen);
            })?;
        let reader_screen = screen.clone();
        thread::Builder::new()
            .name("axiom-terminal-reader".to_owned())
            .spawn(move || {
                tracing::info!("terminal_reader_started");
                let mut buffer = [0_u8; 8192];
                let mut protocol_handler = TerminalProtocolHandler::default();
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) => break,
                        Err(_) => {
                            reader_screen.mark_state(TerminalState::Failed);
                            if terminal_debug_enabled() {
                                eprintln!("terminal_reader_stopped reason=ReadError");
                            }
                            break;
                        }
                        Ok(read) => {
                            reader_screen.feed(&buffer[..read]);
                            protocol_handler.handle(
                                &buffer[..read],
                                || reader_screen.cursor_position(),
                                |reply| protocol_tx.try_send(reply).is_ok(),
                            );
                            reader_screen.signal_update();
                        }
                    }
                }
                if reader_screen.state() == TerminalState::Running {
                    reader_screen.mark_state(TerminalState::Exited);
                    tracing::info!("terminal_child_exited");
                    if terminal_debug_enabled() {
                        eprintln!("terminal_reader_stopped reason=EOF");
                    }
                }
            })?;
        tracing::info!("terminal_spawn_succeeded");
        Ok(Self {
            profile,
            cwd,
            screen,
            master: Mutex::new(pair.master),
            input_tx,
            child: Mutex::new(Some(child)),
            size: Mutex::new((DEFAULT_ROWS, DEFAULT_COLS)),
        })
    }

    pub fn write(&self, bytes: &[u8]) -> io::Result<()> {
        if bytes.len() > MAX_INPUT_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "terminal input exceeds the bounded write size",
            ));
        }
        self.input_tx.try_send(bytes.to_vec()).map_err(|error| match error {
            TrySendError::Full(_) => {
                io::Error::new(io::ErrorKind::WouldBlock, "terminal input queue is full")
            }
            TrySendError::Disconnected(_) => {
                io::Error::new(io::ErrorKind::BrokenPipe, "terminal writer stopped")
            }
        })
    }

    /// Clears the visible VT screen without terminating or recreating the PTY.
    pub fn clear_screen(&self) {
        if let Ok(mut parser) = self.screen.parser.lock() {
            parser.process(b"\x1b[2J\x1b[H");
            self.screen.revision.fetch_add(1, Ordering::Relaxed);
            self.screen.signal_update();
        }
    }

    pub fn resize(&self, rows: u16, cols: u16) -> io::Result<()> {
        let (rows, cols) = clamp_terminal_size(rows, cols);
        let mut size = self.size.lock().expect("terminal size lock poisoned");
        if *size == (rows, cols) {
            return Ok(());
        }
        self.master
            .lock()
            .expect("terminal master lock poisoned")
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(io::Error::other)?;
        self.screen
            .parser
            .lock()
            .expect("terminal parser lock poisoned")
            .screen_mut()
            .set_size(rows, cols);
        self.screen.revision.fetch_add(1, Ordering::Release);
        self.screen.signal_update();
        *size = (rows, cols);
        Ok(())
    }

    pub fn contents(&self) -> String {
        self.screen
            .parser
            .lock()
            .expect("terminal parser lock poisoned")
            .screen()
            .contents()
    }

    pub fn revision(&self) -> u64 {
        self.screen.revision.load(Ordering::Acquire)
    }

    pub fn is_exited(&self) -> bool {
        self.screen.exited.load(Ordering::Acquire)
    }

    pub fn state(&self) -> TerminalState {
        self.screen.state()
    }

    pub fn subscribe(&self) -> Receiver<TerminalEvent> {
        self.screen.subscribe()
    }

    pub fn acknowledge_event(&self, observed_generation: u64) {
        self.screen.notification_pending.store(false, Ordering::Release);
        if self.revision() != observed_generation {
            self.screen.signal_update();
        }
    }

    pub fn profile_label(&self) -> &str {
        self.profile.label()
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    pub fn terminate(&self) -> io::Result<()> {
        if let Some(child) = self
            .child
            .lock()
            .expect("terminal child lock poisoned")
            .as_mut()
        {
            child.kill().map_err(io::Error::other)?;
        }
        self.screen.mark_state(TerminalState::Exited);
        tracing::info!("terminal_session_closed");
        Ok(())
    }
}

fn writer_loop(
    mut writer: Box<dyn Write + Send>,
    input_rx: Receiver<Vec<u8>>,
    screen: Arc<ScreenState>,
) {
    while let Ok(bytes) = input_rx.recv() {
        if writer.write_all(&bytes).and_then(|_| writer.flush()).is_err() {
            screen.mark_state(TerminalState::Failed);
            return;
        }
    }
}

fn terminal_debug_enabled() -> bool {
    std::env::var_os("AXIOM_DEBUG_TERMINAL").is_some_and(|value| {
        !matches!(value.to_string_lossy().as_ref(), "" | "0" | "false" | "off")
    })
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = self.terminate();
        let _ = self.child.get_mut().ok().and_then(Option::take);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Result as IoResult;
    #[cfg(unix)]
    use std::time::{Duration, Instant};
    #[cfg(windows)]
    use std::time::{Duration, Instant};

    #[test]
    fn default_profile_has_a_real_program() {
        match TerminalProfile::platform_default() {
            TerminalProfile::Unix { program } => assert!(!program.as_os_str().is_empty()),
            TerminalProfile::PowerShell { program } => assert!(!program.is_empty()),
            _ => {}
        }
    }

    #[test]
    fn detects_file_locations_urls_and_relative_cwd() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("src")).unwrap();
        std::fs::write(root.path().join("src/Foo.php"), "<?php\n").unwrap();
        let links = detect_links(
            "src/Foo.php:42:7 https://example.com missing.php:2",
            root.path(),
        );
        assert_eq!(links.len(), 2);
        assert!(matches!(
            links[0].kind,
            TerminalLinkKind::FileLine {
                line: 42,
                column: Some(7)
            }
        ));
        assert_eq!(
            links[0].path.as_deref(),
            Some(root.path().join("src/Foo.php").as_path())
        );
        assert_eq!(links[1].kind, TerminalLinkKind::Url);
    }

    #[test]
    fn windows_drive_colon_is_not_treated_as_a_separator() {
        let (path, line, column) = split_location(r"C:\Project\src\Foo.php:20:5");
        assert_eq!(path, r"C:\Project\src\Foo.php");
        assert_eq!((line, column), (Some(20), Some(5)));
    }

    #[test]
    fn input_queue_preserves_order_and_is_bounded() {
        let (sender, receiver) = sync_channel(INPUT_QUEUE_CAPACITY);
        let output = Arc::new(Mutex::new(Vec::new()));
        let writer = RecordingWriter(output.clone());
        let screen = test_screen();
        for value in 0..INPUT_QUEUE_CAPACITY {
            sender.send(vec![value as u8]).unwrap();
        }
        assert!(sender.try_send(vec![255]).is_err());
        drop(sender);
        let worker = thread::spawn(move || writer_loop(Box::new(writer), receiver, screen));
        worker.join().unwrap();
        assert_eq!(
            *output.lock().unwrap(),
            (0..INPUT_QUEUE_CAPACITY).map(|value| value as u8).collect::<Vec<_>>()
        );
    }

    #[test]
    fn output_event_generation_is_coalesced_and_acknowledged() {
        let screen = test_screen();
        let (sender, receiver) = sync_channel(1);
        screen.notifications.lock().unwrap().push(sender);
        screen.revision.fetch_add(1, Ordering::Release);
        screen.signal_update();
        screen.revision.fetch_add(1, Ordering::Release);
        screen.signal_update();
        let event = receiver.try_recv().unwrap();
        assert_eq!(event.generation, 1);
        screen.notification_pending.store(false, Ordering::Release);
        screen.signal_update();
        assert_eq!(receiver.try_recv().unwrap().generation, 2);
    }

    #[test]
    fn late_subscriber_receives_pending_output_generation() {
        let screen = test_screen();
        screen.revision.fetch_add(1, Ordering::Release);
        screen.signal_update();
        let receiver = screen.subscribe();
        let event = receiver.try_recv().unwrap();
        assert_eq!(event.generation, 1);
    }

    #[test]
    fn resize_clamps_zero_dimensions() {
        assert_eq!(clamp_terminal_size(0, 0), (1, 1));
        assert_eq!(clamp_terminal_size(0, 80), (1, 80));
    }

    #[test]
    fn profiles_have_stable_ids_and_wsl_path_conversion() {
        assert_eq!(TerminalProfile::Cmd.id(), "cmd");
        assert_eq!(TerminalProfile::Ubuntu.id(), "ubuntu");
        assert_eq!(
            windows_path_to_wsl(r"E:\dev\Axiom\src"),
            Some("/mnt/e/dev/Axiom/src".to_owned())
        );
        assert_eq!(windows_path_to_wsl("relative/path"), None);
    }

    #[cfg(unix)]
    #[test]
    fn terminal_manager_keeps_sessions_independent_and_bounded() {
        let cwd = tempfile::tempdir().unwrap();
        let mut manager = TerminalManager::new(2);
        let profile = || TerminalProfile::Unix {
            program: PathBuf::from("/bin/sh"),
        };
        let first = manager.create_session(cwd.path(), profile()).unwrap();
        let second = manager.create_session(cwd.path(), profile()).unwrap();
        assert_ne!(first, second);
        assert_eq!(manager.active_session(), Some(second));
        assert_eq!(manager.sessions()[0].label, "sh 1");
        assert_eq!(manager.sessions()[1].label, "sh 2");
        assert!(manager.activate_session(first));
        assert_eq!(manager.active_session(), Some(first));
        assert!(manager.create_session(cwd.path(), profile()).is_err());
        assert!(manager.close_session(first));
        assert_eq!(manager.len(), 1);
        assert!(manager.session(second).is_some());
    }

    #[test]
    fn state_changes_are_observable() {
        let screen = test_screen();
        assert_eq!(screen.state(), TerminalState::Running);
        screen.mark_state(TerminalState::Exited);
        assert_eq!(screen.state(), TerminalState::Exited);
        assert!(screen.exited.load(Ordering::Acquire));
        screen.mark_state(TerminalState::Failed);
        assert_eq!(screen.state(), TerminalState::Failed);
    }

    #[test]
    fn detects_fragmented_cursor_position_queries() {
        let mut handler = TerminalProtocolHandler::default();
        let mut replies = Vec::new();
        assert_eq!(
            handler.handle(b"\x1b[", || (0, 0), |reply| {
                replies.push(reply);
                true
            }),
            DsrReplyStats::default()
        );
        assert_eq!(
            handler.handle(b"6", || (0, 0), |reply| {
                replies.push(reply);
                true
            }),
            DsrReplyStats::default()
        );
        let stats = handler.handle(
            b"n\x1b[6n\x1b[6n",
            || (0, 0),
            |reply| {
                replies.push(reply);
                true
            },
        );
        assert_eq!(
            stats,
            DsrReplyStats {
                requests_seen: 3,
                replies_sent: 3,
            }
        );
        assert_eq!(replies, vec![b"\x1b[1;1R".to_vec(); 3]);
    }

    fn test_screen() -> Arc<ScreenState> {
        Arc::new(ScreenState {
            parser: Mutex::new(vt100::Parser::new(2, 2, 10)),
            revision: AtomicU64::new(0),
            exited: AtomicBool::new(false),
            state: AtomicU64::new(0),
            notifications: Mutex::new(Vec::new()),
            notification_pending: AtomicBool::new(false),
        })
    }

    struct RecordingWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for RecordingWriter {
        fn write(&mut self, bytes: &[u8]) -> IoResult<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> IoResult<()> {
            Ok(())
        }
    }

    #[cfg(unix)]
    #[test]
    fn unix_pty_reads_deterministic_child_output() {
        let cwd = tempfile::tempdir().unwrap();
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: DEFAULT_ROWS,
                cols: DEFAULT_COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = CommandBuilder::new("/bin/sh");
        command.args(["-c", "printf 'AXIOM_UNIX_OK\\n'"]);
        command.cwd(cwd.path());
        let mut child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let _writer = pair.master.take_writer().unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        thread::spawn(move || {
            let mut output = Vec::new();
            reader.read_to_end(&mut output).unwrap();
            let _ = sender.send(output);
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while child.try_wait().unwrap().is_none() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(child.try_wait().unwrap().is_some(), "Unix child timed out");
        let output = receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("Unix PTY reader timed out");
        assert!(
            String::from_utf8_lossy(&output).contains("AXIOM_UNIX_OK"),
            "unexpected Unix PTY output: {output:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn pty_runs_in_requested_cwd_accepts_input_and_resizes() {
        let cwd = tempfile::tempdir().unwrap();
        let session = TerminalSession::spawn(
            cwd.path(),
            TerminalProfile::Unix {
                program: PathBuf::from("/bin/sh"),
            },
        )
        .unwrap();
        session.resize(30, 100).unwrap();
        session
            .write(b"printf 'RUSTSTORM_PTY_OK\\n'; pwd\n")
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline && !session.contents().contains("RUSTSTORM_PTY_OK") {
            thread::sleep(Duration::from_millis(20));
        }
        let contents = session.contents();
        assert!(contents.contains("RUSTSTORM_PTY_OK"), "{contents:?}");
        assert!(
            contents.contains(cwd.path().to_str().unwrap()),
            "{contents:?}"
        );
        session.write(b"exit\n").unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn unix_pty_output_notifies_without_follow_up_input() {
        let cwd = tempfile::tempdir().unwrap();
        let session = TerminalSession::spawn(
            cwd.path(),
            TerminalProfile::Unix {
                program: PathBuf::from("/bin/sh"),
            },
        )
        .unwrap();
        let events = session.subscribe();
        session
            .write(b"sleep 0.1; printf 'AXIOM_UNIX_EVENT_OK\\n'\n")
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut observed = false;
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match events.recv_timeout(remaining.min(Duration::from_millis(250))) {
                Ok(event) => {
                    session.acknowledge_event(event.generation);
                    if session.contents().contains("AXIOM_UNIX_EVENT_OK") {
                        observed = true;
                        break;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        assert!(observed, "Unix PTY output did not arrive through event path");
        session.write(b"exit\n").unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn cmd_deterministic_child_output() {
        let result = run_windows_probe(
            "cmd.exe",
            &["/D", "/S", "/C", "echo AXIOM_CMD_OK"],
            None,
            "AXIOM_CMD_OK",
        );
        assert_probe_succeeded("CMD deterministic", "AXIOM_CMD_OK", result);
    }

    #[cfg(windows)]
    #[test]
    fn cmd_interactive_round_trip() {
        let result = run_windows_probe(
            "cmd.exe",
            &["/D", "/Q"],
            Some(b"echo AXIOM_CMD_INPUT_OK\r\n"),
            "AXIOM_CMD_INPUT_OK",
        );
        assert_probe_succeeded("CMD interactive", "AXIOM_CMD_INPUT_OK", result);
    }

    #[cfg(windows)]
    #[test]
    fn powershell_deterministic_child_output() {
        let result = run_windows_probe(
            "powershell.exe",
            &[
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Write-Output 'AXIOM_PS_OK'",
            ],
            None,
            "AXIOM_PS_OK",
        );
        assert_probe_succeeded("PowerShell deterministic", "AXIOM_PS_OK", result);
    }

    #[cfg(windows)]
    #[test]
    fn powershell_interactive_round_trip() {
        let result = run_windows_probe(
            "powershell.exe",
            &["-NoLogo", "-NoProfile", "-NoExit"],
            Some(b"echo AXIOM_PS_INPUT_OK\r\n"),
            "AXIOM_PS_INPUT_OK",
        );
        assert_probe_succeeded("PowerShell interactive", "AXIOM_PS_INPUT_OK", result);
    }

    #[cfg(windows)]
    struct WindowsProbeResult {
        output: Vec<u8>,
        first_output_ms: Option<u128>,
        total_bytes: usize,
        output_before_teardown: bool,
        dsr_requests_seen: usize,
        dsr_replies_sent: usize,
    }

    #[cfg(windows)]
    fn run_windows_probe(
        program: &str,
        args: &[&str],
        input: Option<&[u8]>,
        expected: &str,
    ) -> Result<WindowsProbeResult, String> {
        let started = Instant::now();
        let cwd = tempfile::tempdir().map_err(|error| error.to_string())?;
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: DEFAULT_ROWS,
                cols: DEFAULT_COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| error.to_string())?;
        let mut command = CommandBuilder::new(program);
        command.args(args.iter().copied());
        command.cwd(cwd.path());
        let mut child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| error.to_string())?;
        drop(pair.slave);
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| error.to_string())?;
        let writer = pair.master.take_writer().map_err(|error| error.to_string())?;
        let writer_screen = test_screen();
        let (input_tx, input_rx) = sync_channel(INPUT_QUEUE_CAPACITY);
        let writer_worker_screen = writer_screen.clone();
        thread::spawn(move || writer_loop(writer, input_rx, writer_worker_screen));
        if let Some(input) = input {
            input_tx
                .try_send(input.to_vec())
                .map_err(|error| format!("failed to queue probe input: {error}"))?;
        }
        let protocol_tx = input_tx.clone();
        let (sender, receiver) =
            std::sync::mpsc::channel::<(Vec<u8>, u128, DsrReplyStats)>();
        thread::spawn(move || {
            let mut buffer = [0_u8; 4096];
            let mut parser = vt100::Parser::new(DEFAULT_ROWS, DEFAULT_COLS, DEFAULT_SCROLLBACK);
            let mut protocol_handler = TerminalProtocolHandler::default();
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => {
                        parser.process(&buffer[..count]);
                        let stats = protocol_handler.handle(
                            &buffer[..count],
                            || parser.screen().cursor_position(),
                            |reply| protocol_tx.try_send(reply).is_ok(),
                        );
                        if sender
                            .send((
                                buffer[..count].to_vec(),
                                started.elapsed().as_millis(),
                                stats,
                            ))
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
        });

        let timeout = Duration::from_secs(5);
        let mut output = Vec::new();
        let mut first_output_ms = None;
        let mut dsr_requests_seen = 0;
        let mut dsr_replies_sent = 0;
        while started.elapsed() < timeout {
            match receiver.recv_timeout(Duration::from_millis(100)) {
                Ok((bytes, elapsed_ms, stats)) => {
                    first_output_ms.get_or_insert(elapsed_ms);
                    output.extend_from_slice(&bytes);
                    dsr_requests_seen += stats.requests_seen;
                    dsr_replies_sent += stats.replies_sent;
                    if String::from_utf8_lossy(&output).contains(expected) {
                        let output_before_teardown = true;
                        if child.try_wait().map_err(|error| error.to_string())?.is_none() {
                            let _ = child.kill();
                        }
                        return Ok(WindowsProbeResult {
                            total_bytes: output.len(),
                            output,
                            first_output_ms,
                            output_before_teardown,
                            dsr_requests_seen,
                            dsr_replies_sent,
                        });
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        let child_running = child
            .try_wait()
            .map_err(|error| error.to_string())?
            .is_none();
        let elapsed_ms = started.elapsed().as_millis();
        if child_running {
            let _ = child.kill();
        }
        Err(format!(
            "timeout elapsed_ms={elapsed_ms} total_bytes={} first_output_ms={first_output_ms:?} child_running={child_running} dsr_requests_seen={dsr_requests_seen} dsr_replies_sent={dsr_replies_sent} output={:?}",
            output.len(),
            String::from_utf8_lossy(&output)
        ))
    }

    #[cfg(windows)]
    fn assert_probe_succeeded(
        label: &str,
        expected: &str,
        result: Result<WindowsProbeResult, String>,
    ) {
        let result = result.unwrap_or_else(|error| panic!("{label} probe failed: {error}"));
        assert!(result.total_bytes > 0, "{label} returned no bytes");
        assert!(
            result.output_before_teardown,
            "{label} output only appeared during teardown"
        );
        assert!(
            result.dsr_requests_seen >= 1,
            "{label} observed no DSR request"
        );
        assert!(
            result.dsr_replies_sent >= 1,
            "{label} sent no DSR reply"
        );
        assert!(
            String::from_utf8_lossy(&result.output).contains(expected),
            "{label} output={:?} first_output_ms={:?}",
            String::from_utf8_lossy(&result.output),
            result.first_output_ms
        );
    }
}
