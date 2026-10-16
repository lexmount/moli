use super::blob_support::run_blob_fixture;

#[tokio::test(flavor = "current_thread")]
async fn blob_and_file_constructors_convert_before_processing_bytes() {
    run_blob_fixture(
        include_str!("../../../../tests/fixtures/blob-constructor-conversion.js"),
        "JSON.stringify(__blobConstructorProgress)",
        "https://blob-constructor-conversion.test/",
        184,
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn blob_and_file_constructors_copy_buffers_after_later_detachment() {
    run_blob_fixture(
        include_str!("../../../../tests/fixtures/blob-constructor-detachment.js"),
        "JSON.stringify(__blobConstructorProgress)",
        "https://blob-constructor-conversion.test/",
        72,
    )
    .await;
}
