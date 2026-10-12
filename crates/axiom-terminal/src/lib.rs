//! Single-session terminal bridge backed by `alacritty_terminal`.

use std::{io, path::{Path, PathBuf}, process::Command, sync::{Arc, Mutex, mpsc::{self, Receiver, SyncSender}, atomic::{AtomicBool, Ordering}}};
use alacritty_terminal::{event::{Event, EventListener, WindowSize}, event_loop::{EventLoop, EventLoopSender, Msg}, grid::{Dimensions, Scroll}, sync::FairMutex, term::{Config, Term}, term::cell::Flags, tty::{self, Options, Shell}, vte::ansi::Color};
pub use alacritty_terminal::term::cell::Flags as CellFlags;
pub use alacritty_terminal::vte::ansi::{Color as TerminalColor, NamedColor, Rgb};

pub use alacritty_terminal::vte::ansi::CursorShape;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TerminalProfileId {
    PowerShell,
    CommandPrompt,
    WslDefault,
    WslDistro(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalProfile {
    pub id: TerminalProfileId,
    pub label: String,
    pub program: PathBuf,
    pub args: Vec<String>,
}

impl TerminalProfile {
    pub fn powershell() -> Self { Self { id: TerminalProfileId::PowerShell, label: "PowerShell".into(), program: "powershell.exe".into(), args: vec!["-NoLogo".into(), "-NoProfile".into()] } }
    pub fn command_prompt() -> Self { Self { id: TerminalProfileId::CommandPrompt, label: "Command Prompt".into(), program: "cmd.exe".into(), args: vec![] } }
    pub fn wsl_default(cwd: Option<&Path>) -> Self { Self::wsl(TerminalProfileId::WslDefault, "WSL", None, cwd) }
    pub fn wsl_distro(name: impl Into<String>, cwd: Option<&Path>) -> Self { let name = name.into(); Self::wsl(TerminalProfileId::WslDistro(name.clone()), name.clone(), Some(name), cwd) }
    fn wsl(id: TerminalProfileId, label: impl Into<String>, distro: Option<String>, cwd: Option<&Path>) -> Self {
        let mut args = Vec::new();
        if let Some(distro) = distro { args.extend(["-d".into(), distro]); }
        if let Some(cwd) = cwd.and_then(windows_path_to_wsl) { args.extend(["--cd".into(), cwd]); }
        Self { id, label: label.into(), program: "wsl.exe".into(), args }
    }
    pub fn default() -> Self {
        if cfg!(windows) { Self::powershell() } else { Self { id: TerminalProfileId::PowerShell, label: "Shell".into(), program: std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()).into(), args: vec![] } }
    }
}

pub fn windows_path_to_wsl(path: &Path) -> Option<String> {
    windows_path_str_to_wsl(&path.to_string_lossy())
}

pub fn normalize_windows_shell_path(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    let rest = text.strip_prefix(r"\\?\");
    if let Some(rest) = rest && rest.len() >= 3 && rest.as_bytes()[1] == b':' && rest.as_bytes()[2] == b'\\' {
        return PathBuf::from(rest);
    }
    path.to_path_buf()
}

pub fn windows_path_str_to_wsl(path: &str) -> Option<String> {
    let path = path.strip_prefix(r"\\?\").filter(|rest| rest.len() >= 3 && rest.as_bytes()[1] == b':' && rest.as_bytes()[2] == b'\\').unwrap_or(path);
    let bytes = path.as_bytes();
    if bytes.len() < 3 || bytes[1] != b':' || !bytes[0].is_ascii_alphabetic() || !matches!(bytes[2], b'\\' | b'/') { return None; }
    let rest = path[3..].replace('\\', "/");
    let drive = (bytes[0] as char).to_ascii_lowercase();
    if rest.is_empty() { Some(format!("/mnt/{drive}")) } else { Some(format!("/mnt/{drive}/{rest}")) }
}

pub fn parse_wsl_distro_output(bytes: &[u8]) -> Vec<String> {
    let text = if bytes.len() >= 2 && bytes.chunks_exact(2).all(|pair| pair[1] == 0) {
        String::from_utf16_lossy(&bytes.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect::<Vec<_>>())
    } else { String::from_utf8_lossy(bytes).into_owned() };
    let mut result = Vec::new();
    for line in text.lines().map(|line| line.trim().trim_start_matches('\u{feff}')).filter(|line| !line.is_empty()) {
        if !result.iter().any(|existing| existing == line) { result.push(line.to_owned()); }
    }
    result
}

pub fn discover_wsl_distros() -> io::Result<Vec<String>> {
    if !cfg!(windows) { return Ok(Vec::new()); }
    let output = Command::new("wsl.exe").args(["--list", "--quiet"]).output()?;
    if !output.status.success() { return Ok(Vec::new()); }
    Ok(parse_wsl_distro_output(&output.stdout))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalSize { pub cols: u16, pub rows: u16 }
impl TerminalSize { pub fn new(cols: u16, rows: u16) -> Self { Self { cols: cols.max(1), rows: rows.max(1) } } }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalState { Running, Exited, Failed }

pub fn encode_key(key: &str, ctrl: bool, application_cursor: bool) -> Option<Vec<u8>> {
    if ctrl && key.len() == 1 { let byte = key.as_bytes()[0].to_ascii_lowercase(); return (byte.is_ascii_lowercase()).then_some(vec![byte - b'a' + 1]); }
    let code = match key.to_ascii_lowercase().as_str() {
        "enter" | "return" => "\r", "backspace" => "\x7f", "tab" => "\t", "space" => " ", "escape" | "esc" => "\x1b",
        "up" | "arrowup" => if application_cursor { "\x1bOA" } else { "\x1b[A" },
        "down" | "arrowdown" => if application_cursor { "\x1bOB" } else { "\x1b[B" },
        "right" | "arrowright" => if application_cursor { "\x1bOC" } else { "\x1b[C" },
        "left" | "arrowleft" => if application_cursor { "\x1bOD" } else { "\x1b[D" },
        "home" => "\x1b[H", "end" => "\x1b[F", "delete" => "\x1b[3~", "pageup" => "\x1b[5~", "pagedown" => "\x1b[6~", _ => return None
    }; Some(code.as_bytes().to_vec())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalCell { pub character: char, pub foreground: Color, pub background: Color, pub flags: Flags }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSnapshot { pub cells: Vec<Vec<TerminalCell>>, pub cursor: (usize, usize), pub cursor_shape: CursorShape, pub cursor_visible: bool, pub display_offset: usize, pub size: TerminalSize, pub state: TerminalState }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalScrollState { pub display_offset: usize, pub max_offset: usize, pub visible_rows: usize }

struct Listener { tx: SyncSender<()>, exited: Arc<AtomicBool> }
impl EventListener for Listener {
    fn send_event(&self, event: Event) { if matches!(event, Event::ChildExit(_)) { self.exited.store(true, Ordering::Release); } let _ = self.tx.try_send(()); }
}
struct Size { cols: usize, rows: usize }
impl Dimensions for Size { fn total_lines(&self) -> usize { self.rows } fn screen_lines(&self) -> usize { self.rows } fn columns(&self) -> usize { self.cols } }

pub struct TerminalSession { terminal: Arc<FairMutex<Term<Listener>>>, sender: EventLoopSender, child: Option<std::thread::JoinHandle<(EventLoop<tty::Pty, Listener>, alacritty_terminal::event_loop::State)>>, events: Arc<Mutex<Receiver<()>>>, state: Arc<AtomicBool>, size: TerminalSize, feed_output_previous_cr: bool }

impl TerminalSession {
    pub fn spawn(profile: &TerminalProfile, cwd: impl AsRef<Path>) -> io::Result<Self> {
        let size = TerminalSize::new(80, 24); let (tx, events) = mpsc::sync_channel(16); let exited = Arc::new(AtomicBool::new(false));
        let listener = Listener { tx: tx.clone(), exited: exited.clone() }; let terminal = Arc::new(FairMutex::new(Term::new(Config::default(), &Size { cols: size.cols as usize, rows: size.rows as usize }, listener)));
        let mut options = Options { working_directory: Some(normalize_windows_shell_path(cwd.as_ref())), ..Options::default() };
        #[cfg(windows)] { options.shell = Some(Shell::new(profile.program.to_string_lossy().into_owned(), profile.args.clone())); }
        #[cfg(not(windows))] { let _ = profile; options.shell = Some(Shell::new(std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()), vec![])); }
        tty::setup_env(); let pty = tty::new(&options, WindowSize { num_lines: size.rows, num_cols: size.cols, cell_width: 0, cell_height: 0 }, 0).map_err(io::Error::other)?;
        let event_loop = EventLoop::new(terminal.clone(), Listener { tx, exited: exited.clone() }, pty, options.drain_on_exit, false).map_err(io::Error::other)?;
        let sender = event_loop.channel(); let child = Some(event_loop.spawn());
        Ok(Self { terminal, sender, child, events: Arc::new(Mutex::new(events)), state: exited, size, feed_output_previous_cr: false })
    }
    pub fn write(&self, bytes: &[u8]) -> io::Result<()> { self.sender.send(Msg::Input(bytes.to_vec().into())).map_err(io::Error::other) }
    pub fn feed_output(&mut self, bytes: &[u8]) {
        let bytes = normalize_terminal_newlines(bytes, &mut self.feed_output_previous_cr);
        let mut processor = alacritty_terminal::vte::ansi::Processor::<
            alacritty_terminal::vte::ansi::StdSyncHandler,
        >::new();
        processor.advance(&mut *self.terminal.lock(), &bytes);
    }
    pub fn bracketed_paste_enabled(&self) -> bool { self.terminal.lock().mode().contains(alacritty_terminal::term::TermMode::BRACKETED_PASTE) }
    pub fn application_cursor_enabled(&self) -> bool { self.terminal.lock().mode().contains(alacritty_terminal::term::TermMode::APP_CURSOR) }
    pub fn paste(&self, text: &str) -> io::Result<()> { let bytes = if self.bracketed_paste_enabled() { encode_bracketed_paste(text) } else { text.as_bytes().to_vec() }; if bytes.is_empty() { Ok(()) } else { self.write(&bytes) } }
    pub fn resize(&mut self, size: TerminalSize) -> io::Result<()> { if size == self.size { return Ok(()) } self.terminal.lock().resize(Size { cols: size.cols as usize, rows: size.rows as usize }); self.size = size; self.sender.send(Msg::Resize(WindowSize { num_lines: size.rows, num_cols: size.cols, cell_width: 0, cell_height: 0 })).map_err(io::Error::other) }
    pub fn scroll_lines(&self, lines: i32) { self.terminal.lock().scroll_display(Scroll::Delta(lines)); }
    pub fn scroll_to_offset(&self, target: usize) { let mut term = self.terminal.lock(); let current = term.grid().display_offset(); let max = term.grid().history_size(); let target = target.min(max); term.scroll_display(Scroll::Delta(target as i32 - current as i32)); }
    pub fn display_offset(&self) -> usize { self.terminal.lock().grid().display_offset() }
    pub fn scroll_state(&self) -> TerminalScrollState { let term = self.terminal.lock(); TerminalScrollState { display_offset: term.grid().display_offset(), max_offset: term.grid().history_size(), visible_rows: self.size.rows as usize } }
    pub fn scroll_page_up(&self) { self.terminal.lock().scroll_display(Scroll::PageUp); }
    pub fn scroll_page_down(&self) { self.terminal.lock().scroll_display(Scroll::PageDown); }
    pub fn scroll_to_bottom(&self) { self.terminal.lock().scroll_display(Scroll::Bottom); }
    pub fn snapshot(&self) -> TerminalSnapshot {
        let term = self.terminal.lock();
        let content = term.renderable_content();
        let mut cells = vec![Vec::with_capacity(self.size.cols as usize); self.size.rows as usize];
        let mut display_line = None;
        let mut first_line = true;
        let mut viewport_row = 0usize;
        for cell in content.display_iter {
            if display_line != Some(cell.point.line.0) {
                display_line = Some(cell.point.line.0);
                if !first_line { viewport_row += 1; }
                first_line = false;
            }
            if viewport_row < cells.len() {
                cells[viewport_row].push(TerminalCell { character: cell.cell.c, foreground: cell.cell.fg, background: cell.cell.bg, flags: cell.cell.flags });
            }
        }
        TerminalSnapshot { cells, cursor: (content.cursor.point.column.0, content.cursor.point.line.0.max(0) as usize), cursor_shape: content.cursor.shape, cursor_visible: content.mode.contains(alacritty_terminal::term::TermMode::SHOW_CURSOR), display_offset: content.display_offset, size: self.size, state: if self.state.load(Ordering::Acquire) { TerminalState::Exited } else { TerminalState::Running } }
    }
    pub fn try_recv_event(&self) -> bool { self.events.lock().ok().is_some_and(|events| events.try_recv().is_ok()) }
    pub fn wait_event(&self) -> bool { self.events.lock().ok().is_some_and(|events| events.recv().is_ok()) }
    pub fn event_receiver(&self) -> Arc<Mutex<Receiver<()>>> { self.events.clone() }
    pub fn shutdown(&mut self) { let _ = self.sender.send(Msg::Shutdown); if let Some(handle) = self.child.take() { let _ = handle.join(); } }
}
fn normalize_terminal_newlines(bytes: &[u8], previous_cr: &mut bool) -> Vec<u8> {
    let mut normalized = Vec::with_capacity(bytes.len());
    for &byte in bytes {
        if byte == b'\n' && !*previous_cr { normalized.push(b'\r'); }
        normalized.push(byte);
        *previous_cr = byte == b'\r';
    }
    normalized
}
fn encode_bracketed_paste(text: &str) -> Vec<u8> { let mut bytes = Vec::with_capacity(text.len() + 12); bytes.extend_from_slice(b"\x1b[200~"); bytes.extend_from_slice(text.as_bytes()); bytes.extend_from_slice(b"\x1b[201~"); bytes }
impl Drop for TerminalSession { fn drop(&mut self) { self.shutdown(); } }

#[cfg(test)]
mod newline_tests {
    use super::normalize_terminal_newlines;

    #[test]
    fn normalizes_lf_without_duplicate_cr() {
        let mut previous_cr = false;
        assert_eq!(normalize_terminal_newlines(b"a\nb\r\nc", &mut previous_cr), b"a\r\nb\r\nc");
    }

    #[test]
    fn preserves_crlf_across_chunks() {
        let mut previous_cr = false;
        assert_eq!(normalize_terminal_newlines(b"a\r", &mut previous_cr), b"a\r");
        assert_eq!(normalize_terminal_newlines(b"\nb", &mut previous_cr), b"\nb");
    }
}

#[cfg(test)]
    mod tests { use super::*; #[test] fn size_is_clamped() { assert_eq!(TerminalSize::new(0, 0), TerminalSize::new(1, 1)); } #[test] fn space_is_encoded_as_one_byte() { assert_eq!(encode_key("space", false, false), Some(vec![0x20])); } #[test] fn special_keys_use_gpui_names() { assert_eq!(encode_key("enter", false, false), Some(b"\r".to_vec())); assert_eq!(encode_key("arrowleft", false, false), Some(b"\x1b[D".to_vec())); } #[test] fn profile_args_are_explicit() { assert_eq!(TerminalProfile::powershell().args, vec!["-NoLogo", "-NoProfile"]); assert!(TerminalProfile::command_prompt().args.is_empty()); assert_eq!(TerminalProfile::wsl_distro("Ubuntu", None).args, vec!["-d", "Ubuntu"]); } #[test] fn wsl_paths_are_arguments_not_shell_text() { assert_eq!(windows_path_str_to_wsl(r"E:\dev\Axiom"), Some("/mnt/e/dev/Axiom".into())); assert_eq!(windows_path_str_to_wsl("C:/Users/Test/My Project"), Some("/mnt/c/Users/Test/My Project".into())); assert_eq!(windows_path_str_to_wsl("E:/á/项目"), Some("/mnt/e/á/项目".into())); assert_eq!(windows_path_str_to_wsl("\\\\server\\share"), None); } #[test] fn distro_output_is_normalized() { assert_eq!(parse_wsl_distro_output(b"Ubuntu\r\nUbuntu\n Debian \r\n\r\n"), vec!["Ubuntu", "Debian"]); } #[cfg(unix)] #[test] fn shell_output_reaches_engine() { let session = TerminalSession::spawn(&TerminalProfile::default(), ".").unwrap(); session.write(b"printf AXIOM_TERMINAL_OK\n").unwrap(); for _ in 0..50 { std::thread::sleep(std::time::Duration::from_millis(20)); if session.try_recv_event() && session.snapshot().cells.iter().flatten().any(|cell| cell.character == 'A') { return; } } panic!("shell output not received"); } }
