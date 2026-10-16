use super::blob_support::run_blob_fixture;
use super::*;

#[tokio::test(flavor = "current_thread")]
async fn blob_reading_waits_for_file_reading_tasks_in_window_and_worker() {
    run_blob_fixture(
        include_str!("../../../../tests/fixtures/blob-read-scheduling.js"),
        "JSON.stringify(__blobReadProgress)",
        "https://blob-read-scheduling.test/",
        40,
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn blob_reading_preserves_realms_byob_cancellation_and_native_construction() {
    run_blob_fixture(
        include_str!("../../../../tests/fixtures/blob-reading-contract.js"),
        "JSON.stringify(__blobReadingProgress)",
        "https://blob-reading-contract.test/",
        236,
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn worker_blob_reads_share_file_reader_fifo_and_task_checkpoints() {
    run_blob_fixture(
        include_str!("../../../../tests/fixtures/blob-reading-worker-fifo.js"),
        "JSON.stringify(__blobReadingProgress)",
        "https://blob-worker-file-reading-fifo.test/",
        12,
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn blob_file_reading_tasks_share_fifo_and_checkpoint_with_file_reader() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (mut page, _resource_source, _owner_wake) =
            page_vm_with_bound_task_sources_and_owner_wake(
                &loader,
                Url::parse("https://blob-file-reading-fifo.test/")?,
            );
        page.vm_mut().eval(
            r#"
globalThis.events = [];
const reader = new FileReader();
reader.onloadstart = () => {
  events.push('start');
  queueMicrotask(() => events.push('start-microtask'));
};
reader.onprogress = () => events.push('progress');
reader.onload = () => events.push('load');
reader.readAsText(new Blob(['reader']));
new Blob(['blob']).text().then(text => {
  events.push(text);
  Promise.resolve().then(() => events.push('blob-microtask'));
});
new Blob(['bytes']).stream().getReader().read().then(({value}) => {
  events.push(new TextDecoder().decode(value));
});
"#,
        )?;
        assert_eq!(page.vm_mut().eval("JSON.stringify(events)")?, "[]");
        for expected in [
            r#"["start","start-microtask"]"#,
            r#"["start","start-microtask","blob","blob-microtask"]"#,
            r#"["start","start-microtask","blob","blob-microtask","bytes"]"#,
            r#"["start","start-microtask","blob","blob-microtask","bytes","progress"]"#,
            r#"["start","start-microtask","blob","blob-microtask","bytes","progress","load"]"#,
        ] {
            assert!(
                page.run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::FileReading,
                    &loader,
                )
                .await?
            );
            // Observe the selected task's own checkpoint; evaluation must not
            // supply an extra checkpoint to finish its Promise reactions.
            assert_eq!(
                page.vm_mut()
                    .eval_without_microtask_checkpoint_for_test("JSON.stringify(events)")?,
                expected,
            );
        }
        assert!(
            !page
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::FileReading,
                    &loader,
                )
                .await?
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("Blob and FileReader completions share real file-reading tasks");
}

#[tokio::test(flavor = "current_thread")]
async fn blob_and_file_reader_tasks_survive_document_open_in_the_same_window() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        for owner in ["window", "iframe.contentWindow"] {
            let (mut page, _resource_source, _owner_wake) =
                page_vm_with_bound_task_sources_and_owner_wake(
                    &loader,
                    Url::parse("https://blob-document-open.test/")?,
                );
            page.vm_mut().eval(&format!(
                r#"
globalThis.outcomes = [];
const iframe = document.createElement('iframe');
document.body.appendChild(iframe);
const owner = {owner};
const blob = new owner.Blob(['read']);
for (const name of ['text', 'arrayBuffer', 'bytes', 'stream']) {{
  const value = Blob.prototype[name].call(blob);
  const promise = name === 'stream' ? value.getReader().read() : value;
  promise.then(result => {{
    const bytes = name === 'stream' ? result.value : result;
    const text = name === 'text' ? bytes : new TextDecoder().decode(bytes);
    outcomes.push([name, text === 'read',
      name === 'stream' ? value instanceof owner.ReadableStream : value instanceof owner.Promise]);
  }});
}}
const reader = new owner.FileReader();
reader.onload = () => outcomes.push(['FileReader', reader.result === 'read', reader.readyState === 2]);
reader.readAsText(blob);
owner.document.open();
owner.document.write('<!doctype html><body>replacement');
owner.document.close();
"#
            ))?;
            for _ in 0..7 {
                assert!(
                    page.run_exact_selected_page_task_for_test(
                        PageSelectedTaskTestSelector::FileReading,
                        &loader,
                    )
                    .await?,
                    "same-Window {owner} file-reading task must survive document.open()",
                );
            }
            assert_eq!(
                page.vm_mut().eval_without_microtask_checkpoint_for_test(
                    "JSON.stringify(outcomes)"
                )?,
                r#"[["text",true,true],["arrayBuffer",true,true],["bytes",true,true],["stream",true,true],["FileReader",true,true]]"#,
            );
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("document.open preserves the Window-owned file-reading source");
}

#[tokio::test(flavor = "current_thread")]
async fn retired_blob_owner_does_not_settle_or_checkpoint_a_live_borrowed_read() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (mut page, _resource_source, _owner_wake) =
            page_vm_with_bound_task_sources_and_owner_wake(
                &loader,
                Url::parse("https://blob-retired-owner.test/")?,
            );
        page.vm_mut().eval(
            r#"
globalThis.outcomes = [];
const iframe = document.createElement('iframe');
document.body.appendChild(iframe);
const child = iframe.contentWindow;
new child.Blob(['retired']).text().then(text => outcomes.push(text));
child.Blob.prototype.text.call(new Blob(['live'])).then(text => outcomes.push(text));
iframe.remove();
"#,
        )?;
        page.vm_mut().eval_without_microtask_checkpoint_for_test(
            "queueMicrotask(() => outcomes.push('checkpoint')); 'queued'",
        )?;
        assert!(
            page.run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::FileReading,
                &loader,
            )
            .await?
        );
        assert_eq!(
            page.vm_mut()
                .eval_without_microtask_checkpoint_for_test("JSON.stringify(outcomes)")?,
            "[]",
            "a retired child's read must not checkpoint its live parent",
        );
        assert!(
            page.run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::FileReading,
                &loader,
            )
            .await?
        );
        assert_eq!(
            page.vm_mut()
                .eval_without_microtask_checkpoint_for_test("JSON.stringify(outcomes)")?,
            r#"["checkpoint","live"]"#,
            "the borrowing method's removed realm must not retire the live Blob's task",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("Blob completion uses its exact Window owner");
}
