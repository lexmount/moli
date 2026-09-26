use super::*;

async fn expect_one_child_frame_task_source(
    vm: &mut ScriptVm,
    expected: impl Into<ChildFrameSemanticTurnKind>,
    context: &str,
) {
    let expected = expected.into();
    let source = vm.run_next_child_frame_semantic_turn_for_test().await;
    assert_eq!(source, Some(expected), "{context}");
}

/// Settle at most one realm-materialization prerequisite before observing the
/// requested child-family turn. This is test setup, not a one-turn executor.
async fn expect_child_frame_task_source_after_realm_prerequisite(
    vm: &mut ScriptVm,
    expected: impl Into<ChildFrameSemanticTurnKind>,
    context: &str,
) {
    let expected = expected.into();
    if expected != ChildFrameSemanticTurnKind::RealmMaterialization
        && vm.has_ready_child_frame_semantic_turn_for_test(
            ChildFrameSemanticTurnKind::RealmMaterialization,
        )
    {
        assert_eq!(
            vm.run_next_child_frame_semantic_turn_for_test().await,
            Some(ChildFrameSemanticTurnKind::RealmMaterialization),
            "{context}: exact child realm prerequisite"
        );
    }
    expect_one_child_frame_task_source(vm, expected, context).await;
}

/// Observe one exact child semantic family through the production Page
/// selected-task dispatcher, allowing the single realm-materialization
/// prerequisite that can precede it.
async fn expect_page_child_frame_task_source_after_realm_prerequisite(
    page: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    loader: &ResourceRequestClient,
    expected: impl Into<ChildFrameSemanticTurnKind>,
    context: &str,
) {
    let expected = expected.into();
    if expected != ChildFrameSemanticTurnKind::RealmMaterialization
        && page
            .run_one_child_frame_task_executor_turn(
                ChildFrameSemanticTurnKind::RealmMaterialization,
                loader,
            )
            .await
            .expect("exact child realm prerequisite should run")
    {
        // Realm materialization is the only prerequisite this helper may
        // consume before the requested family.
    }
    assert!(
        page.run_one_child_frame_task_executor_turn(expected, loader)
            .await
            .expect("exact child semantic task should run"),
        "{context}"
    );
}

fn drain_image_load_event_bodies_for_test(vm: &mut ScriptVm) -> usize {
    let mut count = 0;
    while vm
        .apply_next_image_load_event_body_for_test()
        .expect("DOM-manipulation task should run")
    {
        count += 1;
    }
    count
}
mod extracted;
