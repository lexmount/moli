use super::realtime_opportunity::run_opportunity;
use super::*;
use moli_layout::{LayoutWorkingSetAdmission, LayoutWorkingSetStats};
use moli_page_types::LayoutPolicy;

mod structure;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Retention {
    Cold,
    Warm,
    EvictBeforeAppend,
}

struct FeedRun {
    snapshots: Vec<serde_json::Value>,
    publications: Vec<(&'static str, LayoutWorkingSetStats)>,
}

fn main_stats(page: &PageVm) -> LayoutWorkingSetStats {
    let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
    let host = host.borrow();
    host.realtime_layout_working_set_stats_for_test(host.document_handle())
        .expect("the small text feed must retain an admitted working set")
}

fn evict_work(page: &PageVm) {
    let builds = page.vm().layout_pass_observability_for_test().1;
    let opportunity = page.current_realtime_rendering_opportunity();
    let deadline = page.next_rendering_opportunity_deadline_for_test();
    {
        let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
        let host = host.borrow();
        let inputs = host.realtime_layout_input_snapshot_for_test();
        host.clear_realtime_layout_working_sets_for_test();
        assert_eq!(host.realtime_layout_working_set_count_for_test(), 0);
        assert_eq!(host.realtime_layout_input_snapshot_for_test(), inputs);
    }
    assert_eq!(page.vm().layout_pass_observability_for_test().1, builds);
    assert_eq!(page.current_realtime_rendering_opportunity(), opportunity);
    assert_eq!(
        page.next_rendering_opportunity_deadline_for_test(),
        deadline
    );
}

fn stats_json(stats: LayoutWorkingSetStats) -> serde_json::Value {
    serde_json::json!({
        "admission": format!("{:?}", stats.admission),
        "retained_boxes": stats.retained_boxes,
        "retained_text_bytes": stats.retained_text_bytes,
        "estimated_retained_bytes": stats.estimated_retained_bytes,
        "reused_box_slots": stats.reused_box_slots,
        "reused_construction_boxes": stats.reused_construction_boxes,
        "reused_owned_boxes": stats.reused_owned_boxes,
        "retained_owned_boxes": stats.retained_owned_boxes,
        "candidate_owned_bytes": stats.candidate_owned_bytes,
        "owned_box_capacity_trimmed": stats.owned_box_capacity_trimmed,
        "reused_inline_contexts": stats.reused_inline_contexts,
        "reused_inline_numeric_layouts": stats.reused_inline_numeric_layouts,
        "candidate_boxes": stats.candidate_boxes,
        "candidate_text_bytes": stats.candidate_text_bytes,
        "candidate_estimated_bytes": stats.candidate_estimated_bytes,
        "estimated_graph_bytes": stats.estimated_graph_bytes,
        "estimated_construction_bytes": stats.estimated_construction_bytes,
        "estimated_inline_bytes": stats.estimated_inline_bytes,
        "estimated_inline_numeric_bytes": stats.estimated_inline_numeric_bytes,
        "estimated_font_metadata_bytes": stats.estimated_font_metadata_bytes,
    })
}

fn write_input(page: &mut PageVm, script: &str) {
    let builds = page.vm().layout_pass_observability_for_test().1;
    page.vm_mut().eval(script).unwrap();
    assert_eq!(page.vm().layout_pass_observability_for_test().1, builds);
}

async fn run_frame(
    page: &mut PageVm,
    loader: &crate::network::ResourceRequestClient,
    retention: Retention,
    phase: &'static str,
    after_append: bool,
    publications: &mut Vec<(&'static str, LayoutWorkingSetStats)>,
) {
    assert!(
        page.next_rendering_opportunity_deadline_for_test()
            .is_some()
    );
    if retention == Retention::Cold || (retention == Retention::EvictBeforeAppend && after_append) {
        evict_work(page);
    }
    let builds = page.vm().layout_pass_observability_for_test().1;
    // Only admit the deadline already requested by the page. No screenshot,
    // getter, test rAF, or new rendering demand makes this frame ready.
    run_opportunity(page, loader).await.unwrap();
    let observed = page.vm().layout_pass_observability_for_test();
    assert_eq!(observed.1, builds + 1, "{retention:?}: {phase}");
    let metrics = observed.3.unwrap();
    assert_eq!(
        metrics.reason,
        moli_layout::LayoutFlushReason::RenderingOpportunity
    );
    assert_eq!(metrics.paint_operation_count, 0);
    let stats = main_stats(page);
    assert_eq!(stats.admission, LayoutWorkingSetAdmission::Retained);
    assert!(stats.retained_boxes > 0 && stats.estimated_inline_numeric_bytes > 0);
    // These are characterization records, not a permanent requirement that
    // an append miss. A future optimization may legitimately increase hits.
    eprintln!(
        "k2_feed_publication {}",
        serde_json::json!({
            "retention": format!("{retention:?}"),
            "phase": phase,
            "builds": observed.1,
            "stats": stats_json(stats),
        }),
    );
    publications.push((phase, stats));
}

async fn notify_io(page: &mut PageVm, loader: &crate::network::ResourceRequestClient) {
    let builds = page.vm().layout_pass_observability_for_test().1;
    assert!(
        page.run_exact_selected_page_task_for_test(
            PageSelectedTaskTestSelector::IntersectionObserver,
            loader,
        )
        .await
        .unwrap(),
        "the frame must have queued an actual IO notification task",
    );
    // The callback appends DOM or records owned entries. It does not read
    // geometry to repair the next frame, nor does IO delivery resample.
    assert_eq!(page.vm().layout_pass_observability_for_test().1, builds);
}

fn assert_no_io_task(page: &mut PageVm) {
    assert!(
        page.claim_exact_selected_page_task_for_test(
            PageSelectedTaskTestSelector::IntersectionObserver,
        )
        .is_none(),
    );
}

async fn drain_scroll_events(page: &mut PageVm, loader: &crate::network::ResourceRequestClient) {
    let builds = page.vm().layout_pass_observability_for_test().1;
    for _ in 0..4 {
        let Some(task) = page
            .claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::RenderingUpdate)
        else {
            assert_eq!(page.vm().layout_pass_observability_for_test().1, builds);
            return;
        };
        assert_eq!(
            task.rendering_update_owner_and_kind().unwrap().1,
            RendererPageRenderingUpdateTaskKind::DocumentScrollEvents,
        );
        page.run_claimed_selected_page_task_for_test(task, loader)
            .await
            .unwrap();
    }
    panic!("the finite scroll input must not create an unbounded event queue");
}

fn checkpoint(page: &mut PageVm) -> serde_json::Value {
    assert!(page.current_realtime_rendering_opportunity().is_none());
    assert!(
        page.next_rendering_opportunity_deadline_for_test()
            .is_none()
    );
    assert!(page.vm().next_timeout_deadline().is_none());
    assert_no_io_task(page);
    let builds = page.vm().layout_pass_observability_for_test().1;
    let snapshot = page
        .vm_mut()
        .eval(
            r#"
        JSON.stringify({
          state:feedState,
          items:Array.from(feedList.children, node => {
            const r=node.getBoundingClientRect();
            return [node.id,node.textContent,r.x,r.y,r.width,r.height,
              node.offsetWidth,node.offsetHeight,node.scrollWidth,node.scrollHeight];
          }),
          boxes:[feedBox,feedList,feedSentinel].map(node => {
            const r=node.getBoundingClientRect();
            return [r.x,r.y,r.width,r.height,node.clientWidth,node.clientHeight,
              node.scrollWidth,node.scrollHeight,node.scrollTop];
          })
        })
    "#,
        )
        .unwrap();
    assert_eq!(
        page.vm().layout_pass_observability_for_test().1,
        builds,
        "geometry inspection after the completed frame must not repair stale output",
    );
    serde_json::from_str(&snapshot).unwrap()
}

async fn feed_run(retention: Retention) -> FeedRun {
    let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
    let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
        &loader,
        Url::parse("https://example.com/realtime-retained-feed").unwrap(),
    );
    page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
    {
        let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
        host.borrow().register_layout_test_font(
            "MoliAhem",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../moli-layout/tests/fixtures/moli-ahem.ttf"
            ))
            .to_vec(),
        );
    }
    let items = (0..8)
        .map(|id| format!("<div class=item id=row-{id}>ABCD</div>"))
        .collect::<String>();
    let markup = format!(
        "<style>body{{margin:0}}#feed{{width:200px;height:100px;overflow:auto}}\
         #list{{width:160px}}.item{{display:block;width:100px;height:20px;\
         padding:0;border:0;font:16px/20px MoliAhem}}\
         #sentinel{{width:100px;height:20px}}</style>\
         <div id=feed><div id=list>{items}</div><div id=sentinel></div></div>"
    );
    write_input(
        &mut page,
        &format!(
            "document.body.innerHTML={}",
            serde_json::to_string(&markup).unwrap()
        ),
    );
    write_input(
        &mut page,
        r#"
        globalThis.feedBox=document.getElementById('feed');
        globalThis.feedList=document.getElementById('list');
        globalThis.feedSentinel=document.getElementById('sentinel');
        globalThis.feedState={raf:0,ro:[],io:[],appends:[],microtasks:0,errors:[]};
        window.addEventListener('error', event=>{
          feedState.errors.push(event.message);event.preventDefault();
        });
        requestAnimationFrame(()=>{feedState.raf++});
        globalThis.feedRO=new ResizeObserver(entries=>{
          for(const entry of entries)feedState.ro.push(entry.contentRect.height);
        });
        feedRO.observe(feedList);
        globalThis.feedIO=new IntersectionObserver(entries=>{
          for(const entry of entries){
            feedState.io.push(entry.isIntersecting);
            if(!entry.isIntersecting || feedState.appends.length===2)continue;
            for(let i=0;i<2;i++){
              const item=document.createElement('div');
              item.id='row-'+feedList.children.length;
              item.className='item';item.textContent='ABCD';feedList.append(item);
            }
            feedState.appends.push(feedList.children.length);
            Promise.resolve().then(()=>{feedState.microtasks++});
          }
        },{root:feedBox});
        feedIO.observe(feedSentinel);
    "#,
    );
    let mut publications = Vec::new();
    let mut snapshots = Vec::new();
    run_frame(
        &mut page,
        &loader,
        retention,
        "initial",
        false,
        &mut publications,
    )
    .await;
    notify_io(&mut page, &loader).await;
    snapshots.push(checkpoint(&mut page));

    for (round, width, scroll_top, width_phase, scroll_phase, append_phase) in [
        (0, 120, 80, "width-1", "scroll-1", "append-1"),
        (1, 100, 120, "width-2", "scroll-2", "append-2"),
    ] {
        write_input(
            &mut page,
            &format!("feedList.firstElementChild.style.width='{width}px'"),
        );
        run_frame(
            &mut page,
            &loader,
            retention,
            width_phase,
            false,
            &mut publications,
        )
        .await;
        snapshots.push(checkpoint(&mut page));

        // Identical, fixed scroll inputs in every retention variant. No
        // scrollHeight getter is used to choose or repair the scroll target.
        write_input(&mut page, &format!("feedBox.scrollTop={scroll_top}"));
        drain_scroll_events(&mut page, &loader).await;
        run_frame(
            &mut page,
            &loader,
            retention,
            scroll_phase,
            false,
            &mut publications,
        )
        .await;
        notify_io(&mut page, &loader).await;
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_some()
        );
        run_frame(
            &mut page,
            &loader,
            retention,
            append_phase,
            true,
            &mut publications,
        )
        .await;
        notify_io(&mut page, &loader).await;
        let snapshot = checkpoint(&mut page);
        assert_eq!(snapshot["items"].as_array().unwrap().len(), 10 + round * 2);
        assert_eq!(snapshot["boxes"][1][3], serde_json::json!(200 + round * 40));
        assert_eq!(snapshot["boxes"][0][7], serde_json::json!(220 + round * 40));
        snapshots.push(snapshot);
    }
    let final_snapshot = snapshots.last().unwrap();
    assert_eq!(
        final_snapshot["state"],
        serde_json::json!({
            "raf":1,"ro":[160,200,240],"io":[false,true,false,true,false],
            "appends":[10,12],"microtasks":2,"errors":[]
        })
    );
    assert_eq!(publications.len(), 7);

    // A clean published answer is independent of optional work residency.
    evict_work(&page);
    assert_eq!(&checkpoint(&mut page), final_snapshot);
    FeedRun {
        snapshots,
        publications,
    }
}

#[test]
fn realtime_text_feed_matches_cold_warm_and_evicted_work_without_capture() {
    run_page_vm_large_stack_async_test("realtime-retained-text-feed", || async move {
        let cold = feed_run(Retention::Cold).await;
        let warm = feed_run(Retention::Warm).await;
        let evicted = feed_run(Retention::EvictBeforeAppend).await;
        assert_eq!(warm.snapshots, cold.snapshots);
        assert_eq!(evicted.snapshots, cold.snapshots);
        for (_, stats) in &cold.publications {
            assert_eq!(stats.reused_construction_boxes, 0);
            assert_eq!(stats.reused_inline_contexts, 0);
            assert_eq!(stats.reused_inline_numeric_layouts, 0);
        }
        // A positive control proves that this fixture can reuse real work.
        // Append statistics remain observations, not a zero-hit contract.
        for run in [&warm, &evicted] {
            let stats = run
                .publications
                .iter()
                .find(|(phase, _)| *phase == "width-1")
                .unwrap()
                .1;
            assert!(stats.reused_construction_boxes > 0, "{stats:?}");
            assert_eq!(stats.reused_owned_boxes, stats.retained_boxes, "{stats:?}");
            assert!(stats.reused_inline_contexts > 0, "{stats:?}");
            assert!(stats.reused_inline_numeric_layouts > 0, "{stats:?}");
        }
        for (phase, stats) in &evicted.publications {
            if phase.starts_with("append-") {
                assert_eq!(stats.reused_inline_contexts, 0);
                assert_eq!(stats.reused_inline_numeric_layouts, 0);
            }
        }
    });
}

// Appending a sibling must preserve validated preparation and final numeric
// work for the unchanged paragraphs, while the page observes fresh geometry.
#[test]
fn realtime_tail_append_reuses_unchanged_plain_text_work() {
    run_page_vm_large_stack_async_test("realtime-tail-append-reuse-target", || async move {
        let run = feed_run(Retention::Warm).await;
        for (phase, previous_paragraphs) in [("append-1", 8), ("append-2", 10)] {
            let stats = run
                .publications
                .iter()
                .find(|(name, _)| *name == phase)
                .unwrap()
                .1;
            assert!(
                stats.reused_inline_contexts >= previous_paragraphs,
                "{phase}: {stats:?}"
            );
            // Numeric counts are final-call hits, not a set of unique owners.
            // Together with the unchanged-IFC count, this proves some actual
            // numeric work was skipped; it does not prove every owner hit.
            assert!(
                stats.reused_inline_numeric_layouts > 0,
                "{phase}: {stats:?}"
            );
        }
    });
}
