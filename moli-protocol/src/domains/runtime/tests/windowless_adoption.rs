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
