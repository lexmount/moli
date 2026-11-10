use style::typed_om::{ToTyped, TypedValue};

#[test]
fn empty_keyword_reification_does_not_require_typed_elements() {
    use style::values::generics::font::FontSettings;

    // () deliberately does not implement ToTyped. The empty sentinel is a
    // keyword; arbitrary elements must remain unsupported without extra bounds.
    let empty = FontSettings::<()>::normal().to_typed_value().unwrap();
    assert!(matches!(empty, TypedValue::Keyword(value) if value.0 == "normal"));
    assert!(
        FontSettings(vec![()].into_boxed_slice())
            .to_typed_value()
            .is_none()
    );
}
