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

#[tokio::test(flavor = "current_thread")]
async fn range_partial_contents_attach_before_reentrant_steps() {
    run_blob_fixture(
        include_str!("../../../../tests/fixtures/range-contents-order.js"),
        "JSON.stringify(__blobReadingProgress)",
        "https://range-contents-order.test/",
        10,
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn range_partial_contents_preserve_recursive_subfragments_and_tree_order() {
    run_blob_fixture(
        include_str!("../../../../tests/fixtures/range-subfragment-order.js"),
        "JSON.stringify(__blobReadingProgress)",
        "https://range-subfragment-order.test/",
        5,
    )
    .await;
}
