use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

/// Writes an executable script without this process ever holding a write fd on it. Parallel
/// tests fork children that inherit open fds until exec, so an fd held here can make a later
/// `execve` of the script fail with ETXTBSY; the write fd lives only in the short-lived `sh`.
pub fn write_executable(path: &Path, script: &str) {
    let mut sh = Command::new("/bin/sh")
        .args(["-c", r#"cat > "$1" && chmod 700 "$1""#, "sh"])
        .arg(path)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    sh.stdin
        .take()
        .unwrap()
        .write_all(script.as_bytes())
        .unwrap();
    assert!(sh.wait().unwrap().success(), "writing {}", path.display());
}
