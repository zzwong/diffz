//! Touchpad scrolling state machines shared by the source and rich views: inertia after
//! the finger lifts, and the deliberate pull that turns to the next file at an edge.
//! Both are pure; the UI feeds them event times and pixel deltas and applies the results.
use std::collections::VecDeque;

/// Silence after the last finger event that means the finger lifted or stopped. Touchpads
/// report every 7–12 ms while a finger moves, so this is several report intervals.
pub const LIFT_GAP_MS: f64 = 40.;
/// How far back release velocity is estimated from, in milliseconds: a few reports, so a
/// finger that slows to a rest before lifting reads as stopped.
const VELOCITY_WINDOW_MS: f64 = 40.;
/// Slowest release, in logical pixels per millisecond, that still coasts.
pub const MIN_FLING_SPEED: f32 = 0.25;
/// Speed under which coasting stops.
const STOP_SPEED: f32 = 0.02;
/// Velocity retained after each millisecond of coasting; 0.997 gives a time constant of
/// about 330 ms, so a release at 1.5 px/ms travels roughly 500 px.
const DECAY_PER_MS: f32 = 0.997;
/// Coasting distance is clamped so a wild release cannot skip a whole file.
const MAX_FLING_DISTANCE: f32 = 4000.;

/// Inertia after a finger scroll ends. Wayland and X11 deliver raw touchpad deltas and
/// nothing after the finger lifts (GPUI also drops `axis_stop`), so the view decides that
/// the gesture ended once [`LIFT_GAP_MS`] pass without input and coasts from the release
/// velocity. macOS produces its own momentum events, so this is not used there.
#[derive(Default, Debug)]
pub struct Kinetic {
    samples: VecDeque<(f64, f32)>,
    fling: Option<Fling>,
}
#[derive(Debug, Clone, Copy)]
struct Fling {
    /// Release speed in logical pixels per millisecond, signed.
    v0: f32,
    started_ms: f64,
    emitted: f32,
}
impl Fling {
    fn tau() -> f32 {
        -1. / DECAY_PER_MS.ln()
    }
    fn distance(&self, now_ms: f64) -> f32 {
        let t = (now_ms - self.started_ms).max(0.) as f32;
        let d = self.v0 * Self::tau() * (1. - (-t / Self::tau()).exp());
        d.clamp(-MAX_FLING_DISTANCE, MAX_FLING_DISTANCE)
    }
    fn speed(&self, now_ms: f64) -> f32 {
        let t = (now_ms - self.started_ms).max(0.) as f32;
        (self.v0 * (-t / Self::tau()).exp()).abs()
    }
}
impl Kinetic {
    /// Record one finger delta (logical pixels, positive scrolls down). Any coast in
    /// progress stops: the finger is back on the pad.
    pub fn finger(&mut self, now_ms: f64, dy: f32) {
        self.fling = None;
        self.samples.push_back((now_ms, dy));
        while self
            .samples
            .front()
            .is_some_and(|(t, _)| now_ms - *t > VELOCITY_WINDOW_MS)
        {
            self.samples.pop_front();
        }
    }
    /// Stop coasting and forget the gesture; wheel clicks, keyboard scrolling and file
    /// changes call this.
    pub fn cancel(&mut self) {
        self.fling = None;
        self.samples.clear();
    }
    /// Release speed in pixels per millisecond estimated from the recent samples, or
    /// `None` when the finger had stopped before lifting.
    pub fn release_velocity(&self, now_ms: f64) -> Option<f32> {
        let (first, last) = (self.samples.front()?, self.samples.back()?);
        if self.samples.len() < 3 || now_ms - last.0 > LIFT_GAP_MS * 2. {
            return None;
        }
        let span = last.0 - first.0;
        if span < 8. {
            return None;
        }
        // The first sample's delta covers movement before the window opened.
        let distance: f32 = self.samples.iter().skip(1).map(|(_, dy)| *dy).sum();
        let v = distance / span as f32;
        (v.abs() >= MIN_FLING_SPEED).then_some(v)
    }
    /// The gesture ended at `now_ms`; start coasting when the release was fast enough.
    /// Returns whether a coast began.
    pub fn lift(&mut self, now_ms: f64) -> bool {
        let v0 = self.release_velocity(now_ms);
        self.samples.clear();
        self.fling = v0.map(|v0| Fling {
            v0,
            started_ms: now_ms,
            emitted: 0.,
        });
        self.fling.is_some()
    }
    pub fn coasting(&self) -> bool {
        self.fling.is_some()
    }
    /// Distance to scroll for the frame at `now_ms`, or `None` once coasting has stopped.
    /// Distance is a function of elapsed time, so a dropped frame resumes on the same curve.
    pub fn tick(&mut self, now_ms: f64) -> Option<f32> {
        let fling = self.fling.as_mut()?;
        let step = fling.distance(now_ms) - fling.emitted;
        fling.emitted += step;
        if fling.speed(now_ms) < STOP_SPEED {
            self.fling = None;
        }
        Some(step)
    }
}

/// Logical pixels a fresh gesture must pull past a file edge before the view turns to
/// the neighbouring file.
pub const PULL_THRESHOLD: f32 = 120.;

/// Turning to the previous or next file at a content edge. Reaching the edge arms the
/// gate; a later, separate gesture in the same direction then pulls against it, and the
/// turn happens once the pull reaches [`PULL_THRESHOLD`]. Coasting never pulls: the UI
/// feeds only finger and wheel events here and calls [`BoundaryScroll::arm`] when a
/// coast stops at an edge.
#[derive(Default, Debug)]
pub struct BoundaryScroll {
    armed: i8,
    pulling: bool,
    progress: f32,
}
impl BoundaryScroll {
    /// `edge` is the boundary the view sits at in the direction of `delta` (`0` when
    /// scrolling is still possible), `delta` the wheel movement (positive is down), and
    /// `fresh` whether this event starts a new gesture. Returns the direction to turn.
    pub fn update(&mut self, edge: i8, delta: f32, fresh: bool) -> Option<i8> {
        let direction = if delta > 0. {
            1
        } else if delta < 0. {
            -1
        } else {
            0
        };
        if edge == 0 {
            self.armed = direction;
            self.stop_pull();
            return None;
        }
        if edge != direction {
            self.armed = 0;
            self.stop_pull();
            return None;
        }
        if fresh {
            self.pulling = self.armed == edge;
            self.progress = 0.;
        }
        if !self.pulling {
            self.armed = edge;
            return None;
        }
        self.progress += delta.abs();
        if self.progress >= PULL_THRESHOLD {
            *self = Self::default();
            Some(edge)
        } else {
            None
        }
    }
    /// A coast (or keyboard jump) stopped at `edge`, so the next gesture may pull.
    pub fn arm(&mut self, edge: i8) {
        self.armed = edge;
        self.stop_pull();
    }
    /// The finger lifted: a pull in progress is abandoned, the arming stays.
    pub fn release(&mut self) {
        self.stop_pull();
    }
    /// How far the current pull has come, from 0 to 1.
    pub fn progress(&self) -> f32 {
        if self.pulling {
            (self.progress / PULL_THRESHOLD).clamp(0., 1.)
        } else {
            0.
        }
    }
    /// The edge being pulled against, or 0.
    pub fn pulling(&self) -> i8 {
        if self.pulling { self.armed } else { 0 }
    }
    fn stop_pull(&mut self) {
        self.pulling = false;
        self.progress = 0.;
    }
}
