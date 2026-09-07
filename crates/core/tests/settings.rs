use diffz_core::domain::Settings;
#[test]
fn missing_fields_fall_back_to_defaults() {
    let s: Settings = serde_json::from_str("{}").unwrap();
    assert_eq!(s, Settings::default());
    let s: Settings = serde_json::from_str(r#"{"split":true,"wrap":false}"#).unwrap();
    assert!(s.split);
    assert_eq!(s.wrap, Some(false));
    assert_eq!(s.font_size, 14.0);
    assert!(s.dark);
    assert_eq!(s.theme, None);
}
#[test]
fn round_trips() {
    let s = Settings {
        split: true,
        wrap: Some(true),
        font_size: 16.0,
        dark: false,
        theme: Some("tokyo-night".into()),
        rich: false,
        rich_inline: true,
    };
    let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    assert_eq!(back, s);
}
