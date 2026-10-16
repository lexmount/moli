use super::blob_support::run_blob_fixture;

#[tokio::test(flavor = "current_thread")]
async fn blob_text_decodes_utf8_and_removes_only_the_initial_utf8_bom() {
    run_blob_fixture(
        include_str!("../../../../tests/fixtures/blob-text-utf8.js"),
        "JSON.stringify(__blobTextProgress)",
        "https://blob-text-utf8.test/",
        240,
    )
    .await;
}
