use crate::{AdapterError, Result};
use std::{fs::OpenOptions, io::Write, path::Path};
/// create_new fails when the path already exists or names a symlink target, which keeps the write atomic. Exported paths are never interpreted further.
pub fn write_private_json(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("export exceeds 16 MiB limit".into());
    }
    let mut o = OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut f = o.open(path)?;
    if let Err(e) = f
        .write_all(&bytes)
        .and_then(|_| f.write_all(b"\n"))
        .and_then(|_| f.sync_all())
    {
        drop(f);
        let _ = std::fs::remove_file(path);
        return Err(AdapterError::Io(e));
    }
    Ok(())
}
