use super::*;

#[tokio::test(flavor = "current_thread")]
async fn blob_slice_boundaries_do_not_overflow_in_window_or_worker() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (mut page, _resource_source, _owner_wake) =
            page_vm_with_bound_task_sources_and_owner_wake(
                &loader,
                Url::parse("https://blob-slice-boundaries.test/")?,
            );
        page.vm_mut().eval(include_str!(
            "../../../../tests/fixtures/blob-slice-boundaries.js"
        ))?;
        let deadline = Instant::now() + Duration::from_secs(20);
        while page.vm_mut().eval("typeof __uiEventResults")? == "undefined" {
            let _ = page
                .run_next_child_frame_task_source_for_semantic_test()
                .await;
            for selector in [
                PageSelectedTaskTestSelector::FileReading,
                PageSelectedTaskTestSelector::ResourceCompletion,
                PageSelectedTaskTestSelector::WorkerHostBridge,
                PageSelectedTaskTestSelector::DedicatedWorkerClientEvent,
            ] {
                let _ = page
                    .run_exact_selected_page_task_for_test(selector, &loader)
                    .await?;
            }
            anyhow::ensure!(
                Instant::now() < deadline,
                "Blob slice fixture did not finish: {}",
                page.vm_mut().eval("JSON.stringify(__blobSliceProgress)")?
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
        assert_eq!(checks.len(), 360);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("Blob slice handles signed limits and clamped indices in every realm");
}
