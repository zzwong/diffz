use super::*;

/// The floating file panel shown while the collapsed panel's toggle, or the panel
/// itself, is hovered. `open` is the target; `visibility` is where the motion is.
#[derive(Default)]
pub struct FilesPeek {
    pub open: bool,
    pub(super) visibility: f32,
    button_hovered: bool,
    pub(super) panel_hovered: bool,
    pub(super) edge_hovered: bool,
    edge_blocked: bool,
}

impl FilesPeek {
    /// Arms on the hover-enter event alone: a click that collapses the pinned panel
    /// leaves the pointer on the button, and reading "is hovered" would reopen it.
    pub(crate) fn hover_button(&mut self, hovered: bool, pinned: bool) -> bool {
        let entered = hovered && !self.button_hovered;
        self.button_hovered = hovered;
        if entered && !pinned {
            self.open = true;
        }
        self.closing()
    }
    pub(super) fn hover_panel(&mut self, hovered: bool) -> bool {
        self.panel_hovered = hovered;
        // A pointer that catches the fading panel brings it back from where it is.
        if hovered && self.visibility > 0. {
            self.open = true;
        }
        self.closing()
    }
    /// Reports whether a dwell timer should start, and whether a close should be
    /// scheduled. The edge opens on dwell rather than on entry, so a pointer merely
    /// passing out of the window leaves nothing behind.
    pub(super) fn hover_edge(&mut self, hovered: bool) -> (bool, bool) {
        let entered = hovered && !self.edge_hovered;
        self.edge_hovered = hovered;
        if !hovered {
            self.edge_blocked = false;
        }
        (entered && !self.edge_blocked, self.closing())
    }
    /// The dwell ran out with the pointer still on the edge: true when it opened.
    pub(super) fn dwell_elapsed(&mut self, pinned: bool) -> bool {
        if self.open || pinned || self.edge_blocked || !self.edge_hovered {
            return false;
        }
        self.open = true;
        true
    }
    /// The pointer left the window. Wayland sends no move with that, so every hover
    /// flag would stay set and hold the panel open over a window the user moved on to.
    pub(super) fn pointer_left(&mut self) -> bool {
        self.button_hovered = false;
        self.panel_hovered = false;
        self.edge_hovered = false;
        self.closing()
    }
    /// A pointer seen away from the edge ends the block left by a collapse.
    pub(super) fn clear_edge_block(&mut self, pointer_x: f32) {
        if pointer_x > FILES_PEEK_EDGE_PX {
            self.edge_blocked = false;
        }
    }
    fn closing(&self) -> bool {
        self.open && !self.button_hovered && !self.panel_hovered && !self.edge_hovered
    }
    pub(super) fn expire(&mut self) -> bool {
        let close = self.closing();
        if close {
            self.open = false;
        }
        close
    }
    pub(super) fn reset(&mut self) {
        // The collapse gesture ends with the pointer against the left edge, where the
        // zone appears under it; without the block it would hand the panel straight back.
        *self = Self {
            edge_blocked: true,
            ..Self::default()
        };
    }
    pub(super) fn shown(&self) -> bool {
        self.open || self.visibility > 0.
    }
    fn target(&self) -> f32 {
        if self.open { 1. } else { 0. }
    }
    pub(super) fn moving(&self) -> bool {
        self.visibility != self.target()
    }
    /// Steps the progress towards the target; true while it has further to go.
    pub(super) fn advance(&mut self, dt_ms: f32) -> bool {
        let target = self.target();
        let step = dt_ms
            / if self.open {
                FILES_PEEK_IN_MS
            } else {
                FILES_PEEK_OUT_MS
            };
        self.visibility = if self.visibility < target {
            (self.visibility + step).min(target)
        } else {
            (self.visibility - step).max(target)
        };
        self.moving()
    }
    /// Left offset in pixels and opacity. Both directions read the same progress
    /// through the same curve, so a reversal mid-flight stays continuous.
    pub(super) fn frame(&self) -> (f32, f32) {
        let eased = ease_out_cubic(self.visibility);
        (-FILES_PEEK_SLIDE_PX * (1. - eased), eased)
    }
}
