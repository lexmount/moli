use super::*;

async fn assert_native_drag_default_payload(in_child: bool) {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for pointer in ["mouse", "pen"] {
        for mode in [
            "link-relative",
            "link-absolute",
            "link-empty",
            "link-fragment",
            "link-unicode",
            "link-base",
            "link-down-mutation",
            "link-start-mutation",
            "link-override",
            "link-clear",
            "link-poison",
            "link-invalid",
            "link-nohref",
            "image",
            "image-srcset",
            "image-linked",
            "image-start-mutation",
            "image-clear",
            "image-poison",
            "container",
            "plain",
        ] {
            let mut vm = new_storage_page_task_executor_test_vm_with_loader(
                "https://drag-default-payload.test/audit.html",
                &loader,
            );
            vm.eval("if (!document.documentElement) document.appendChild(document.createElement('html')); if (!document.head) document.documentElement.appendChild(document.createElement('head')); if (!document.body) document.documentElement.appendChild(document.createElement('body')); 'ready'").unwrap();
            vm.eval(&format!(
                "{}; globalThis.__payloadResults=__installDragDefaultPayloadProbe('{mode}',{in_child}); 'ready'",
                include_str!("drag_default_payload.js")
            )).unwrap();
            vm.publish_layout_for_test().unwrap();
            for (phase, event, x, buttons) in [
                ("hover", "mousemove", 80.0, 0),
                ("down", "mousedown", 80.0, 1),
                ("start", "mousemove", 100.0, 1),
                ("target-enter", "mousemove", 310.0, 1),
                ("target", "mousemove", 320.0, 1),
                ("target-over", "mousemove", 330.0, 1),
                ("up", "mouseup", 330.0, 0),
            ] {
                vm.eval(&format!("globalThis.__dragPhase='{phase}'; 'ready'"))
                    .unwrap();
                vm.dispatch_mouse_event_at_point_with_pointer_and_modifiers(
                    x,
                    80.0,
                    event,
                    if event == "mousemove" { -1 } else { 0 },
                    Some(buttons),
                    i32::from(event != "mousemove"),
                    0.0,
                    0.0,
                    crate::runtime::RendererPointerEventProperties {
                        pointer_id: 17,
                        pointer_type: pointer.to_owned(),
                        pressure: if buttons == 0 { 0.0 } else { 0.5 },
                        ..Default::default()
                    },
                    0,
                )
                .unwrap();
            }
            let mut tasks = 0;
            while vm
                .run_one_user_interaction_executor_turn(&loader)
                .await
                .unwrap()
            {
                tasks += 1;
                assert!(tasks <= 1, "only the dragstart URI item queues a callback");
            }
            assert_eq!(
                vm.eval("globalThis.__payloadFinished=__payloadResults.finish(); __payloadFinished.complete")
                    .unwrap(),
                "true",
                "{pointer}, {mode}, child {in_child}: {}",
                vm.eval("JSON.stringify(__payloadFinished.checks.filter(c=>!c.pass))")
                    .unwrap()
            );
        }
    }
}

#[tokio::test]
async fn native_drag_default_urls_are_snapshotted_before_dragstart() {
    assert_native_drag_default_payload(false).await;
}

#[tokio::test]
async fn native_drag_default_urls_use_the_source_documents_base_and_realm() {
    assert_native_drag_default_payload(true).await;
}
