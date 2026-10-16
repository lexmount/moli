use super::blob_support::run_blob_fixture;

#[tokio::test(flavor = "current_thread")]
async fn blob_slice_boundaries_do_not_overflow_in_window_or_worker() {
    run_blob_fixture(
        include_str!("../../../../tests/fixtures/blob-slice-boundaries.js"),
        "JSON.stringify(__blobSliceProgress)",
        "https://blob-slice-boundaries.test/",
        360,
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn blob_slice_conversion_stops_at_the_first_exception_in_window_or_worker() {
    run_blob_fixture(
        include_str!("../../../../tests/fixtures/blob-slice-conversion.js"),
        "JSON.stringify(__blobSliceConversionProgress)",
        "https://blob-slice-boundaries.test/",
        340,
    )
    .await;
}
