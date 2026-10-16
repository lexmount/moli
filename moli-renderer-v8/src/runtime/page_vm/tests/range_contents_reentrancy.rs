use super::blob_support::run_blob_fixture;

#[tokio::test(flavor = "current_thread")]
async fn range_extraction_preserves_original_contents_during_reentrant_unload() {
    run_blob_fixture(
        include_str!("../../../../tests/fixtures/range-contents-reentrancy.js"),
        "JSON.stringify(__blobReadingProgress)",
        "https://range-contents-reentrancy.test/",
        19,
    )
    .await;
}
