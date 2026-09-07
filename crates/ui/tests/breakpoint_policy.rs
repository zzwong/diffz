//! These are policy tests, not font-rendering tests. Use --layout-probe for the actual backend.
use diffz_ui::native_text::measured_breaks;
use std::collections::HashSet;
#[test]
fn exact_native_widths_not_character_counts() {
    let c = vec![(0, 0.0), (1, 12.0), (2, 14.0), (3, 16.0), (4, 28.0)];
    let r = measured_breaks(&c, &HashSet::new(), Some(15.0));
    assert_eq!(r, vec![0..2, 2..4]);
}
#[test]
fn preferred_unicode_break_before_emergency() {
    let c = (0..=8).map(|i| (i, i as f32 * 10.0)).collect::<Vec<_>>();
    let p = HashSet::from([3]);
    assert_eq!(measured_breaks(&c, &p, Some(50.0)), vec![0..3, 3..8]);
}
#[test]
fn a_wide_cluster_is_not_dropped() {
    assert_eq!(
        measured_breaks(&[(0, 0.0), (25, 30.0)], &HashSet::new(), Some(8.0)),
        std::iter::once(0..25).collect::<Vec<_>>()
    );
}
#[test]
fn wrapping_never_discards_tail() {
    let c = (0..=1000).map(|i| (i, i as f32)).collect::<Vec<_>>();
    let p = measured_breaks(&c, &HashSet::new(), Some(37.0));
    assert_eq!(p[0].start, 0);
    assert_eq!(p.last().unwrap().end, 1000);
    for w in p.windows(2) {
        assert_eq!(w[0].end, w[1].start);
    }
}
