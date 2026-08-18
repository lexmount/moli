use super::*;

use crate::{
    page_task_queue::{
        PageServiceWorkerClientMessageTargetEffect, PageServiceWorkerInternalTargetEffect,
        RendererOwnerWakeSource, RendererServiceWorkerInternalTaskKind,
    },
    runtime::{IntoPageTaskCompletion, PageTaskCompletion},
    service_worker_runtime::{ServiceWorkerClientId, ServiceWorkerVersionId},
    structured_clone::V8StructuredClonePayload,
    types::{
        ServiceWorkerClientMessageCompletion, ServiceWorkerUnregisterCompletion,
        ServiceWorkerWindowClientTarget,
    },
};

fn service_worker_message(
    target: ServiceWorkerWindowClientTarget,
) -> ServiceWorkerClientMessageCompletion {
    ServiceWorkerClientMessageCompletion {
        target,
        source_version_id: ServiceWorkerVersionId::from_u64_for_test(43),
        source_script_url: Url::parse("https://service-worker-page-source.test/worker.js")
            .expect("worker URL"),
        source_state: "activated",
        payload: V8StructuredClonePayload::default(),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn service_worker_internal_body_authorizes_the_exact_root_document() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://service-worker-page-source.test/current").unwrap();
        let (mut page_vm, _resource_source, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let current_root = page_vm.document_lifecycle.identity().document;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();

        page_vm
            .service_worker_task_sender_for_root_for_test(current_root)
            .send_service_worker_unregister(ServiceWorkerUnregisterCompletion {
                request_id: 47,
                document_owner: crate::native_bridge::WindowDocumentOwner::for_test(53),
                result: false,
            })
            .expect("current-root callback should enter the stable source");
        assert_eq!(
            wake_rx
                .recv()
                .await
                .expect("internal callback wake")
                .source_for_test(),
            RendererOwnerWakeSource::ServiceWorkerInternalTask
        );
        let current_task = page_vm
            .take_service_worker_internal_body_task_for_test()
            .expect("current-root callback should consume one typed turn");
        let current = page_vm.apply_selected_page_service_worker_internal_turn(current_task)?;
        assert_eq!(current.action.root_document, current_root);
        assert_eq!(
            current.action.task_kind,
            RendererServiceWorkerInternalTaskKind::Unregister
        );
        assert_eq!(
            current.action.target_effect,
            PageServiceWorkerInternalTargetEffect::CurrentRootTaskHadNoExactTarget
        );
        let current_completion = current.action.into_page_task_completion();
        assert!(matches!(
            current_completion,
            PageTaskCompletion::NoCompletion
        ));
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "a missing exact request must not manufacture a checkpoint or runtime follow-up"
        );

        // This test intentionally stops at the body/settlement boundary. The
        // typed NoCompletion mapping above proves that the selected dispatcher
        // has no task-end work to apply.
        let stale_root = current_root.successor_for_testing();
        page_vm
            .service_worker_task_sender_for_root_for_test(stale_root)
            .send_service_worker_unregister(ServiceWorkerUnregisterCompletion {
                request_id: 59,
                document_owner: crate::native_bridge::WindowDocumentOwner::for_test(61),
                result: true,
            })
            .expect("stale callback should still enter one discard turn");
        assert_eq!(
            wake_rx
                .recv()
                .await
                .expect("stale callback wake")
                .source_for_test(),
            RendererOwnerWakeSource::ServiceWorkerInternalTask
        );
        let stale_task = page_vm
            .take_service_worker_internal_body_task_for_test()
            .expect("stale callback should consume one typed discard turn");
        let stale = page_vm.apply_selected_page_service_worker_internal_turn(stale_task)?;
        assert_eq!(stale.action.root_document, stale_root);
        assert_eq!(
            stale.action.target_effect,
            PageServiceWorkerInternalTargetEffect::DiscardedStaleRoot { current_root }
        );
        let stale_completion = stale.action.into_page_task_completion();
        assert!(matches!(stale_completion, PageTaskCompletion::NoCompletion));
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "a stale root must not enter replacement V8 or advance its runtime residence"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("ServiceWorker internal exact-root turns should complete");
}

#[tokio::test(flavor = "current_thread")]
async fn service_worker_client_message_authorizes_root_before_window_client_target() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://service-worker-message-source.test/current").unwrap();
        let (mut page_vm, _resource_source, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let current_root = page_vm.document_lifecycle.identity().document;
        let stale_target = ServiceWorkerWindowClientTarget {
            client_id: ServiceWorkerClientId::from_u64_for_test(u64::MAX - 7),
            document_owner: crate::native_bridge::WindowDocumentOwner::for_test(u64::MAX - 5),
        };

        page_vm
            .service_worker_task_sender_for_root_for_test(current_root)
            .send_service_worker_client_message(service_worker_message(stale_target))
            .expect("current-root message should enter the stable source");
        assert_eq!(
            wake_rx
                .recv()
                .await
                .expect("client-message wake")
                .source_for_test(),
            RendererOwnerWakeSource::ServiceWorkerClientMessage
        );
        let stale_client_task = page_vm
            .take_service_worker_client_message_body_task_for_test()
            .expect("stale client target should consume one typed discard turn");
        let stale_client =
            page_vm.apply_selected_page_service_worker_client_message_turn(stale_client_task)?;
        assert_eq!(stale_client.action.owner.root_document(), current_root);
        assert_eq!(stale_client.action.owner.target(), stale_target);
        assert_eq!(
            stale_client.action.target_effect,
            PageServiceWorkerClientMessageTargetEffect::DiscardedStaleTarget
        );
        let stale_root = current_root.successor_for_testing();
        page_vm
            .service_worker_task_sender_for_root_for_test(stale_root)
            .send_service_worker_client_message(service_worker_message(stale_target))
            .expect("old-root message should still enter one discard turn");
        assert_eq!(
            wake_rx
                .recv()
                .await
                .expect("old-root message wake")
                .source_for_test(),
            RendererOwnerWakeSource::ServiceWorkerClientMessage
        );
        let stale_document_task = page_vm
            .take_service_worker_client_message_body_task_for_test()
            .expect("old-root message should consume one typed discard turn");
        let stale_document =
            page_vm.apply_selected_page_service_worker_client_message_turn(stale_document_task)?;
        assert_eq!(stale_document.action.owner.root_document(), stale_root);
        assert_eq!(
            stale_document.action.target_effect,
            PageServiceWorkerClientMessageTargetEffect::DiscardedStaleRoot { current_root }
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("ServiceWorker client-message authorization turns should complete");
}
