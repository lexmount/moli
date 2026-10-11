use super::page_fixture::assert_page_fixture;

#[tokio::test(flavor = "multi_thread")]
async fn uncompiled_handler_sources_survive_real_page_adoption_with_utf16_intact() {
    assert_page_fixture(
        include_str!(
            "../../../../../moli-renderer-v8/tests/fixtures/uncompiled-event-handler-sources.js"
        ),
        147,
    )
    .await;
}
