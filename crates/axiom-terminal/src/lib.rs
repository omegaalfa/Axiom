//! Cross-platform PTY session and VT screen state for Axiom.

use std::{
    env,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
};

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalProfile {
    Unix { program: PathBuf },
    PowerShell { program: String },
    Cmd,
    Wsl,
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
            Self::Cmd => "Command Prompt",
            Self::Wsl => "WSL",
        }
    }

    fn command(&self) -> CommandBuilder {
        match self {
            Self::Unix { program } => CommandBuilder::new(program),
            Self::PowerShell { program } => CommandBuilder::new(program),
            Self::Cmd => CommandBuilder::new("cmd.exe"),
            Self::Wsl => CommandBuilder::new("wsl.exe"),
        }
    }
}

struct ScreenState {
    parser: Mutex<vt100::Parser>,
    revision: AtomicU64,
    exited: AtomicBool,
}

pub struct TerminalSession {
    profile: TerminalProfile,
    cwd: PathBuf,
    screen: Arc<ScreenState>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    child: Mutex<Option<Box<dyn Child + Send + Sync>>>,
    size: Mutex<(u16, u16)>,
}

impl TerminalSession {
    pub fn spawn(cwd: impl AsRef<Path>, profile: TerminalProfile) -> io::Result<Self> {
        let cwd = cwd.as_ref().to_path_buf();
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: DEFAULT_ROWS,
                cols: DEFAULT_COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(io::Error::other)?;
        let mut command = profile.command();
        command.cwd(&cwd);
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(io::Error::other)?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().map_err(io::Error::other)?;
        let writer = pair.master.take_writer().map_err(io::Error::other)?;
        let screen = Arc::new(ScreenState {
            parser: Mutex::new(vt100::Parser::new(
                DEFAULT_ROWS,
                DEFAULT_COLS,
                DEFAULT_SCROLLBACK,
            )),
            revision: AtomicU64::new(0),
            exited: AtomicBool::new(false),
        });
        let reader_screen = screen.clone();
        thread::Builder::new()
            .name("axiom-terminal-reader".to_owned())
            .spawn(move || {
                let mut buffer = [0_u8; 8192];
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(read) => {
                            reader_screen
                                .parser
                                .lock()
                                .expect("terminal parser lock poisoned")
                                .process(&buffer[..read]);
                            reader_screen.revision.fetch_add(1, Ordering::Release);
                        }
                    }
                }
                reader_screen.exited.store(true, Ordering::Release);
                reader_screen.revision.fetch_add(1, Ordering::Release);
            })?;
        Ok(Self {
            profile,
            cwd,
            screen,
            master: Mutex::new(pair.master),
            writer: Mutex::new(writer),
            child: Mutex::new(Some(child)),
            size: Mutex::new((DEFAULT_ROWS, DEFAULT_COLS)),
        })
    }

    pub fn write(&self, bytes: &[u8]) -> io::Result<()> {
        let mut writer = self.writer.lock().expect("terminal writer lock poisoned");
        writer.write_all(bytes)?;
        writer.flush()
    }

    /// Clears the visible VT screen without terminating or recreating the PTY.
    pub fn clear_screen(&self) {
        if let Ok(mut parser) = self.screen.parser.lock() {
            parser.process(b"\x1b[2J\x1b[H");
            self.screen.revision.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn resize(&self, rows: u16, cols: u16) -> io::Result<()> {
        let rows = rows.max(1);
        let cols = cols.max(1);
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
        Ok(())
    }
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
}
