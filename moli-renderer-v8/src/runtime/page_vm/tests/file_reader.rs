use super::*;

#[tokio::test(flavor = "current_thread")]
async fn file_reader_bindings_convert_arguments_before_read_state_checks() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (mut page, _resource_source, _owner_wake) =
            page_vm_with_bound_task_sources_and_owner_wake(
                &loader,
                Url::parse("https://file-reader-bindings.test/")?,
            );
        page.vm_mut().eval(include_str!(
            "../../../../tests/fixtures/file-reader-bindings.js"
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
                "FileReader binding fixture did not finish: {}",
                page.vm_mut()
                    .eval("JSON.stringify(__fileReaderBindingProgress)")?
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
        assert_eq!(checks.len(), 280);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("FileReader operations perform WebIDL conversion before read algorithms");
}

#[tokio::test(flavor = "current_thread")]
async fn file_reader_completion_cleans_up_load_before_loadend_or_reentrant_read() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        for owner in ["window", "iframe.contentWindow"] {
            for restart in [false, true] {
                let (mut page, _resource_source, _owner_wake) =
                    page_vm_with_bound_task_sources_and_owner_wake(
                        &loader,
                        Url::parse("https://file-reader-completion-cleanup.test/")?,
                    );
                page.vm_mut().eval(&format!(
                    r#"
globalThis.events = [];
const iframe = document.createElement('iframe');
document.body.appendChild(iframe);
const owner = {owner};
const reader = new owner.FileReader();
reader.onload = () => {{
  const result = reader.result;
  events.push('load:' + result);
  Promise.resolve().then(() => {{
    events.push('microtask:' + result);
    reader.onloadend = () => events.push('end:' + reader.result);
    if ({restart} && result === 'old') reader.readAsText(new owner.Blob(['new']));
  }});
}};
reader.readAsText(new owner.Blob(['old']));
"#
                ))?;
                for _ in 0..3 {
                    assert!(
                        page.run_exact_selected_page_task_for_test(
                            PageSelectedTaskTestSelector::FileReading,
                            &loader,
                        )
                        .await?,
                    );
                }
                let expected = if restart {
                    r#"["load:old","microtask:old"]"#
                } else {
                    r#"["load:old","microtask:old","end:old"]"#
                };
                assert_eq!(
                    page.vm_mut().eval("JSON.stringify(events)")?,
                    expected,
                    "owner={owner}, restart={restart}",
                );
                if restart {
                    for _ in 0..3 {
                        assert!(
                            page.run_exact_selected_page_task_for_test(
                                PageSelectedTaskTestSelector::FileReading,
                                &loader,
                            )
                            .await?,
                        );
                    }
                    assert_eq!(
                        page.vm_mut().eval("JSON.stringify(events)")?,
                        r#"["load:old","microtask:old","load:new","microtask:new","end:new"]"#,
                        "owner={owner}",
                    );
                }
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("load cleanup precedes loadend without another FileReading task");
}

#[tokio::test(flavor = "current_thread")]
async fn file_reader_page_tasks_preserve_microtasks_epochs_realms_and_worker_fifo() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (mut page, _resource_source, _owner_wake) =
            page_vm_with_bound_task_sources_and_owner_wake(
                &loader,
                Url::parse("https://file-reader-task-source.test/")?,
            );
        page.vm_mut().eval(include_str!(
            "../../../../tests/fixtures/file-reader-task-source.js"
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
                "FileReader fixture did not finish: {}",
                page.vm_mut()
                    .eval("JSON.stringify(__fileReaderTaskProgress)")?
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
        assert_eq!(facts["rows"].as_array().unwrap().len(), 21);
        assert_eq!(checks.len(), 138);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("FileReader phases use complete owner tasks and immutable request tickets");
}

#[tokio::test(flavor = "current_thread")]
async fn file_reader_tasks_survive_document_open_in_the_same_window() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        for owner in ["window", "iframe.contentWindow"] {
            let (mut page, _resource_source, _owner_wake) =
                page_vm_with_bound_task_sources_and_owner_wake(
                    &loader,
                    Url::parse("https://file-reader-document-open.test/")?,
                );
            page.vm_mut().eval(&format!(
                r#"
globalThis.events = [];
const iframe = document.createElement('iframe');
document.body.appendChild(iframe);
const owner = {owner};
const reader = new owner.FileReader();
reader.onloadstart = () => events.push('start');
reader.onprogress = () => events.push('progress');
reader.onload = () => events.push('load:' + reader.result);
reader.onloadend = () => events.push('end:' + reader.readyState);
reader.readAsText(new owner.Blob(['read']));
owner.document.open();
owner.document.write('<!doctype html><body>replacement');
owner.document.close();
"#
            ))?;
            for _ in 0..3 {
                assert!(
                    page.run_exact_selected_page_task_for_test(
                        PageSelectedTaskTestSelector::FileReading,
                        &loader,
                    )
                    .await?,
                    "same-Window {owner} FileReader task must survive document.open()",
                );
            }
            assert_eq!(
                page.vm_mut()
                    .eval_without_microtask_checkpoint_for_test("JSON.stringify(events)")?,
                r#"["start","progress","load:read","end:2"]"#,
                "{owner}",
            );
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("document.open preserves Window-owned FileReader phases");
}
