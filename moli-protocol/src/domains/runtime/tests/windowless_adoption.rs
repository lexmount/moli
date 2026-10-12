use super::page_fixture::assert_page_fixture;

#[tokio::test(flavor = "multi_thread")]
async fn windowless_documents_adopt_native_nodes_across_real_pages() {
    assert_page_fixture(
        include_str!(
            "../../../../../moli-renderer-v8/tests/fixtures/windowless-cross-page-adoption.js"
        ),
        96,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn windowless_handlers_preserve_native_state_across_real_pages() {
    assert_page_fixture(
        include_str!(
            "../../../../../moli-renderer-v8/tests/fixtures/windowless-cross-page-handlers.js"
        ),
        208,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn windowless_attributes_follow_the_native_owner_across_real_pages() {
    assert_page_fixture(
        include_str!(
            "../../../../../moli-renderer-v8/tests/fixtures/windowless-cross-page-attributes.js"
        ),
        80,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn windowless_handler_policy_survives_real_popup_retirement() {
    let fixture = include_str!(
        "../../../../../moli-renderer-v8/tests/fixtures/windowless-handler-csp-retirement.js"
    );
    assert_page_fixture(
        &format!(
            "const passed = {fixture}; globalThis.__uiEventResults = \
             {{total:1, passed:passed ? 1 : 0, checks:[{{name:'retained global policy',passed}}]}};"
        ),
        1,
    )
    .await;
}
