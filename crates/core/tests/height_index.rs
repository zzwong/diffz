use diffz_core::height_index::HeightIndex;
#[test]
fn prefix_updates_and_lookup() {
    let mut h = HeightIndex::new(5, 20.0);
    assert_eq!(h.total(), 100.0);
    h.update(1, 100.0).unwrap();
    assert_eq!(h.prefix(2), 120.0);
    assert_eq!(h.total(), 180.0);
    assert_eq!(h.locate(21.0), (1, 1.0));
    assert_eq!(h.locate(150.0), (3, 10.0));
}
#[test]
fn invalid_geometry_is_rejected() {
    let mut h = HeightIndex::new(2, 20.0);
    assert!(h.update(0, f32::NAN).is_err());
    assert!(h.update(4, 20.0).is_err());
    assert!(h.update(0, -1.0).is_err());
}
#[test]
fn empty_list_has_no_fake_row() {
    let h = HeightIndex::new(0, 20.0);
    assert_eq!(h.total(), 0.0);
    assert_eq!(h.locate(9.0), (0, 0.0));
}
