//! Wheel and touchpad scrolling shared by the source and rich views: gesture tracking,
//! inertia after the finger lifts, and the pull that turns to a neighbouring file at an
//! edge. The pure state machines live in `diffz_core::scroll`; this file feeds them
//! platform events and applies their output to whichever view is showing.
use crate::app::{Panel, Workbench};
use diffz_core::scroll::LIFT_GAP_MS;
use gpui_kit::{prelude::*, *};
use std::time::{Duration, Instant};

/// Whether the platform leaves inertia to the application. macOS synthesises momentum
/// events itself; Wayland and X11 deliver raw finger deltas and then nothing.
const CLIENT_INERTIA: bool = !cfg!(target_os = "macos");
/// A gesture that begins within this long of the previous one ending does not count as a
/// separate, deliberate gesture at a file edge.
const FRESH_GAP: Duration = Duration::from_millis(350);

/// The view a wheel event or coast step applies to.
pub(crate) enum ScrollTarget {
    Source,
    Rich(ListState),
}

impl Workbench {
    fn now_ms(&self) -> f64 {
        self.gesture_epoch.elapsed().as_secs_f64() * 1000.
    }
    /// Both views route their `on_scroll_wheel` here.
    pub(crate) fn wheel(
        &mut self,
        event: &ScrollWheelEvent,
        target: ScrollTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if diffz_core::timing::scroll_trace() {
            eprintln!(
                "diffz-scroll wheel t={} phase={:?} delta={:?}",
                diffz_core::timing::trace_us(),
                event.touch_phase,
                event.delta
            );
        }
        if self.panel != Panel::None {
            return;
        }
        let font = self.viewport.as_ref().map_or(14., |v| v.borrow().font_size);
        let (x, y, precise) = match event.delta {
            ScrollDelta::Pixels(p) => (f32::from(p.x), f32::from(p.y), true),
            ScrollDelta::Lines(p) => (p.x * font * 1.4, p.y * font * 1.4, false),
        };
        let now = Instant::now();
        let now_ms = self.now_ms();
        // libinput reports a zero delta when the finger leaves the pad, and the compositor
        // forwards it just before `axis_stop`; GPUI drops the stop but passes the zero.
        let lifted = matches!(event.touch_phase, TouchPhase::Ended | TouchPhase::Cancelled)
            || (precise && x == 0. && y == 0.);
        if lifted {
            self.finger_lifted(window, cx);
            return;
        }
        // A new gesture: the platform says so, or the previous one ended a while ago. The
        // gap matters on macOS too, where the system's own momentum events follow `Ended`.
        let fresh = event.touch_phase == TouchPhase::Started
            || (!self.gesture_active
                && self
                    .last_wheel
                    .is_none_or(|last| now.duration_since(last) > FRESH_GAP));
        self.last_wheel = Some(now);
        self.gesture_active = true;
        if precise && CLIENT_INERTIA {
            self.kinetic.finger(now_ms, -y);
        } else {
            self.kinetic.cancel();
        }
        self.show_scrollbars(cx);
        // The lift may never be reported (X11, other compositors); a silence decides it.
        self.gesture_task = Some(cx.spawn_in(window, async move |this, cx| {
            smol::Timer::after(Duration::from_millis(LIFT_GAP_MS as u64)).await;
            let _ = this.update_in(cx, |this, window, cx| this.finger_lifted(window, cx));
        }));

        let mut turn = None;
        if y.abs() > x.abs() && y.abs() > 0.1 {
            let edge = match &target {
                ScrollTarget::Source => self
                    .viewport
                    .as_ref()
                    .map_or(0, |v| v.borrow().boundary(-y)),
                ScrollTarget::Rich(list) => {
                    let (start, end) = rich_edges(list);
                    Workbench::edge_of(-y, start, end)
                }
            };
            turn = self.boundary_scroll.update(edge, -y, fresh);
            if edge != 0 {
                self.status = edge_status(edge, self.boundary_scroll.progress()).into();
            }
        }
        match target {
            ScrollTarget::Source => {
                if turn.is_none()
                    && let Some(v) = &self.viewport
                {
                    v.borrow_mut().scroll(-x, -y);
                }
            }
            // The list scrolled itself before this bubble-phase listener ran.
            ScrollTarget::Rich(_) => {}
        }
        if let Some(direction) = turn {
            self.kinetic.cancel();
            self.turn_file(direction, window, cx);
        }
        cx.stop_propagation();
        cx.notify();
    }
    /// The gesture ended: start coasting when the release was fast, drop any pull in
    /// progress, and persist the position.
    fn finger_lifted(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.gesture_task = None;
        if !self.gesture_active {
            return;
        }
        self.gesture_active = false;
        self.boundary_scroll.release();
        let now_ms = self.now_ms();
        if CLIENT_INERTIA && self.kinetic.lift(now_ms) {
            if diffz_core::timing::scroll_trace() {
                eprintln!("diffz-scroll coast t={}", diffz_core::timing::trace_us());
            }
            self.start_coast(window, cx);
        } else {
            self.kinetic.cancel();
            self.scroll_settled(cx);
        }
        cx.notify();
    }
    /// Advance the coast once per frame until it stops or meets an edge.
    fn start_coast(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.coast_ticking {
            return;
        }
        self.coast_ticking = true;
        cx.on_next_frame(window, |this, window, cx| this.coast_tick(window, cx));
    }
    fn coast_tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let now_ms = self.now_ms();
        if let Some(step) = self.kinetic.tick(now_ms)
            && step != 0.
            && !self.apply_coast_step(step)
        {
            // The coast met an edge: stop there and let the next gesture pull.
            self.kinetic.cancel();
            self.boundary_scroll.arm(if step > 0. { 1 } else { -1 });
        }
        if self.kinetic.coasting() {
            cx.on_next_frame(window, |this, window, cx| this.coast_tick(window, cx));
        } else {
            self.coast_ticking = false;
            self.scroll_settled(cx);
        }
        cx.notify();
    }
    /// Scroll the showing view by `step` (positive is down); false when it sits at the
    /// edge the step moves towards.
    fn apply_coast_step(&mut self, step: f32) -> bool {
        if self.rich_active() {
            let Some(list) = self.rich_cache.as_ref().map(|c| c.list.clone()) else {
                return false;
            };
            let (start, end) = rich_edges(&list);
            if Workbench::edge_of(step, start, end) != 0 {
                return false;
            }
            list.scroll_by(px(step));
            true
        } else {
            let Some(v) = &self.viewport else {
                return false;
            };
            let mut v = v.borrow_mut();
            if v.boundary(step) != 0 {
                return false;
            }
            v.scroll(0., step);
            true
        }
    }
    /// Scrolling has come to rest: save the view and hide the scrollbars after a moment.
    fn scroll_settled(&mut self, cx: &mut Context<Self>) {
        self.schedule_view_save(cx);
        self.scrollbar_hide_task = Some(cx.spawn(async move |this, cx| {
            loop {
                smol::Timer::after(Duration::from_millis(700)).await;
                let done = this
                    .update(cx, |this, cx| {
                        if this.scrollbar_drag || this.horizontal_drag || this.gesture_active {
                            return false;
                        }
                        if let Some(v) = &this.viewport {
                            v.borrow_mut().scrollbars_visible = false;
                        }
                        cx.notify();
                        true
                    })
                    .unwrap_or(true);
                if done {
                    break;
                }
            }
        }));
    }
    fn show_scrollbars(&mut self, cx: &mut Context<Self>) {
        if let Some(v) = &self.viewport {
            let mut v = v.borrow_mut();
            if !v.scrollbars_visible {
                v.scrollbars_visible = true;
                cx.notify();
            }
        }
        // The hide timer restarts once the gesture (or its coast) settles.
        self.scrollbar_hide_task = None;
    }
    /// Stop any coast and forget the gesture, e.g. when the file changes under it.
    pub(crate) fn cancel_scroll_gesture(&mut self) {
        self.kinetic.cancel();
        self.gesture_active = false;
        self.gesture_task = None;
    }
}

/// Whether the rich list sits at its first item's top, and at its last item's bottom.
pub(crate) fn rich_edges(list: &ListState) -> (bool, bool) {
    let top = list.logical_scroll_top();
    let at_start = top.item_ix == 0 && top.offset_in_item <= px(0.5);
    let count = list.item_count();
    let viewport = list.viewport_bounds();
    let at_end = count == 0
        || list
            .bounds_for_item(count - 1)
            .is_some_and(|b| b.bottom() <= viewport.bottom() + px(0.5));
    (at_start, at_end)
}

/// Status line while a gesture sits at a file edge.
fn edge_status(edge: i8, progress: f32) -> &'static str {
    match (edge > 0, progress > 0.) {
        (true, false) => "End of file · scroll again to pull the next file in",
        (true, true) => "Keep pulling for the next file",
        (false, false) => "Start of file · scroll again to pull the previous file in",
        (false, true) => "Keep pulling for the previous file",
    }
}
