use super::*;
use crate::frame_owner_model::ChildFrameSemanticTurnKind;

#[tokio::test]
async fn isolated_event_default_view_uses_its_construction_window() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://event-worlds.test/", &loader);
    vm.eval(
        "document.body.innerHTML = '<iframe></iframe>'; \
         const frame = document.querySelector('iframe'); \
         frame.srcdoc = '<button id=target></button>'; void frame.contentWindow; 'ready'",
    )
    .unwrap();
    vm.run_one_child_frame_task_executor_turn(
        ChildFrameSemanticTurnKind::RealmMaterialization,
        &loader,
    )
    .await
    .unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .unwrap();
    let child = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .next()
        .unwrap()
        .context_id;
    vm.eval_in_child_default_context(
        child,
        r#"
        document.getElementById('target').addEventListener('construction', function(event) {
            globalThis.constructionFacts = [
                event instanceof parent.Event, !(event instanceof Event),
                event.createdIn === undefined, event.target === this,
                event.currentTarget === this, this === document.getElementById('target')
            ];
        }); 'ready'
        "#,
    )
    .unwrap();
    let isolated = vm
        .create_isolated_world("construction-world", false)
        .unwrap();
    vm.eval_in_isolated_context(
        isolated,
        r#"
        const event = new Event('construction'); event.createdIn = 'isolated';
        document.querySelector('iframe').contentDocument.getElementById('target').dispatchEvent(event);
        'dispatched'
        "#,
    )
    .unwrap();
    assert_eq!(
        vm.eval_in_child_default_context(child, "JSON.stringify(constructionFacts)")
            .unwrap(),
        "[true,true,true,true,true,true]"
    );
}
