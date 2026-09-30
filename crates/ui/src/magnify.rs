//! Pinch to magnify the whole window, like Safari: a visual zoom that leaves layout and the
//! code font size alone. GPUI does the magnifying (`Window::set_magnification`); this module is
//! diffz's policy for when to, and is off unless `DIFFZ_PINCH_MAGNIFY=1` while it is a spike.
//!
//! `DIFFZ_DEBUG_MAGNIFY=<scale>@<x>,<y>` starts the window magnified by `scale` about the
//! window point `x`,`y`, to check text crispness and click mapping without a trackpad.
use gpui_kit::*;
use std::sync::OnceLock;

/// The most a pinch magnifies.
pub(crate) const MAX_SCALE: f32 = 5.0;
/// A pinch that ends below this snaps back to no magnification.
const SNAP_TO_ONE_BELOW: f32 = 1.05;

/// Whether a trackpad pinch magnifies the window (`DIFFZ_PINCH_MAGNIFY=1`).
pub(crate) fn pinch_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("DIFFZ_PINCH_MAGNIFY").is_ok_and(|v| v == "1"))
}

/// Magnifies the window for one pinch event. The point under the fingers stays put.
pub(crate) fn pinch(event: &PinchEvent, window: &mut Window) {
    let scale = next_scale(window.magnification().scale, event.delta);
    window.magnify_about(event.position, scale);
    // Keep live caches at the cap so reversing the pinch can reuse them.
    window.set_magnification_live(matches!(
        event.phase,
        TouchPhase::Started | TouchPhase::Moved
    ));
    window.set_magnification_live_raster_exact(scale >= MAX_SCALE);
    if event.phase == TouchPhase::Ended && settle(window.magnification().scale) == 1.0 {
        window.reset_magnification();
    }
}

/// Drops any magnification, reporting whether there was one to drop.
pub(crate) fn reset(window: &mut Window) -> bool {
    if window.magnification().is_identity() {
        return false;
    }
    window.set_magnification_live(false);
    window.reset_magnification();
    true
}

/// Applies `DIFFZ_DEBUG_MAGNIFY`, if set, to a new window.
pub(crate) fn apply_debug_magnification(window: &mut Window) {
    let Some((scale, anchor)) = std::env::var("DIFFZ_DEBUG_MAGNIFY")
        .ok()
        .as_deref()
        .and_then(parse_debug)
    else {
        return;
    };
    window.magnify_about(anchor, scale.min(MAX_SCALE));
}

/// The scale after a pinch step. macOS reports each step as a fraction to grow by.
fn next_scale(scale: f32, delta: f32) -> f32 {
    let next = scale * (1.0 + delta);
    if next.is_finite() {
        next.clamp(1.0, MAX_SCALE)
    } else {
        scale
    }
}

/// The scale a finished pinch rests at.
fn settle(scale: f32) -> f32 {
    if scale < SNAP_TO_ONE_BELOW {
        1.0
    } else {
        scale
    }
}

/// Parses `<scale>@<x>,<y>`, or a bare `<scale>` to magnify about the top-left corner.
fn parse_debug(value: &str) -> Option<(f32, Point<Pixels>)> {
    let (scale, anchor) = match value.split_once('@') {
        Some((scale, anchor)) => (scale, Some(anchor)),
        None => (value, None),
    };
    let scale: f32 = scale
        .trim()
        .parse()
        .ok()
        .filter(|s: &f32| s.is_finite() && *s >= 1.0)?;
    let anchor = match anchor {
        Some(anchor) => {
            let (x, y) = anchor.split_once(',')?;
            let x: f32 = x.trim().parse().ok().filter(|v: &f32| v.is_finite())?;
            let y: f32 = y.trim().parse().ok().filter(|v: &f32| v.is_finite())?;
            point(px(x), px(y))
        }
        None => Point::default(),
    };
    Some((scale, anchor))
}

#[cfg(test)]
mod tests {
    use super::{MAX_SCALE, next_scale, parse_debug, pinch, settle};
    use gpui_kit::{EmptyView, PinchEvent, Point, TestAppContext, TouchPhase, point, px};

    #[gpui_kit::gpui::test]
    fn pinch_cap_keeps_live_caches_until_the_gesture_ends(cx: &mut TestAppContext) {
        cx.skip_drawing();
        let handle = cx.add_window(|_, _| EmptyView);
        handle
            .update(cx, |_, window, _| {
                let mut event = PinchEvent {
                    delta: 1.0,
                    phase: TouchPhase::Started,
                    ..Default::default()
                };
                pinch(&event, window);
                assert_eq!(window.magnification().scale, 2.0);
                assert!(window.is_magnification_live());

                event.phase = TouchPhase::Moved;
                event.delta = 2.0;
                pinch(&event, window);
                assert_eq!(window.magnification().scale, MAX_SCALE);
                assert!(window.is_magnification_live());

                event.delta = 0.1;
                pinch(&event, window);
                assert_eq!(window.magnification().scale, MAX_SCALE);
                assert!(window.is_magnification_live());

                event.delta = -0.2;
                pinch(&event, window);
                assert_eq!(window.magnification().scale, 4.0);
                assert!(window.is_magnification_live());

                event.delta = 0.0;
                event.phase = TouchPhase::Ended;
                pinch(&event, window);
                assert!(!window.is_magnification_live());
            })
            .unwrap();
    }

    #[::core::prelude::v1::test]
    fn pinch_steps_grow_by_their_fraction_and_clamp() {
        assert_eq!(next_scale(1.0, 0.5), 1.5);
        assert_eq!(next_scale(2.0, -0.25), 1.5);
        assert_eq!(next_scale(4.0, 1.0), MAX_SCALE);
        assert_eq!(next_scale(1.2, -0.9), 1.0);
        assert_eq!(next_scale(2.0, f32::NAN), 2.0);
    }

    #[::core::prelude::v1::test]
    fn a_pinch_ending_near_one_snaps_back() {
        assert_eq!(settle(1.02), 1.0);
        assert_eq!(settle(1.05), 1.05);
        assert_eq!(settle(3.0), 3.0);
    }

    #[::core::prelude::v1::test]
    fn debug_magnification_parses_scale_and_anchor() {
        assert_eq!(
            parse_debug("2@400,300"),
            Some((2.0, point(px(400.), px(300.))))
        );
        assert_eq!(
            parse_debug(" 4 @ 10 , 20 "),
            Some((4.0, point(px(10.), px(20.))))
        );
        assert_eq!(parse_debug("1.5"), Some((1.5, Point::default())));
        assert_eq!(parse_debug("0.5@1,1"), None);
        assert_eq!(parse_debug("2@1"), None);
        assert_eq!(parse_debug("big"), None);
        assert_eq!(parse_debug("inf"), None);
    }
}
