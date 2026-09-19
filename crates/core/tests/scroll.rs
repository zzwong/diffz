use diffz_core::scroll::{BoundaryScroll, Kinetic, LIFT_GAP_MS, MIN_FLING_SPEED, PULL_THRESHOLD};

/// A touchpad stream: `n` finger reports `dy` pixels apart, 8 ms apart, starting at `t0`.
fn finger_stream(k: &mut Kinetic, t0: f64, n: usize, dy: f32) -> f64 {
    let mut t = t0;
    for _ in 0..n {
        k.finger(t, dy);
        t += 8.;
    }
    t - 8.
}

#[test]
fn fast_release_coasts_and_decays_to_a_stop() {
    let mut k = Kinetic::default();
    let last = finger_stream(&mut k, 0., 12, 12.); // 1.5 px/ms
    assert!(
        k.release_velocity(last + 10.)
            .is_some_and(|v| (v - 1.5).abs() < 0.05)
    );
    assert!(k.lift(last + LIFT_GAP_MS));
    assert!(k.coasting());
    let start = last + LIFT_GAP_MS;
    let mut total = 0.;
    let mut t = start;
    let mut steps = vec![];
    while let Some(step) = k.tick(t) {
        steps.push(step);
        total += step;
        t += 8.333;
        assert!(t < start + 5000., "coast must end");
    }
    assert!(!k.coasting());
    assert!(
        steps.iter().all(|s| *s >= 0.),
        "steps keep the release direction"
    );
    assert!(
        steps[1..].windows(2).all(|w| w[1] <= w[0] + 1e-3),
        "steps shrink"
    );
    assert!(
        (300. ..800.).contains(&total),
        "1.5 px/ms release travels {total} px"
    );
}

#[test]
fn stopped_finger_does_not_coast() {
    let mut k = Kinetic::default();
    let last = finger_stream(&mut k, 0., 12, 12.);
    // The finger decelerates to a rest before lifting.
    let mut t = last;
    for dy in [6., 3., 1., 0.5, 0.2, 0., 0., 0.] {
        t += 8.;
        k.finger(t, dy);
    }
    assert_eq!(k.release_velocity(t + 10.), None);
    assert!(!k.lift(t + LIFT_GAP_MS));
    assert_eq!(k.tick(t + 100.), None);
}

#[test]
fn slow_release_does_not_coast() {
    let mut k = Kinetic::default();
    let slow = MIN_FLING_SPEED * 8. * 0.8;
    let last = finger_stream(&mut k, 0., 12, slow);
    assert!(!k.lift(last + LIFT_GAP_MS));
}

#[test]
fn single_wheel_click_has_no_velocity() {
    let mut k = Kinetic::default();
    k.finger(0., 45.);
    assert_eq!(k.release_velocity(20.), None);
    k.finger(60., 45.);
    assert_eq!(k.release_velocity(80.), None, "two samples are not enough");
}

#[test]
fn new_touch_cancels_the_coast() {
    let mut k = Kinetic::default();
    let last = finger_stream(&mut k, 0., 12, 12.);
    assert!(k.lift(last + LIFT_GAP_MS));
    assert!(k.tick(last + LIFT_GAP_MS + 8.).is_some());
    k.finger(last + 100., 2.);
    assert!(!k.coasting());
    assert_eq!(k.tick(last + 108.), None);
    k.cancel();
    assert!(!k.coasting());
}

#[test]
fn coast_distance_depends_on_time_not_frame_count() {
    let mut a = Kinetic::default();
    let mut b = Kinetic::default();
    let last = finger_stream(&mut a, 0., 12, 12.);
    finger_stream(&mut b, 0., 12, 12.);
    a.lift(last + LIFT_GAP_MS);
    b.lift(last + LIFT_GAP_MS);
    let start = last + LIFT_GAP_MS;
    let mut da = 0.;
    let mut t = start;
    while t <= start + 200. {
        da += a.tick(t).unwrap_or(0.);
        t += 8.333;
    }
    let db: f32 = [start + 100., start + 200.]
        .iter()
        .map(|t| b.tick(*t).unwrap_or(0.))
        .sum();
    assert!(
        (da - db).abs() < 1.,
        "same elapsed time, same distance: {da} vs {db}"
    );
}

#[test]
fn reaching_the_edge_arms_and_a_fresh_pull_turns_after_the_threshold() {
    let mut gate = BoundaryScroll::default();
    assert_eq!(gate.update(0, 30., false), None);
    assert_eq!(
        gate.update(1, 30., false),
        None,
        "the gesture that reached the edge"
    );
    assert_eq!(gate.progress(), 0.);
    // A separate gesture starts pulling.
    assert_eq!(gate.update(1, 30., true), None);
    assert!(gate.progress() > 0. && gate.progress() < 1.);
    assert_eq!(gate.pulling(), 1);
    let mut turned = None;
    for _ in 0..20 {
        if let Some(d) = gate.update(1, 30., false) {
            turned = Some(d);
            break;
        }
    }
    assert_eq!(turned, Some(1));
    assert_eq!(gate.progress(), 0.);
    assert_eq!(gate.pulling(), 0);
}

#[test]
fn tiny_events_after_a_pause_do_not_turn() {
    let mut gate = BoundaryScroll::default();
    assert_eq!(gate.update(0, 30., false), None);
    assert_eq!(gate.update(1, 30., false), None);
    // A resting finger jitters after the pause: fresh but far below the threshold.
    assert_eq!(gate.update(1, 0.3, true), None);
    assert_eq!(gate.update(1, 0.2, false), None);
    assert!(gate.progress() < 0.01);
}

#[test]
fn lifting_abandons_the_pull_but_keeps_the_arming() {
    let mut gate = BoundaryScroll::default();
    assert_eq!(gate.update(1, 30., false), None);
    assert_eq!(gate.update(1, 30., true), None);
    assert!(gate.progress() > 0.);
    gate.release();
    assert_eq!(gate.progress(), 0.);
    assert_eq!(gate.pulling(), 0);
    // The next fresh pull starts from zero and still needs the full threshold.
    let mut turned = None;
    let mut pulled = 0.;
    let mut fresh = true;
    while turned.is_none() {
        turned = gate.update(1, 30., fresh);
        fresh = false;
        pulled += 30.;
    }
    assert!(pulled >= PULL_THRESHOLD);
}

#[test]
fn reversing_direction_disarms() {
    let mut gate = BoundaryScroll::default();
    assert_eq!(gate.update(1, 30., false), None);
    assert_eq!(gate.update(1, -30., true), None);
    assert_eq!(gate.update(1, 30., true), None);
    assert_eq!(gate.pulling(), 0, "the edge must be reached again first");
}

#[test]
fn a_coast_that_stops_at_the_edge_arms_the_gate() {
    let mut gate = BoundaryScroll::default();
    gate.arm(1);
    assert_eq!(gate.update(1, 30., true), None);
    assert_eq!(gate.pulling(), 1);
}

#[test]
fn a_single_gesture_never_turns_even_past_the_threshold() {
    let mut gate = BoundaryScroll::default();
    assert_eq!(gate.update(0, 30., true), None);
    for _ in 0..50 {
        assert_eq!(gate.update(1, 30., false), None);
    }
    assert_eq!(gate.progress(), 0.);
}
