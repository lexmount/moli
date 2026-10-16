use super::blob_support::run_blob_fixture;
use super::*;

#[tokio::test(flavor = "current_thread")]
async fn worker_fetch_blob_body_streams_preserve_file_reading_clone_and_byob() {
    run_blob_fixture(
        include_str!("../../../../tests/fixtures/fetch-blob-reading-worker.js"),
        "JSON.stringify(__blobReadingProgress)",
        "https://worker-fetch-blob-reading.test/",
        76,
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn fetch_blob_body_streams_preserve_tasks_clone_byob_cancel_and_transfer() {
    run_blob_fixture(
        include_str!("../../../../tests/fixtures/fetch-blob-reading-contract.js"),
        "JSON.stringify(__blobReadingProgress)",
        "https://fetch-blob-reading-contract.test/",
        856,
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn fetch_blob_body_waits_for_file_reading_before_networking_completion() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (mut page, _resource_source, _owner_wake) =
            page_vm_with_bound_task_sources_and_owner_wake(
                &loader,
                Url::parse("https://fetch-blob-task-sources.test/")?,
            );
        page.vm_mut().eval(
            r#"
globalThis.events = [];
const fileReader = new FileReader();
fileReader.onloadstart = () => events.push('reader-start');
fileReader.readAsText(new Blob(['reader']));
new Response(new Blob(['response'])).body.getReader().read()
  .then(({value}) => events.push(new TextDecoder().decode(value)));
new Request(location.href, {method: 'POST', body: new Blob(['request'])})
  .body.getReader().read().then(({value}) => events.push(new TextDecoder().decode(value)));
new Response(new Blob(['text'])).text().then(value => events.push(value));
"#,
        )?;
        assert_eq!(page.vm_mut().eval("JSON.stringify(events)")?, "[]");
        assert!(
            page.due_page_timer_ready_descriptor(
                crate::page_task_queue::RendererPageTimerSelection::AnyReady,
            )
            .is_none()
        );
        assert!(
            !page
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ResourceCompletion,
                    &loader,
                )
                .await?
        );
        for expected in [
            r#"["reader-start"]"#,
            r#"["reader-start","response"]"#,
            r#"["reader-start","response","request"]"#,
            r#"["reader-start","response","request"]"#,
        ] {
            assert!(
                page.run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::FileReading,
                    &loader,
                )
                .await?
            );
            assert_eq!(
                page.vm_mut()
                    .eval_without_microtask_checkpoint_for_test("JSON.stringify(events)")?,
                expected,
            );
        }
        let Some(crate::page_task_queue::RendererPageReadyDescriptor::Timer {
            deadline,
            selection,
        }) = page.due_page_timer_ready_descriptor(
            crate::page_task_queue::RendererPageTimerSelection::AnyReady,
        )
        else {
            anyhow::bail!("fully-read must enqueue its native Networking callback");
        };
        page.apply_selected_page_scheduler_task_on_owner_lane_for_test(
            crate::page_task_queue::RendererPageSchedulerTask::Timer {
                deadline,
                selection,
            },
            loader.clone(),
        )
        .await?;
        assert_eq!(
            page.vm_mut()
                .eval_without_microtask_checkpoint_for_test("JSON.stringify(events)")?,
            r#"["reader-start","response","request","text"]"#,
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("constructed Blob bodies use FileReading, then public Body uses Networking");
}

#[tokio::test(flavor = "current_thread")]
async fn fetch_blob_body_stream_realm_and_lifetime_belong_to_blob() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (mut page, _resource_source, _owner_wake) =
            page_vm_with_bound_task_sources_and_owner_wake(
                &loader,
                Url::parse("https://fetch-blob-relevant-global.test/")?,
            );
        page.vm_mut().eval(
            r#"
globalThis.events = [];
const iframe = document.createElement('iframe');
document.body.appendChild(iframe);
const child = iframe.contentWindow;
const retired = new Response(new child.Blob(['retired']));
const live = new child.Response(new Blob(['live']));
globalThis.correctRealms = Object.getPrototypeOf(retired.body) === child.ReadableStream.prototype
  && Object.getPrototypeOf(live.body) === ReadableStream.prototype;
retired.body.getReader().read().then(() => events.push('retired'));
live.body.getReader().read().then(({value}) => events.push(new TextDecoder().decode(value)));
iframe.remove();
"#,
        )?;
        assert_eq!(page.vm_mut().eval("correctRealms")?, "true");
        assert_eq!(page.vm_mut().eval("JSON.stringify(events)")?, "[]");
        for _ in 0..3 {
            let _ = page
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::FileReading,
                    &loader,
                )
                .await?;
        }
        assert_eq!(
            page.vm_mut()
                .eval_without_microtask_checkpoint_for_test("JSON.stringify(events)")?,
            r#"["live"]"#,
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("Blob body get-stream uses the Blob relevant realm and execution context");
}

#[tokio::test(flavor = "current_thread")]
async fn fetch_blob_body_file_reading_survives_same_window_document_open() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        for owner in ["window", "iframe.contentWindow"] {
            let (mut page, _resource_source, _owner_wake) =
                page_vm_with_bound_task_sources_and_owner_wake(
                    &loader,
                    Url::parse("https://fetch-blob-document-open.test/")?,
                );
            page.vm_mut().eval(&format!(
                r#"
globalThis.events = [];
const iframe = document.createElement('iframe');
document.body.appendChild(iframe);
const owner = {owner};
const response = new Response(new owner.Blob(['response']));
const request = new Request(location.href, {{method: 'POST', body: new owner.Blob(['request'])}});
for (const object of [response, request]) object.body.getReader().read()
  .then(({{value}}) => events.push(new TextDecoder().decode(value)));
owner.document.open();
owner.document.write('<!doctype html><body>replacement');
owner.document.close();
"#,
            ))?;
            for _ in 0..2 {
                assert!(
                    page.run_exact_selected_page_task_for_test(
                        PageSelectedTaskTestSelector::FileReading,
                        &loader,
                    )
                    .await?
                );
            }
            assert_eq!(
                page.vm_mut()
                    .eval_without_microtask_checkpoint_for_test("JSON.stringify(events)")?,
                r#"["response","request"]"#,
            );
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("document.open preserves constructed body file-reading tasks");
}
