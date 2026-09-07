use crate::domain::SourcePoint;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewportAnchor {
    pub point: SourcePoint,
    pub viewport_y: f32,
    pub horizontal: f32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationReason {
    Open,
    File,
    Hunk,
    Find,
    Thread,
    Restore,
    AcceptRevision,
    UserCollapse,
}
#[derive(Debug, Default)]
pub struct NavigationClock(u64);
impl NavigationClock {
    pub fn generation(&self) -> u64 {
        self.0
    }
    pub fn user_input(&mut self) {
        self.0 = self.0.wrapping_add(1)
    }
    pub fn accepts(&self, g: u64) -> bool {
        self.0 == g
    }
}
pub fn restore_scroll(
    a: &ViewportAnchor,
    source_y: f32,
    content: f32,
    viewport: f32,
) -> Result<f32, &'static str> {
    if [a.viewport_y, source_y, content, viewport]
        .iter()
        .any(|v| !v.is_finite())
        || content < 0.
        || viewport < 0.
    {
        return Err("invalid measured geometry");
    }
    Ok((source_y - a.viewport_y).clamp(0., (content - viewport).max(0.)))
}
