use std::{
    io::{self, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde_json::json;

pub(crate) const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_CAPTURE_BYTES: u64 = 64 * 1024;
pub(crate) const MAX_COMMAND_OUTPUT_CHARS: usize = 4000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CommandProfile {
    Default,
    PowerShell,
    Cmd,
    Wsl,
    Ubuntu,
}

impl CommandProfile {
    fn parse(value: Option<&str>) -> Result<Self, String> {
        match value.map(str::trim).filter(|value| !value.is_empty()) {
            None => Ok(Self::Default),
            Some(value) if value.eq_ignore_ascii_case("powershell") => Ok(Self::PowerShell),
            Some(value) if value.eq_ignore_ascii_case("cmd") => Ok(Self::Cmd),
            Some(value) if value.eq_ignore_ascii_case("wsl") => Ok(Self::Wsl),
            Some(value) if value.eq_ignore_ascii_case("ubuntu") => Ok(Self::Ubuntu),
            Some(value) => Err(format!(
                "unsupported terminal profile: {value}; expected PowerShell, CMD, WSL, or Ubuntu"
            )),
        }
    }
}

pub(crate) fn execute(
    workspace_root: &Path,
    command: String,
    cwd: Option<String>,
    profile: Option<String>,
) -> Result<String, ExecuteCommandError> {
    execute_with_timeout(workspace_root, command, cwd, profile, COMMAND_TIMEOUT, None)
}

pub(crate) fn execute_with_sink(
    workspace_root: &Path,
    command: String,
    cwd: Option<String>,
    profile: Option<String>,
    sink: std::sync::Arc<dyn Fn(bool, String) + Send + Sync>,
) -> Result<String, ExecuteCommandError> {
    execute_with_timeout(
        workspace_root,
        command,
        cwd,
        profile,
        COMMAND_TIMEOUT,
        Some(sink),
    )
}

fn execute_with_timeout(
    workspace_root: &Path,
    command: String,
    cwd: Option<String>,
    profile: Option<String>,
    timeout: Duration,
    sink: Option<std::sync::Arc<dyn Fn(bool, String) + Send + Sync>>,
) -> Result<String, ExecuteCommandError> {
    let command = command.trim().to_owned();
    if command.is_empty() {
        return Err(ExecuteCommandError::Invalid("command must not be empty".into()));
    }
    let profile = CommandProfile::parse(profile.as_deref())
        .map_err(ExecuteCommandError::Invalid)?;
    let cwd = resolve_cwd(workspace_root, cwd.as_deref())?;
    let mut process = build_command(profile, &command, &cwd);
    process.current_dir(&cwd).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = process
        .spawn()
        .map_err(|error| ExecuteCommandError::Spawn(error.to_string()))?;
    let stdout = read_pipe(child.stdout.take(), false, sink.clone());
    let stderr = read_pipe(child.stderr.take(), true, sink);
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ExecuteCommandError::Timeout);
            }
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ExecuteCommandError::Wait(error.to_string()));
            }
        }
    };
    let stdout = stdout
        .join()
        .map_err(|_| ExecuteCommandError::Read("stdout reader panicked".into()))?
        .map_err(|error| ExecuteCommandError::Read(error.to_string()))?;
    let stderr = stderr
        .join()
        .map_err(|_| ExecuteCommandError::Read("stderr reader panicked".into()))?
        .map_err(|error| ExecuteCommandError::Read(error.to_string()))?;
    let stdout = String::from_utf8_lossy(&stdout).into_owned();
    let stderr = String::from_utf8_lossy(&stderr).into_owned();
    let stdout = truncate(&stdout);
    let stderr = truncate_to(&stderr, MAX_COMMAND_OUTPUT_CHARS.saturating_sub(stdout.chars().count()));
    Ok(json!({
        "exit_code": status.code(),
        "success": status.success(),
        "stdout": stdout,
        "stderr": stderr,
        "cwd": cwd.to_string_lossy(),
    })
    .to_string())
}

fn resolve_cwd(workspace_root: &Path, requested: Option<&str>) -> Result<PathBuf, ExecuteCommandError> {
    let root = workspace_root
        .canonicalize()
        .map_err(|error| ExecuteCommandError::Cwd(error.to_string()))?;
    let candidate = match requested.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => {
            let path = PathBuf::from(value);
            if path.is_absolute() {
                path
            } else {
                root.join(path)
            }
        }
        None => root.clone(),
    };
    let cwd = candidate
        .canonicalize()
        .map_err(|error| ExecuteCommandError::Cwd(error.to_string()))?;
    if !cwd.starts_with(&root) || !cwd.is_dir() {
        return Err(ExecuteCommandError::Cwd(
            "working directory must be a directory inside the workspace".into(),
        ));
    }
    Ok(cwd)
}

fn build_command(profile: CommandProfile, command: &str, cwd: &Path) -> Command {
    let process = if cfg!(windows) {
        match profile {
            CommandProfile::PowerShell | CommandProfile::Default => {
                let mut process = Command::new("powershell.exe");
                process.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", command]);
                process
            }
            CommandProfile::Cmd => {
                let mut process = Command::new("cmd.exe");
                process.args(["/D", "/C", command]);
                process
            }
            CommandProfile::Wsl | CommandProfile::Ubuntu => {
                let mut process = Command::new("wsl.exe");
                if matches!(profile, CommandProfile::Ubuntu) {
                    process.args(["-d", "Ubuntu"]);
                }
                process.args(["--cd", &cwd.to_string_lossy(), "--", "sh", "-lc", command]);
                process
            }
        }
    } else {
        let mut process = Command::new("sh");
        process.args(["-lc", command]);
        process
    };
    process
}

fn read_pipe<T>(
    pipe: Option<T>,
    stderr: bool,
    sink: Option<std::sync::Arc<dyn Fn(bool, String) + Send + Sync>>,
) -> thread::JoinHandle<io::Result<Vec<u8>>>
where
    T: Read + Send + 'static,
{
    thread::spawn(move || {
        let Some(pipe) = pipe else { return Ok(Vec::new()) };
        let mut bytes = Vec::new();
        let mut pipe = pipe.take(MAX_CAPTURE_BYTES);
        let mut buffer = [0_u8; 4096];
        loop {
            let count = pipe.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..count]);
            if let Some(sink) = &sink {
                sink(stderr, String::from_utf8_lossy(&buffer[..count]).into_owned());
            }
        }
        Ok(bytes)
    })
}

fn truncate(value: &str) -> String {
    truncate_to(value, MAX_COMMAND_OUTPUT_CHARS)
}

fn truncate_to(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ExecuteCommandError {
    Invalid(String),
    Cwd(String),
    Spawn(String),
    Wait(String),
    Read(String),
    Timeout,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_is_bounded_deterministically() {
        assert_eq!(truncate("abcdef"), "abcdef");
        assert_eq!(truncate(&"x".repeat(MAX_COMMAND_OUTPUT_CHARS + 10)).len(), MAX_COMMAND_OUTPUT_CHARS);
    }

    #[test]
    fn cwd_is_restricted_to_workspace() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        assert!(resolve_cwd(workspace.path(), Some("." )).is_ok());
        assert!(matches!(
            resolve_cwd(workspace.path(), Some(outside.path().to_str().unwrap())),
            Err(ExecuteCommandError::Cwd(_))
        ));
    }

    #[test]
    fn command_timeout_terminates_the_process() {
        let workspace = tempfile::tempdir().unwrap();
        let (command, profile) = if cfg!(windows) {
            ("Start-Sleep -Seconds 2".into(), Some("PowerShell".into()))
        } else {
            ("sleep 2".into(), None)
        };
        assert_eq!(
            execute_with_timeout(
                workspace.path(),
                command,
                None,
                profile,
                Duration::from_millis(50),
                None,
            ),
            Err(ExecuteCommandError::Timeout)
        );
    }

    #[cfg(windows)]
    #[test]
    fn powershell_captures_stdout() {
        let workspace = tempfile::tempdir().unwrap();
        let result = execute(workspace.path(), "Write-Output hello".into(), None, Some("PowerShell".into())).unwrap();
        assert!(result.contains("hello"));
    }

    #[cfg(unix)]
    #[test]
    fn default_shell_captures_stdout() {
        let workspace = tempfile::tempdir().unwrap();
        let result = execute(workspace.path(), "printf hello".into(), None, None).unwrap();
        assert!(result.contains("hello"));
    }
}
