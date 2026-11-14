use super::*;
use moli_file_api::{BlobData, BlobReadCompletion, BlobSource};
use parking_lot::Mutex;

#[derive(Default)]
struct DeferredBlobSource {
    completion: Mutex<Option<BlobReadCompletion>>,
}

impl std::fmt::Debug for DeferredBlobSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DeferredBlobSource")
    }
}

impl BlobSource for DeferredBlobSource {
    fn read(&self, range: std::ops::Range<usize>, complete: BlobReadCompletion) {
        assert_eq!(range, 0..7);
        assert!(self.completion.lock().replace(complete).is_none());
    }
}

impl DeferredBlobSource {
    fn install(page: &mut PageVm) -> anyhow::Result<Arc<Self>> {
        let source = Arc::new(Self::default());
        let data = BlobData::from_source(source.clone(), 7);
        page.vm_mut()
            .with_default_context_scope_for_test(|scope, _| {
                let blob = crate::blob::build_blob_object_from_data(scope, data, String::new())
                    .expect("native source-backed Blob");
                let global = scope.get_current_context().global(scope);
                assert_eq!(
                    global.set(
                        scope,
                        crate::util::v8str(scope, "delayedBlob").into(),
                        blob.into()
                    ),
                    Some(true),
                );
                Ok(())
            })?;
        Ok(source)
    }

    fn complete(&self) {
        let complete = self.completion.lock().take().expect("accepted read");
        complete(Ok(b"payload".to_vec()));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn idle_waits_for_backend_blob_reads_before_their_callbacks_are_ready() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        for read in [
            "delayedBlob.text().then(finish)",
            "delayedBlob.arrayBuffer().then(value => finish(new TextDecoder().decode(value)))",
            "delayedBlob.stream().getReader().read().then(({value}) => finish(new TextDecoder().decode(value)))",
            "const reader = new FileReader(); reader.onloadend = () => finish(reader.result); reader.readAsText(delayedBlob)",
        ] {
            let (mut page, _resource_source, _owner_wake) =
                page_vm_with_bound_task_sources_and_owner_wake(
                    &loader,
                    Url::parse("https://idle-blob-io.test/")?,
                );
            let source = DeferredBlobSource::install(&mut page)?;
            page.vm_mut().eval(&format!(
                "globalThis.events = []; function finish(text) {{ events.push(text); queueMicrotask(() => events.push('microtask')); }}; {read}; 'queued'"
            ))?;
            // FileReader has a loadstart task before it starts backend I/O.
            while source.completion.lock().is_none() {
                assert!(
                    page.run_exact_selected_page_task_for_test(
                        PageSelectedTaskTestSelector::FileReading,
                        &loader,
                    )
                    .await?
                );
            }
            while page
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::FileReading,
                    &loader,
                )
                .await?
            {}
            assert!(
                page.claim_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::FileReading,
                )
                .is_none(),
                "the backend has not published a completion yet"
            );
            assert!(matches!(
                page.advance_idle_wait_turn(
                    PageVmIdleWaitState::EventLoop,
                    Duration::from_secs(1),
                )
                .await?,
                PageVmIdleWaitAdvance::Waiting { .. }
            ));
            assert_eq!(page.vm_mut().eval("JSON.stringify(events)")?, "[]");

            source.complete();
            while page
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::FileReading,
                    &loader,
                )
                .await?
            {}
            assert_eq!(
                page.vm_mut().eval_without_microtask_checkpoint_for_test(
                    "JSON.stringify(events)"
                )?,
                r#"["payload","microtask"]"#
            );
            assert!(matches!(
                page.advance_idle_wait_turn(
                    PageVmIdleWaitState::EventLoop,
                    Duration::from_secs(1),
                )
                .await?,
                PageVmIdleWaitAdvance::Completed
            ));
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("backend Blob reads keep idle pending through completion and microtasks");
}

#[tokio::test(flavor = "current_thread")]
async fn idle_ignores_backend_blob_reads_owned_by_a_retired_window() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page, _resource_source, _owner_wake) =
            page_vm_with_bound_task_sources_and_owner_wake(
                &loader,
                Url::parse("https://idle-retired-blob.test/")?,
            );
        let source = DeferredBlobSource::install(&mut page)?;
        page.vm_mut().eval(
            r#"
            globalThis.settled = false;
            const frame = document.createElement('iframe'); document.body.append(frame);
            new frame.contentWindow.Blob([delayedBlob]).text().then(() => settled = true);
            frame.remove();
            'queued'
        "#,
        )?;
        assert!(source.completion.lock().is_some());
        assert!(matches!(
            page.advance_idle_wait_turn(PageVmIdleWaitState::EventLoop, Duration::from_secs(1),)
                .await?,
            PageVmIdleWaitAdvance::Completed
        ));
        source.complete();
        while page
            .run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::FileReading,
                &loader,
            )
            .await?
        {}
        assert_eq!(page.vm_mut().eval("String(settled)")?, "false");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("retired Window Blob reads do not retain idle or settle callbacks");
}
