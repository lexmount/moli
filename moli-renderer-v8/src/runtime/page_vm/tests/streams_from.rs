use super::blob_support::run_blob_fixture;

#[tokio::test(flavor = "current_thread")]
async fn readable_stream_from_preserves_sync_protocol_across_realms_and_workers() {
    run_blob_fixture(
        include_str!("../../../../tests/fixtures/streams-sync-iterator.js"),
        "JSON.stringify(__blobReadingProgress)",
        "https://streams-sync-iterator.test/",
        60,
    )
    .await;
}
