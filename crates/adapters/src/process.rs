//! Process execution has fixed limits and no shell. Errors omit arguments; captured stderr is only
//! shown through the bounded, redacted `stderr_excerpt`.
//! On Unix, the process group is this component's sole unsafe boundary.
use crate::{AdapterError, Result};
use diffz_core::provider::Cancellation;
use std::{
    collections::BTreeMap,
    ffi::OsString,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Debug, Clone)]
pub struct ProcessRequest {
    pub executable: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: Option<PathBuf>,
    pub env: BTreeMap<OsString, OsString>,
    pub env_remove: Vec<OsString>,
    pub stdin: Vec<u8>,
    pub deadline: Duration,
    pub stdout_limit: usize,
    pub stderr_limit: usize,
}
impl ProcessRequest {
    pub fn new(executable: PathBuf) -> Self {
        Self {
            executable,
            args: vec![],
            cwd: None,
            env: BTreeMap::new(),
            env_remove: vec![],
            stdin: vec![],
            deadline: Duration::from_secs(30),
            stdout_limit: 32 * 1024 * 1024,
            stderr_limit: 64 * 1024,
        }
    }
    pub fn args(mut self, args: impl IntoIterator<Item = impl Into<OsString>>) -> Self {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }
}
#[derive(Debug)]
pub struct ProcessOutput {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}
struct ChildGuard(Child);
impl ChildGuard {
    fn terminate(&mut self) {
        #[cfg(unix)]
        {
            let pid = self.0.id() as i32; // Child was started in its own group; no user-provided PID.
            // SAFETY: this child owns the group ID, and the signal passed here is valid.
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            self.terminate()
        }
    }
}
fn drain(
    mut pipe: impl Read + Send + 'static,
    cap: usize,
    overflow: Arc<AtomicBool>,
) -> mpsc::Receiver<std::io::Result<Vec<u8>>> {
    let (tx, rx) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let result = (|| {
            let mut out = Vec::new();
            let mut buf = [0; 8192];
            loop {
                let n = pipe.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                if out.len().saturating_add(n) > cap {
                    overflow.store(true, Ordering::SeqCst);
                    return Err(std::io::Error::other("output limit"));
                }
                out.extend_from_slice(&buf[..n]);
            }
            Ok(out)
        })();
        let _ = tx.send(result);
    });
    rx
}
pub struct Runner;
impl Runner {
    pub fn run(request: ProcessRequest, cancel: Cancellation) -> Result<ProcessOutput> {
        if cancel.cancelled() {
            return Err("process cancelled".into());
        }
        if !request.executable.is_absolute() {
            return Err("the executable must resolve to a trusted absolute path".into());
        }
        if request.stdin.len() > 32 * 1024 * 1024 {
            return Err("process input is larger than the permitted safety bound".into());
        }
        let mut cmd = Command::new(&request.executable);
        cmd.args(&request.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(cwd) = &request.cwd {
            cmd.current_dir(cwd);
        }
        cmd.envs(&request.env);
        for k in &request.env_remove {
            cmd.env_remove(k);
        }
        // Fixed noninteractive settings prevent reads from waiting on a pager or prompt.
        cmd.env("GIT_TERMINAL_PROMPT", "0")
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_PAGER", "cat")
            .env("PAGER", "cat")
            .env("NO_COLOR", "1");
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        let mut child = ChildGuard(cmd.spawn().map_err(|e| {
            AdapterError::Message(format!("configured executable could not start: {e}"))
        })?);
        let overflow = Arc::new(AtomicBool::new(false));
        let stdout = drain(
            child.0.stdout.take().ok_or("stdout pipe unavailable")?,
            request.stdout_limit,
            overflow.clone(),
        );
        let stderr = drain(
            child.0.stderr.take().ok_or("stderr pipe unavailable")?,
            request.stderr_limit,
            overflow.clone(),
        );
        let mut stdin = child.0.stdin.take().ok_or("stdin pipe unavailable")?;
        let (tx, rx) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let result = stdin.write_all(&request.stdin);
            drop(stdin);
            let _ = tx.send(result);
        });
        let start = Instant::now();
        let (mut out, mut err, mut input, mut status) = (None, None, None, None);
        loop {
            if cancel.cancelled() {
                child.terminate();
                return Err("process cancelled".into());
            }
            if overflow.load(Ordering::SeqCst) {
                child.terminate();
                return Err(
                    "process output limit exceeded; response was not truncated into success".into(),
                );
            }
            if start.elapsed() > request.deadline {
                child.terminate();
                return Err("process deadline exceeded; child group terminated".into());
            }
            if out.is_none()
                && let Ok(v) = stdout.try_recv()
            {
                out = Some(v)
            }
            if err.is_none()
                && let Ok(v) = stderr.try_recv()
            {
                err = Some(v)
            }
            if input.is_none()
                && let Ok(v) = rx.try_recv()
            {
                input = Some(v)
            }
            if status.is_none() {
                status = child.0.try_wait()?;
            }
            if let (Some(s), Some(_), Some(_), Some(_)) = (&status, &out, &err, &input) {
                let s = *s;
                let stdout = out.take().ok_or("missing stdout result")??;
                let stderr = err.take().ok_or("missing stderr result")??;
                // A rejected command may close its pipe; on success, that condition is an error.
                if s.success() {
                    input.take().ok_or("missing stdin result")??;
                }
                return Ok(ProcessOutput {
                    status: s,
                    stdout,
                    stderr,
                });
            }
            thread::sleep(Duration::from_millis(5));
        }
    }
}
/// Search only absolute PATH entries. LocalGit also rejects tools found inside its repository.
///
/// The containing directory is canonicalized, but the file name is kept as found, so a symlink
/// shim such as mise's `gh -> mise` still runs with the argv[0] it dispatches on.
pub fn resolve_program(name: &str) -> Result<PathBuf> {
    let path = std::env::var_os("PATH").ok_or("PATH is unset")?;
    resolve_program_in(name, &path)
}
fn resolve_program_in(name: &str, path: &std::ffi::OsStr) -> Result<PathBuf> {
    if name.contains('/') || name.contains('\\') {
        return Err("the program name must not contain a path".into());
    }
    for dir in std::env::split_paths(path) {
        if !dir.is_absolute() {
            continue;
        }
        let p = dir.join(name);
        if !p.is_file() {
            continue;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if p.metadata()?.permissions().mode() & 0o111 == 0 {
                continue;
            }
        }
        return Ok(dir.canonicalize()?.join(name));
    }
    Err(format!("no absolute PATH directory contains {name}").into())
}
/// A short, single-line stderr excerpt for error messages. Control characters are dropped and
/// words that look like credentials are redacted, since CLI tools may echo headers or URLs.
pub fn stderr_excerpt(stderr: &[u8]) -> Option<String> {
    const LIMIT: usize = 300;
    const SECRET_PREFIXES: [&str; 8] = [
        "ghp_",
        "gho_",
        "ghu_",
        "ghs_",
        "ghr_",
        "github_pat_",
        "glpat-",
        "gloas-",
    ];
    let text = String::from_utf8_lossy(&stderr[..stderr.len().min(4096)]);
    let mut words = vec![];
    let mut redact_next = false;
    // Bidirectional and zero-width format characters could disguise the displayed text.
    let hidden = |c: char| matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}');
    for word in text.split(|c: char| c.is_whitespace() || c.is_control() || hidden(c)) {
        if word.is_empty() {
            continue;
        }
        let lower = word.to_ascii_lowercase();
        let secret = (redact_next && word.len() >= 16)
            || SECRET_PREFIXES.iter().any(|p| lower.contains(p))
            || lower.contains("token=")
            || lower.contains("private-token:")
            || (lower.contains("://") && word.contains('@'));
        redact_next = matches!(
            lower.trim_end_matches(':'),
            "bearer" | "basic" | "token" | "authorization" | "private-token"
        );
        words.push(if secret { "[redacted]" } else { word });
    }
    let line = words.join(" ");
    if line.is_empty() {
        return None;
    }
    if line.len() <= LIMIT {
        return Some(line);
    }
    let mut end = LIMIT;
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    Some(format!("{}…", &line[..end]))
}
pub fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(libc::O_NONBLOCK);
    }
    let f = opts.open(path)?;
    let meta = f.metadata()?;
    if !meta.is_file() {
        return Err(
            "source must be a regular file, rather than a pipe, directory, or device".into(),
        );
    }
    if meta.len() > limit {
        return Err("file exceeds safety limit".into());
    }
    let mut bytes = Vec::new();
    f.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err("the file is over the safety limit; truncated input was not loaded".into());
    }
    Ok(bytes)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    fn executable(path: &Path, script: &str) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, script).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    #[test]
    #[cfg(unix)]
    fn symlinked_multicall_tool_keeps_its_own_name() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let (real, shims) = (temp.path().join("real"), temp.path().join("shims"));
        std::fs::create_dir_all(&real).unwrap();
        std::fs::create_dir_all(&shims).unwrap();
        // Like mise or busybox: the shim is a symlink, and the target dispatches on argv[0].
        executable(
            &real.join("multicall"),
            "#!/bin/sh\nprintf '%s' \"${0##*/}\"\n",
        );
        symlink(real.join("multicall"), shims.join("gh")).unwrap();
        let linked_dir = temp.path().join("linked-shims");
        symlink(&shims, &linked_dir).unwrap();
        let path = std::env::join_paths([PathBuf::from("relative/shims"), linked_dir]).unwrap();

        let resolved = resolve_program_in("gh", &path).unwrap();
        assert_eq!(resolved, shims.canonicalize().unwrap().join("gh"));
        let output = Runner::run(ProcessRequest::new(resolved), Cancellation::default()).unwrap();
        assert_eq!(output.stdout, b"gh");
    }
    #[test]
    #[cfg(unix)]
    fn only_absolute_path_entries_are_searched() {
        let temp = tempfile::tempdir().unwrap();
        executable(&temp.path().join("gh"), "#!/bin/sh\n");
        let relative = relative_to_cwd(temp.path());
        let path = std::env::join_paths([relative]).unwrap();
        assert!(resolve_program_in("gh", &path).is_err());
        assert!(resolve_program_in("../gh", temp.path().as_os_str()).is_err());
        assert!(resolve_program_in("gh", temp.path().as_os_str()).is_ok());
    }
    #[cfg(unix)]
    fn relative_to_cwd(dir: &Path) -> PathBuf {
        let cwd = std::env::current_dir().unwrap();
        let depth = cwd.components().count() - 1;
        let mut up: PathBuf = std::iter::repeat_n("..", depth).collect();
        up.push(dir.strip_prefix("/").unwrap());
        up
    }
    #[test]
    fn stderr_excerpt_is_bounded_single_line_and_redacted() {
        assert_eq!(stderr_excerpt(b" \n\t "), None);
        assert_eq!(
            stderr_excerpt(b"mise ERROR no tasks defined in ~\n\x1b[0m\r\n").as_deref(),
            Some("mise ERROR no tasks defined in ~ [0m")
        );
        let noisy = "HTTP 401: Bad credentials\nAuthorization: token ghp_abcdefghijklmnopqrstuvwxyz0123456789 \
             Bearer abcdefghijklmnopqrstuvwxyz https://user:secret@github.com/x?access_token=abc \
             glpat-0123456789abcdef \u{202e}txt.exe token expired";
        let excerpt = stderr_excerpt(noisy.as_bytes()).unwrap();
        assert_eq!(
            excerpt,
            "HTTP 401: Bad credentials Authorization: token [redacted] Bearer [redacted] \
             [redacted] [redacted] txt.exe token expired"
        );
        let long = stderr_excerpt("é".repeat(1000).as_bytes()).unwrap();
        assert!(long.len() <= 300 + '…'.len_utf8() && long.ends_with('…'));
    }
}
