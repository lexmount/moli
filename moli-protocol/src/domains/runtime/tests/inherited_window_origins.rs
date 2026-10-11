use super::page_fixture::assert_page_fixture;

#[tokio::test(flavor = "multi_thread")]
async fn inherited_opaque_popup_children_keep_origin_tokens_and_sandbox_isolation() {
    assert_page_fixture(
        include_str!(
            "../../../../../moli-renderer-v8/tests/fixtures/inherited-window-origin-tokens.js"
        ),
        41,
    )
    .await;
}
