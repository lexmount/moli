use super::*;

#[tokio::test(flavor = "current_thread")]
async fn range_extraction_preserves_original_contents_during_navigation_abort() {
    run_range_fixture(
        include_str!("../../../../tests/fixtures/range-contents-reentrancy.js"),
        "JSON.stringify(__blobReadingProgress)",
        "https://range-contents-reentrancy.test/",
        19,
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn range_partial_contents_attach_before_reentrant_steps() {
    run_range_fixture(
        include_str!("../../../../tests/fixtures/range-contents-order.js"),
        "JSON.stringify(__blobReadingProgress)",
        "https://range-contents-order.test/",
        10,
    )
    .await;
}

async fn run_range_fixture(
    fixture: &'static str,
    progress_expression: &'static str,
    url: &'static str,
    expected_checks: usize,
) {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (mut page, _resource_source, _owner_wake) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, Url::parse(url)?);
        page.vm_mut().eval(fixture)?;
        let deadline = Instant::now() + Duration::from_secs(20);
        while page.vm_mut().eval("typeof __uiEventResults")? == "undefined" {
            let _ = page
                .run_next_child_frame_task_source_for_semantic_test()
                .await;
            let _ = page
                .run_one_oldest_ready_page_task_on_owner_lane_for_test(&loader)
                .await?;
            anyhow::ensure!(
                Instant::now() < deadline,
                "Range fixture did not finish: {}",
                page.vm_mut().eval(progress_expression)?
            );
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        let facts: serde_json::Value =
            serde_json::from_str(&page.vm_mut().eval("JSON.stringify(__uiEventResults)")?)?;
        let checks = facts["checks"].as_array().unwrap();
        let failures = checks
            .iter()
            .filter(|check| check["passed"] != true)
            .collect::<Vec<_>>();
        assert!(failures.is_empty(), "{failures:#?}");
        assert_eq!(checks.len(), expected_checks);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("Range fixture should preserve all specified checks");
}
