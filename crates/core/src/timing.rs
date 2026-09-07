//! Timing marks for startup, written to stderr only while `DIFFZ_TIMING` is present.
//! Each mark records wall time in Unix milliseconds, letting a launcher subtract its own
//! startup cost and observe the whole path, including dynamic loading.
use std::sync::atomic::{AtomicBool, Ordering};

fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("DIFFZ_TIMING").is_some())
}
fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}
/// Emit one named mark.
pub fn mark(name: &str) {
    if enabled() {
        eprintln!("diffz-timing {name} {}", now_ms());
    }
}
/// Emit a mark on first reach; later calls do nothing.
pub fn mark_once(name: &str) {
    static SEEN: AtomicBool = AtomicBool::new(false);
    if enabled() && !SEEN.swap(true, Ordering::Relaxed) {
        mark(name);
    }
}
