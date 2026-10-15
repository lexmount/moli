use super::*;
use crate::runtime::{RendererPageCommand, RendererPointerEventProperties};

const FIXTURE: &str =
    include_str!("../../../../../../tests/fixtures/realtime-working-set-churn.js");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Residency {
    Cold,
    Warm,
    EvictAlternating,
}

struct StructureRun {
    snapshots: Vec<serde_json::Value>,
    publications: Vec<LayoutWorkingSetStats>,
}

fn mouse(page: &mut PageVm, event: &str, delta: f64) {
    page.dispatch_renderer_page_command(RendererPageCommand::DispatchMouseEventAtPoint {
        x: 12.0,
        y: 12.0,
        event_name: event.to_owned(),
        button: if event == "wheel" { -1 } else { 0 },
        buttons: Some(i32::from(event == "mousedown")),
        click_count: 1,
        delta_x: 0.0,
        delta_y: delta,
        pointer: RendererPointerEventProperties::default(),
        modifiers: 0,
    })
    .unwrap();
}

async fn settle(
    page: &mut PageVm,
    loader: &crate::network::ResourceRequestClient,
    residency: Residency,
    run: &mut StructureRun,
) {
    if residency == Residency::Cold
        || (residency == Residency::EvictAlternating && run.publications.len().is_multiple_of(2))
    {
        evict_work(page);
    }
    let before = page.vm().layout_pass_observability_for_test().1;
    let deadline = page
        .next_rendering_opportunity_deadline_for_test()
        .expect("the page requested its own frame");
    page.admit_rendering_opportunity_at_for_test(deadline);
    let mut frame_ran = false;
    // A requested frame can already be ready before explicit admission. It
    // shares this source with document scroll events; neither kind owns an
    // unconditional queue-head position. Consume both in their actual order.
    for _ in 0..8 {
        let Some(task) = page
            .claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::RenderingUpdate)
        else {
            break;
        };
        match task.rendering_update_owner_and_kind().unwrap().1 {
            RendererPageRenderingUpdateTaskKind::FrameLifecycle => {
                assert!(!frame_ran, "one mutation must not run two frames");
                frame_ran = true;
            }
            RendererPageRenderingUpdateTaskKind::DocumentScrollEvents => {}
            kind => panic!("unexpected rendering work in structural fixture: {kind:?}"),
        }
        page.run_claimed_selected_page_task_for_test(task, loader)
            .await
            .unwrap();
        assert_eq!(
            page.vm().layout_pass_observability_for_test().1,
            before + u64::from(frame_ran),
            "scroll delivery must not build layout or consume another frame"
        );
    }
    assert!(frame_ran, "the requested frame must complete");
    let observed = page.vm().layout_pass_observability_for_test();
    assert_eq!(observed.1, before + 1, "{residency:?}");
    let metrics = observed.3.unwrap();
    assert_eq!(
        metrics.reason,
        moli_layout::LayoutFlushReason::RenderingOpportunity
    );
    assert_eq!(metrics.paint_event_count, 0);
    assert_eq!(metrics.paint_operation_count, 0);
    run.publications.push(main_stats(page));
    if let Some(task) = page
        .claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::IntersectionObserver)
    {
        page.run_claimed_selected_page_task_for_test(task, loader)
            .await
            .unwrap();
    }
    assert_eq!(page.vm().layout_pass_observability_for_test().1, observed.1);
    assert_no_io_task(page);
    assert!(page.current_realtime_rendering_opportunity().is_none());
    assert!(
        page.next_rendering_opportunity_deadline_for_test()
            .is_none()
    );
    assert!(page.vm().next_timeout_deadline().is_none());
    let snapshot: serde_json::Value = serde_json::from_str(
        &page
            .vm_mut()
            .eval("JSON.stringify(churn.snapshot())")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        page.vm().layout_pass_observability_for_test().1,
        observed.1,
        "inspection after the lifecycle must not repair geometry: {residency:?}"
    );
    let rows = snapshot["rows"].as_array().unwrap();
    let count = rows.len();
    assert_eq!(snapshot["list"][3], serde_json::json!(count * 24));
    assert_eq!(snapshot["feed"][9], serde_json::json!((count + 1) * 24));
    assert_eq!(snapshot["state"]["errors"], serde_json::json!([]));
    assert_eq!(snapshot["state"]["raf"], snapshot["state"]["microtasks"]);
    assert!(rows.iter().all(|row| row["identity"] == true));
    let scroll_top = snapshot["scrollTop"].as_u64().unwrap() as usize;
    assert_eq!(snapshot["hit"], rows[scroll_top / 24]["id"]);
    eprintln!(
        "k2_structure_publication {}",
        serde_json::json!({
            "residency": format!("{residency:?}"),
            "sequence": run.publications.len(),
            "phase": snapshot["phase"],
            "rows": count,
            "stats": stats_json(*run.publications.last().unwrap()),
        })
    );
    run.snapshots.push(snapshot);
}

async fn structure_run(residency: Residency) -> StructureRun {
    let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
    let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
        &loader,
        Url::parse("https://example.com/realtime-working-set-churn").unwrap(),
    );
    page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
    run_opportunity(&mut page, &loader).await.unwrap();
    {
        let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
        // Make every fixture deadline already due without a wall-clock sleep.
        // This deterministically covers the queue ordering seen under load.
        host.borrow().set_realtime_opportunity_clock_for_test(
            std::time::Instant::now() - std::time::Duration::from_secs(5),
        );
        host.borrow().register_layout_test_font(
            "MoliAhem",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../moli-layout/tests/fixtures/moli-ahem.ttf"
            ))
            .to_vec(),
        );
    }
    write_input(&mut page, FIXTURE);
    let mut run = StructureRun {
        snapshots: Vec::new(),
        publications: Vec::new(),
    };
    settle(&mut page, &loader, residency, &mut run).await;
    let operations: Vec<String> = serde_json::from_str(
        &page
            .vm_mut()
            .eval("JSON.stringify(churn.operations)")
            .unwrap(),
    )
    .unwrap();
    for round in 0..2 {
        for operation in &operations {
            write_input(
                &mut page,
                &format!(
                    "churn.apply({}, {round})",
                    serde_json::to_string(operation).unwrap(),
                ),
            );
            if operation == "wheel" {
                mouse(&mut page, "wheel", 48.0);
                assert!(page.next_action_window_deadline().is_none());
            }
            settle(&mut page, &loader, residency, &mut run).await;
            let expected = run.snapshots.last().unwrap()["hit"].clone();
            mouse(&mut page, "mousedown", 0.0);
            mouse(&mut page, "mouseup", 0.0);
            // The expected target comes from an already completed layout;
            // the input packet itself has only fixed CSS coordinates.
            let clicked: serde_json::Value = serde_json::from_str(
                &page
                    .vm_mut()
                    .eval("JSON.stringify(churn.state.clicks.at(-1))")
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(
                clicked,
                serde_json::json!({
                    "id":expected, "connected":true, "identity":true
                })
            );
            settle(&mut page, &loader, residency, &mut run).await;
        }
    }
    let last = run.snapshots.last().unwrap();
    assert_eq!(run.publications.len(), 65);
    assert_eq!(last["rows"].as_array().unwrap().len(), 14);
    assert_eq!(last["state"]["clicks"].as_array().unwrap().len(), 32);
    assert_eq!(last["state"]["wheels"], serde_json::json!([48, 48]));
    assert_eq!(last["state"]["raf"].as_array().unwrap().len(), 65);
    assert_eq!(
        last["state"]["io"],
        serde_json::json!([false, true, false, true, false, true, false, true, false])
    );
    assert_eq!(
        last["state"]["ro"],
        serde_json::json!([
            384, 432, 480, 504, 480, 456, 432, 72, 336, 384, 432, 456, 432, 408, 384, 72, 336
        ])
    );
    assert_eq!(last["state"]["replacements"].as_array().unwrap().len(), 2);
    assert!(
        last["state"]["replacements"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| { item["same"] == false && item["oldConnected"] == false })
    );
    evict_work(&page);
    let builds = page.vm().layout_pass_observability_for_test().1;
    let after_eviction: serde_json::Value = serde_json::from_str(
        &page
            .vm_mut()
            .eval("JSON.stringify(churn.snapshot())")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(&after_eviction, last);
    assert_eq!(page.vm().layout_pass_observability_for_test().1, builds);
    run
}

#[test]
fn realtime_structural_churn_keeps_callbacks_geometry_and_input_equal_with_eviction() {
    run_page_vm_large_stack_async_test("realtime-structural-churn", || async move {
        let cold = structure_run(Residency::Cold).await;
        let warm = structure_run(Residency::Warm).await;
        let evicted = structure_run(Residency::EvictAlternating).await;
        assert_eq!(warm.snapshots, cold.snapshots);
        assert_eq!(evicted.snapshots, cold.snapshots);
        assert!(cold.publications.iter().all(|stats| {
            stats.reused_inline_contexts == 0 && stats.reused_inline_numeric_layouts == 0
        }));
        for run in [&warm, &evicted] {
            assert!(
                run.publications
                    .iter()
                    .any(|stats| stats.reused_inline_contexts > 0)
            );
            assert!(
                run.publications
                    .iter()
                    .any(|stats| stats.reused_inline_numeric_layouts > 0)
            );
            assert!(
                run.publications
                    .iter()
                    .any(|stats| stats.reused_owned_boxes > 0)
            );
        }
    });
}
