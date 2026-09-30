//! Hooks for the macOS and Linux profile scripts, active only while `DIFFZ_PROFILE_STEPS` is set.
//! With `DIFFZ_PROFILE_STEPS=N` (N ≥ 0), the window redraws on every frame for a moment after
//! it opens, so GPUI grows its pool of frame buffers to the maximum on every run, and after
//! the first source installs, diffz steps through N files as `]` would. Unset, nothing runs.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

/// Frames redrawn back to back after the window opens: about two seconds at 120 Hz.
pub(super) const SATURATE_FRAMES: u32 = 240;
/// Pause between steps: long enough for each file to lay out and paint.
const STEP_MS: u64 = 300;

/// The step count a `DIFFZ_PROFILE_STEPS` value asks for; anything but a whole number is none.
fn parse_steps(value: Option<&str>) -> Option<usize> {
    value?.trim().parse().ok()
}
/// The profile step count, or `None` when profiling is off.
pub(super) fn steps() -> Option<usize> {
    static STEPS: std::sync::OnceLock<Option<usize>> = std::sync::OnceLock::new();
    *STEPS.get_or_init(|| parse_steps(std::env::var("DIFFZ_PROFILE_STEPS").ok().as_deref()))
}

impl Workbench {
    /// Lets snapshot readers release their references before asking glibc to return free pages.
    /// A newer replacement cancels this pending trim. The optional profile flag only enables
    /// logging its return value; the production trim itself is always scheduled on Linux/glibc.
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    pub(super) fn trim_allocator_after_switch(&mut self, cx: &mut Context<Self>) {
        const TRIM_DELAY: Duration = Duration::from_secs(3);
        self.cancel_allocator_trim();
        let cancel = Cancellation::default();
        self.allocator_trim_cancel = cancel.clone();
        let profile_log = std::env::var_os("DIFFZ_PROFILE_MALLOC_TRIM").is_some();

        cx.spawn(async move |_this, cx| {
            smol::Timer::after(TRIM_DELAY).await;
            if cancel.cancelled() {
                return;
            }
            let still_current = cancel.clone();
            let _ = cx
                .background_spawn(async move {
                    if still_current.cancelled() {
                        return;
                    }
                    unsafe extern "C" {
                        fn malloc_trim(pad: usize) -> i32;
                    }
                    // SAFETY: malloc_trim is glibc's process-wide allocator release operation.
                    let released = unsafe { malloc_trim(0) };
                    if profile_log {
                        eprintln!("diffz-profile malloc_trim {released}");
                    }
                })
                .await;
        })
        .detach();
    }

    /// Prevent a pending trim from contending with a source load.
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    pub(super) fn cancel_allocator_trim(&mut self) {
        self.allocator_trim_cancel.cancel();
    }
    /// Redraw on each of the next `frames` frames while profiling.
    pub(super) fn saturate_frames(
        &mut self,
        frames: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if steps().is_none() {
            return;
        }
        cx.notify();
        if let Some(left) = frames.checked_sub(1) {
            cx.on_next_frame(window, move |this, window, cx| {
                this.saturate_frames(left, window, cx)
            });
        }
    }
    /// Step through the profile's files once, after the first source installs; a later
    /// install in the same process, such as a hand-off, does not step. Writes
    /// `diffz-profile stepped <count>` to stderr when done.
    pub(super) fn start_profile_steps(&mut self, cx: &mut Context<Self>) {
        static STARTED: AtomicBool = AtomicBool::new(false);
        let Some(steps) = steps() else { return };
        if STARTED.swap(true, Ordering::Relaxed) {
            return;
        }
        cx.spawn(async move |this, cx| {
            let mut stepped = 0;
            while stepped < steps {
                smol::Timer::after(Duration::from_millis(STEP_MS)).await;
                match this.update(cx, |app, cx| app.step_file(true, cx)) {
                    Ok(true) => stepped += 1,
                    _ => break,
                }
            }
            eprintln!("diffz-profile stepped {stepped}");
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::parse_steps;

    #[test]
    fn profile_steps_need_a_whole_number() {
        assert_eq!(parse_steps(None), None);
        assert_eq!(parse_steps(Some("0")), Some(0));
        assert_eq!(parse_steps(Some("20")), Some(20));
        assert_eq!(parse_steps(Some(" 3\n")), Some(3));
        for ignored in ["", "-1", "ten", "2.5"] {
            assert_eq!(parse_steps(Some(ignored)), None, "{ignored:?}");
        }
    }
}
