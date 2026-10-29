use super::*;

#[test]
fn scroll_dictionaries_preserve_conversion_order_and_silent_cancellation() {
    for policy in [
        moli_page_types::LayoutPolicy::Mock,
        moli_page_types::LayoutPolicy::OnDemand,
    ] {
        let mut vm = new_storage_html_test_vm("https://scroll-dictionary.test/");
        vm.set_layout_policy(policy);
        assert_eq!(
            vm.eval(include_str!(
                "../../../../tests/fixtures/scroll-options-dictionary.js"
            ))
            .unwrap(),
            "true"
        );
    }
}

#[test]
fn native_scroll_uses_state_after_conversion_without_partial_updates() {
    let mut vm = new_storage_html_test_vm("https://scroll-native-conversion.test/");
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    assert_eq!(
        eval_with_layout_publications(
            &mut vm,
            include_str!("../../../../tests/fixtures/scroll-native-geometry.js"),
        )
        .unwrap(),
        "true"
    );
}
