use super::*;

use crate::page_task_queue::{
    PageImageLoadEventStalePayloadEffect, PageImageLoadEventTargetEffect,
    RendererPageImageLoadEventTask,
};

fn take_next_image_load_event_task_for_test(
    page_vm: &mut PageVm,
) -> Option<RendererPageImageLoadEventTask> {
    let task = page_vm
        .take_dom_manipulation_body_task_for_test(PageDomManipulationTestFamily::ImageLoadEvent)?;
    let crate::page_task_queue::RendererPageDomManipulationTask::ImageLoadEvent(task) = task else {
        unreachable!("exact image-load selection must preserve its task variant")
    };
    Some(task)
}

#[tokio::test(flavor = "current_thread")]
async fn image_event_body_leaves_reactions_and_runtime_scripts_for_selected_completion() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/image-body-boundary").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
globalThis.__imageTaskBoundary = [];
const image = new Image();
image.addEventListener("load", () => {
  __imageTaskBoundary.push("callback");
  image.decode().then(
    () => __imageTaskBoundary.push("decode"),
    () => __imageTaskBoundary.push("decode-error")
  );
  Promise.resolve().then(() => {
    __imageTaskBoundary.push("microtask");
    const script = document.createElement("script");
    script.textContent = "__imageTaskBoundary.push('runtime-script')";
    document.body.appendChild(script);
  });
});
document.body.appendChild(image);
image.src = "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";
"queued"
"#,
        )?;

        let task = take_next_image_load_event_task_for_test(&mut page_vm)
            .expect("one exact image-load task should be ready");
        let body = page_vm.apply_selected_page_image_load_event_turn(task)?;
        assert_eq!(
            body.action.target_effect,
            PageImageLoadEventTargetEffect::DispatchedToCurrentOwner
        );
        assert_eq!(
            page_vm.vm_mut().eval("__imageTaskBoundary.join('|')")?,
            "callback",
            "the image body must leave listener and image.decode() reactions pending"
        );

        page_vm.finish_selected_page_callback_task(&loader).await?;
        assert_eq!(
            page_vm.vm_mut().eval("__imageTaskBoundary.join('|')")?,
            "callback|microtask|runtime-script|decode",
            "selected image completion must own decode/listener reactions and runtime-script follow-up"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("image body/completion boundary test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn unadmitted_lazy_image_does_not_publish_a_terminal_or_flush_runtime_work() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/unadmitted-lazy-image").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
const spacer = document.createElement("div");
spacer.style.height = "3000px";
document.body.appendChild(spacer);
const image = document.createElement("img");
image.id = "unadmitted-lazy-image";
image.loading = "lazy";
image.src = "data:image/gif;base64,AAAA";
document.body.appendChild(image);
"queued"
"#,
        )?;
        assert!(
            page_vm
                .vm_mut()
                .refresh_layout_snapshot_for_test(moli_layout::LayoutViewport::new(
                    800, 600, 1.0,
                ))?,
            "the test needs one real sampled layout"
        );
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "the setup must leave unrelated runtime work pending"
        );
        assert!(
            take_next_image_load_event_task_for_test(&mut page_vm).is_none(),
            "a far lazy image must not start a request or manufacture a terminal task"
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "lazy admission inspection must not consume unrelated runtime work"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("unadmitted lazy-image boundary test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn image_event_completion_syncs_a_microtask_created_child_after_the_checkpoint() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/image-microtask-child").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
globalThis.__imageChildOrder = [];
const image = new Image();
image.addEventListener("load", () => {
  __imageChildOrder.push("callback");
  Promise.resolve().then(() => {
    __imageChildOrder.push("microtask");
    const frame = document.createElement("iframe");
    frame.id = "image-microtask-child";
    frame.srcdoc = "<!doctype html><body>child</body>";
    document.body.appendChild(frame);
  });
});
document.body.appendChild(image);
image.src = "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";
"queued"
"#,
        )?;

        let task = take_next_image_load_event_task_for_test(&mut page_vm)
            .expect("one exact image-load task should be ready");
        page_vm
            .run_claimed_dom_manipulation_task_through_selected_dispatcher_for_test(
                crate::page_task_queue::RendererPageDomManipulationTask::ImageLoadEvent(task),
                &loader,
            )
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval("__imageChildOrder.join('|')")?,
            "callback|microtask"
        );
        assert!(
            page_vm.vm().has_pending_child_navigation_commit_for_test(),
            "a reaction-created srcdoc frame must publish a typed navigation commit during image task completion"
        );
        assert_eq!(
            page_vm
                .run_next_child_frame_task_source_for_semantic_test()
                .await,
            Some(crate::frame_owner_model::ChildFrameSemanticTurnKind::NavigationCommit)
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("image post-checkpoint child synchronization test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn image_update_discards_a_retired_document_request_before_queuing_its_terminal_task() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/image-document-open").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let before_document = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("initial main Document owner should exist");

        page_vm.vm_mut().eval(
            r#"
globalThis.__documentExactImageEvents = [];
const retiredImage = new Image();
retiredImage.addEventListener("load", () => __documentExactImageEvents.push("retired"));
document.body.appendChild(retiredImage);
retiredImage.src = "/retired-without-network.png";

document.open();
document.write("<!doctype html><body>replacement</body>");
document.close();

const currentImage = new Image();
currentImage.addEventListener("load", () => __documentExactImageEvents.push("current"));
document.body.appendChild(currentImage);
currentImage.src = "/current-without-network.png";
"queued"
"#,
        )?;
        let after_document = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("replacement main Document owner should exist");
        assert_ne!(before_document, after_document);
        assert_eq!(
            before_document.local_window_id,
            after_document.local_window_id
        );
        assert!(
            !page_vm.vm().has_ready_timeout(),
            "image event dispatch must not acquire a PageTimer descriptor"
        );

        let current = take_next_image_load_event_task_for_test(&mut page_vm)
            .expect("replacement image task should survive retired request cancellation");
        page_vm
            .run_claimed_dom_manipulation_task_through_selected_dispatcher_for_test(
                crate::page_task_queue::RendererPageDomManipulationTask::ImageLoadEvent(current),
                &loader,
            )
            .await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__documentExactImageEvents.join('|')")?,
            "current"
        );
        assert!(
            take_next_image_load_event_task_for_test(&mut page_vm).is_none(),
            "the retired request must be cancelled before it manufactures a terminal task"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("Document-exact image event test should run");
}

#[test]
fn image_event_preserves_replacement_payload_across_real_page_vm_replacement() {
    run_page_vm_large_stack_async_test("image-event-page-vm-replacement", || async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/replacement.html",
            "HTTP/1.1 200 OK",
            "<!doctype html><body>replacement</body>".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/initial.html")).unwrap();
        let (page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let local_executor = page_vm.local_executor.clone();

        let task = async move {
            let mut page_vm = page_vm;
            page_vm.vm_mut().eval(
                r#"
const retiredImage = new Image();
document.body.appendChild(retiredImage);
retiredImage.src = "/retired-without-network.png";
"queued-retired"
"#,
            )?;
            let retired_root = page_vm.document_lifecycle.identity().document;

            let replacement_url = format!("{base_url}/replacement.html");
            page_vm.vm_mut().eval(&format!(
                "location.href = {replacement_url:?}; 'navigating'"
            ))?;
            let mut pending_document_lifecycle_turn = None;
            let navigation = page_vm
                .follow_pending_location_navigation_one_turn_async(
                    &mut pending_document_lifecycle_turn,
                    PageVmInitStage::Load,
                )
                .await?;
            assert!(matches!(
                navigation,
                crate::runtime::PageVmFollowNavigationTurnOutcome::Completed
                    | crate::runtime::PageVmFollowNavigationTurnOutcome::PostParseLifecycle { .. }
            ));

            let current_root = page_vm.document_lifecycle.identity().document;
            assert_ne!(retired_root, current_root);
            page_vm.vm_mut().eval(
                r#"
globalThis.__replacementImageEvents = [];
const currentImage = new Image();
currentImage.addEventListener("load", () => {
  __replacementImageEvents.push("load");
});
document.body.appendChild(currentImage);
currentImage.src = "/current-without-network.png";
"queued-current"
"#,
            )?;

            let stale_task = take_next_image_load_event_task_for_test(&mut page_vm)
                .expect("retired PageVm image task should consume one stale turn");
            let stale = page_vm.apply_selected_page_image_load_event_turn(stale_task)?;
            let PageImageLoadEventTargetEffect::DiscardedStaleOwner {
                current_owner: None,
                stale_payload_effect:
                    PageImageLoadEventStalePayloadEffect::ForeignPageVmStatePreserved,
            } = stale.action.target_effect
            else {
                panic!(
                    "retired image task returned an unexpected effect: {:?}",
                    stale.action.target_effect
                );
            };

            let current = take_next_image_load_event_task_for_test(&mut page_vm)
                .expect("replacement image task must survive stale-head settlement");
            assert_ne!(
                stale.action.owner.root_document(),
                current.owner().root_document(),
                "retired and replacement image tasks must retain distinct PageVm namespaces"
            );
            page_vm
                .run_claimed_dom_manipulation_task_through_selected_dispatcher_for_test(
                    crate::page_task_queue::RendererPageDomManipulationTask::ImageLoadEvent(
                        current,
                    ),
                    &loader,
                )
                .await?;
            assert_eq!(
                page_vm
                    .vm_mut()
                    .eval("__replacementImageEvents.join('|')")?,
                "load",
                "retired root task must not consume the replacement Host payload"
            );
            assert!(!page_vm.vm().has_ready_timeout());
            Ok::<_, anyhow::Error>(())
        };
        let outcome = local_executor.run(task).await;
        outcome.expect("image replacement should use exact root arbitration");
        server
            .await
            .expect("image replacement server should finish");
    });
}
