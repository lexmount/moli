use super::*;

#[tokio::test(flavor = "current_thread")]
async fn blob_and_file_constructors_convert_before_processing_bytes() {
    run_blob_constructor_fixture(
        include_str!("../../../../tests/fixtures/blob-constructor-conversion.js"),
        184,
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn blob_and_file_constructors_copy_buffers_after_later_detachment() {
    run_blob_constructor_fixture(
        include_str!("../../../../tests/fixtures/blob-constructor-detachment.js"),
        72,
    )
    .await;
}

async fn run_blob_constructor_fixture(fixture: &'static str, expected_checks: usize) {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (mut page, _resource_source, _owner_wake) =
            page_vm_with_bound_task_sources_and_owner_wake(
                &loader,
                Url::parse("https://blob-constructor-conversion.test/")?,
            );
        page.vm_mut().eval(fixture)?;
        let deadline = Instant::now() + Duration::from_secs(20);
        while page.vm_mut().eval("typeof __uiEventResults")? == "undefined" {
            let _ = page
                .run_next_child_frame_task_source_for_semantic_test()
                .await;
            for selector in [
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
                "Blob constructor fixture did not finish: {}",
                page.vm_mut()
                    .eval("JSON.stringify(__blobConstructorProgress)")?
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
    .expect("Blob/File conversion preserves argument order and deferred buffer copying");
}
