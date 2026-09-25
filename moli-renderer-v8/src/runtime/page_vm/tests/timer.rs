use super::*;

use super::super::page_timer::PageTimerTurnAction;
use crate::page_task_queue::{
    RendererPageReadyDescriptor, RendererPageSchedulerTask, RendererPageTimerSelection,
};

const ANY_READY_TIMER: RendererPageTimerSelection = RendererPageTimerSelection::AnyReady;

#[tokio::test(flavor = "current_thread")]
async fn popup_animation_frame_flushes_autofocus_before_its_callback_snapshot() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page_vm, _queue, _wake) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader, Url::parse("https://example.com/popup-frame-autofocus")?,
        );
        page_vm.vm_mut().eval(r#"
globalThis.popup=open();
const d=popup.document;
d.open();
d.write('<!doctype html><body><input id=candidate autofocus>');
globalThis.order=[];
const candidate=d.getElementById('candidate');
candidate.addEventListener('focus',()=>{
  order.push('focus');
  queueMicrotask(()=>order.push('microtask'));
  popup.requestAnimationFrame(()=>order.push('from-focus'));
});
popup.requestAnimationFrame(()=>order.push(d.activeElement===candidate?'frame-focused':'frame-unfocused'));
'scheduled'
"#)?;
        assert!(page_vm.claim_exact_selected_page_task_for_test(
            PageSelectedTaskTestSelector::RenderingUpdate,
        ).is_none(), "the open parser has not dispatched DOMContentLoaded");
        tokio::time::sleep(Duration::from_millis(20)).await;
        let deadline = due_timer_deadline(&page_vm);
        run_timer_through_selected_dispatcher(&mut page_vm, deadline, &loader).await?;
        assert_eq!(page_vm.vm_mut().eval("order.join('|')")?, "");
        assert!(page_vm.run_exact_selected_page_task_for_test(
            PageSelectedTaskTestSelector::RenderingUpdate, &loader,
        ).await?);
        assert_eq!(page_vm.vm_mut().eval("order.join('|')")?,
            "focus|microtask|frame-focused|from-focus");

        page_vm.vm_mut().eval(r#"
candidate.blur();
d.open();
d.write('<!doctype html><body><input id=replacement autofocus>');
popup.requestAnimationFrame(()=>order.push(d.activeElement===d.body?'processed-preserved':'focused-again'));
'reopened'
"#)?;
        tokio::time::sleep(Duration::from_millis(20)).await;
        let deadline = due_timer_deadline(&page_vm);
        run_timer_through_selected_dispatcher(&mut page_vm, deadline, &loader).await?;
        assert!(page_vm.run_exact_selected_page_task_for_test(
            PageSelectedTaskTestSelector::RenderingUpdate, &loader,
        ).await?);
        assert_eq!(page_vm.vm_mut().eval("order.join('|')")?,
            "focus|microtask|frame-focused|from-focus|processed-preserved");
        page_vm.vm_mut().eval("popup.close()")?;
        Ok::<_, anyhow::Error>(())
    }).await.expect("popup rendering must flush its own Document's autofocus");
}

fn due_timer_deadline(page_vm: &PageVm) -> Instant {
    match page_vm
        .due_page_timer_ready_descriptor(ANY_READY_TIMER)
        .expect("a due timer descriptor should be ready")
    {
        RendererPageReadyDescriptor::Timer { deadline, .. } => deadline,
        other => panic!("expected timer descriptor, got {other:?}"),
    }
}

async fn run_timer_through_selected_dispatcher(
    page_vm: &mut PageVm,
    deadline: Instant,
    loader: &crate::network::ResourceRequestClient,
) -> anyhow::Result<()> {
    page_vm
        .apply_selected_page_scheduler_task_on_owner_lane_for_test(
            RendererPageSchedulerTask::Timer {
                deadline,
                selection: ANY_READY_TIMER,
            },
            loader.clone(),
        )
        .await?;
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn animation_frame_wake_publishes_one_rendering_batch_after_autofocus() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(
                &loader,
                Url::parse("https://example.com/animation-frame-batch")?,
            );
        page_vm.vm_mut().eval(
            r#"
globalThis.frameOrder = [];
globalThis.frameTimes = [];
requestAnimationFrame(timestamp => {
  frameOrder.push('first');
  frameTimes.push(timestamp);
  queueMicrotask(() => {
    frameOrder.push('first-microtask');
    cancelAnimationFrame(cancelledFrame);
  });
  requestAnimationFrame(next => {
    frameOrder.push('next-frame');
    frameTimes.push(next);
  });
});
const cancelledFrame = requestAnimationFrame(() => frameOrder.push('cancelled'));
"queued"
"#,
        )?;
        // A frame deadline can expire while a parser script still owns the
        // thread. Its later insertion must be considered before any callback.
        tokio::time::sleep(Duration::from_millis(20)).await;
        page_vm.vm_mut().eval(
            r#"
requestAnimationFrame(timestamp => {
  frameOrder.push('late');
  frameTimes.push(timestamp);
});
const candidate = document.createElement('input');
candidate.autofocus = true;
candidate.addEventListener('focus', () => {
  frameOrder.push('focus');
  queueMicrotask(() => frameOrder.push('focus-microtask'));
  requestAnimationFrame(timestamp => {
    frameOrder.push('from-focus');
    frameTimes.push(timestamp);
  });
});
document.body.append(candidate);
"inserted"
"#,
        )?;
        let deadline = due_timer_deadline(&page_vm);
        run_timer_through_selected_dispatcher(&mut page_vm, deadline, &loader).await?;
        assert_eq!(
            page_vm.vm_mut().eval("frameOrder.join('|')")?,
            "",
            "the deadline only publishes rendering work"
        );
        let claimed = page_vm
            .claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::RenderingUpdate)
            .expect("frame wake should publish a rendering task");
        assert_eq!(
            claimed.rendering_update_owner_and_kind().unwrap().1,
            crate::page_task_queue::RendererPageRenderingUpdateTaskKind::AnimationFrameCallbacks
        );
        page_vm
            .run_claimed_selected_page_task_for_test(claimed, &loader)
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval("frameOrder.join('|')")?,
            "focus|focus-microtask|first|first-microtask|late|from-focus"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("frameTimes.length === 3 && frameTimes.every(t => t === frameTimes[0])")?,
            "true"
        );
        assert!(
            page_vm
                .claim_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::RenderingUpdate,
                )
                .is_none(),
            "callbacks registered during the batch await another frame"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
        let deadline = due_timer_deadline(&page_vm);
        run_timer_through_selected_dispatcher(&mut page_vm, deadline, &loader).await?;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::RenderingUpdate,
                    &loader,
                )
                .await?
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("frameOrder.at(-1) === 'next-frame' && frameTimes[3] > frameTimes[0]")?,
            "true"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("animation frame rendering batch should run");
}

#[tokio::test(flavor = "current_thread")]
async fn animation_frame_republishes_after_document_open_without_retargeting_a_claim() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(
                &loader,
                Url::parse("https://example.com/animation-frame-document-open")?,
            );
        page_vm.vm_mut().eval(
            "globalThis.frameRan = false; requestAnimationFrame(() => { frameRan = true; })",
        )?;
        tokio::time::sleep(Duration::from_millis(20)).await;
        let deadline = due_timer_deadline(&page_vm);
        run_timer_through_selected_dispatcher(&mut page_vm, deadline, &loader).await?;
        let claimed = page_vm
            .claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::RenderingUpdate)
            .expect("frame should publish before replacement");
        let old_owner = claimed.rendering_update_owner_and_kind().unwrap().0;
        page_vm.vm_mut().eval(
            "document.open(); document.write('<!doctype html><body>replacement'); document.close()",
        )?;
        page_vm
            .run_claimed_selected_page_task_for_test(claimed, &loader)
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval("String(frameRan)")?,
            "false",
            "stale rendering authority must not execute against the replacement incarnation"
        );
        let replacement = page_vm
            .claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::RenderingUpdate)
            .expect("surviving Window callbacks must acquire a new Document task");
        assert_ne!(
            replacement.rendering_update_owner_and_kind().unwrap().0,
            old_owner
        );
        page_vm
            .run_claimed_selected_page_task_for_test(replacement, &loader)
            .await?;
        assert_eq!(page_vm.vm_mut().eval("String(frameRan)")?, "true");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("animation callbacks should survive document.open");
}

#[tokio::test(flavor = "current_thread")]
async fn page_timer_turn_consumes_exactly_one_due_timer() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader(&loader, Vec::new());
        page_vm.vm_mut().eval(
            r#"
globalThis.__pageTimerTurnOrder = [];
setTimeout(() => {
  __pageTimerTurnOrder.push("first");
  Promise.resolve().then(() => __pageTimerTurnOrder.push("microtask:first"));
}, 0);
setTimeout(() => {
  __pageTimerTurnOrder.push("second");
  Promise.resolve().then(() => __pageTimerTurnOrder.push("microtask:second"));
}, 0);
"queued"
"#,
        )?;

        let first_deadline = due_timer_deadline(&page_vm);
        run_timer_through_selected_dispatcher(&mut page_vm, first_deadline, &loader).await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("JSON.stringify(__pageTimerTurnOrder)")?,
            r#"["first","microtask:first"]"#,
            "one selected timer task must checkpoint its callback without draining the next timer"
        );

        let second_deadline = due_timer_deadline(&page_vm);
        run_timer_through_selected_dispatcher(&mut page_vm, second_deadline, &loader).await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("JSON.stringify(__pageTimerTurnOrder)")?,
            r#"["first","microtask:first","second","microtask:second"]"#
        );
        assert!(
            page_vm
                .due_page_timer_ready_descriptor(ANY_READY_TIMER)
                .is_none(),
            "two timer tasks must consume exactly two selected turns"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("typed timer one-turn test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn timer_body_leaves_reactions_for_selected_callback_completion() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader(&loader, Vec::new());
        page_vm.vm_mut().eval(
            r#"
globalThis.__timerBodyBoundary = [];
setTimeout(() => {
  __timerBodyBoundary.push("callback");
  Promise.resolve().then(() => __timerBodyBoundary.push("microtask"));
}, 0);
"queued"
"#,
        )?;

        let deadline = due_timer_deadline(&page_vm);
        let body = page_vm.apply_selected_page_timer_turn(deadline, ANY_READY_TIMER)?;
        assert!(matches!(body.action, PageTimerTurnAction::Consumed { deadline: actual, ref popup_parser_completions, .. }
            if actual == deadline && popup_parser_completions.is_empty()));
        assert_eq!(
            page_vm.vm_mut().eval("__timerBodyBoundary.join('|')")?,
            "callback",
            "the timer heap executor must leave Promise reactions pending"
        );

        page_vm.finish_selected_page_callback_task(&loader).await?;
        assert_eq!(
            page_vm.vm_mut().eval("__timerBodyBoundary.join('|')")?,
            "callback|microtask",
            "the selected timer completion must own the single task checkpoint"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("timer body/completion boundary test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn selected_timer_error_still_completes_its_task_checkpoint() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader(&loader, Vec::new());
        page_vm.vm_mut().eval(
            r#"
globalThis.__timerErrorBoundary = [];
setTimeout(() => {
  __timerErrorBoundary.push("callback");
  Promise.resolve().then(() => __timerErrorBoundary.push("microtask"));
  throw new Error("selected timer error boundary");
}, 0);
"queued"
"#,
        )?;

        let deadline = due_timer_deadline(&page_vm);
        run_timer_through_selected_dispatcher(&mut page_vm, deadline, &loader).await?;
        assert_eq!(
            page_vm.vm_mut().eval("__timerErrorBoundary.join('|')")?,
            "callback|microtask",
            "a throwing callback still consumed a selected timer task whose checkpoint must finish"
        );
        assert!(
            page_vm
                .vm_mut()
                .runtime_observable_lifecycle_errors_for_testing()
                .iter()
                .any(|warning| warning.contains("selected timer error boundary")),
            "the callback failure must remain observable after task completion"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("timer error completion boundary test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn timer_completion_syncs_a_microtask_created_child_after_the_checkpoint() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader(&loader, Vec::new());
        page_vm.vm_mut().eval(
            r#"
globalThis.__timerChildOrder = [];
setTimeout(() => {
  __timerChildOrder.push("callback");
  Promise.resolve().then(() => {
    __timerChildOrder.push("microtask");
    const frame = document.createElement("iframe");
    frame.id = "timer-microtask-child";
    frame.srcdoc = "<!doctype html><body>child</body>";
    document.body.appendChild(frame);
  });
}, 0);
"queued"
"#,
        )?;

        let deadline = due_timer_deadline(&page_vm);
        run_timer_through_selected_dispatcher(&mut page_vm, deadline, &loader).await?;
        assert_eq!(
            page_vm.vm_mut().eval("__timerChildOrder.join('|')")?,
            "callback|microtask"
        );
        assert!(
            page_vm.vm().has_pending_child_navigation_commit_for_test(),
            "a reaction-created srcdoc frame must publish a typed navigation commit during timer completion"
        );
        assert_eq!(
            page_vm
                .run_next_child_frame_task_source_for_semantic_test()
                .await,
            Some(crate::frame_owner_model::ChildFrameSemanticTurnKind::NavigationCommit),
            "the microtask-created frame must remain a concrete later Page task"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("timer post-checkpoint child synchronization test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn interval_reaction_can_cancel_the_rescheduled_timer_at_task_end() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader(&loader, Vec::new());
        page_vm.vm_mut().eval(
            r#"
globalThis.__intervalCheckpointOrder = [];
const interval = setInterval(() => {
  __intervalCheckpointOrder.push("callback");
  Promise.resolve().then(() => {
    __intervalCheckpointOrder.push("microtask:clear");
    clearInterval(interval);
  });
}, 0);
"queued"
"#,
        )?;

        let deadline = due_timer_deadline(&page_vm);
        run_timer_through_selected_dispatcher(&mut page_vm, deadline, &loader).await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__intervalCheckpointOrder.join('|')")?,
            "callback|microtask:clear"
        );
        assert_eq!(
            page_vm.vm().next_timeout_deadline(),
            None,
            "the task-end reaction must cancel the interval body that was rescheduled before checkpoint"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("interval task-end cancellation test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn selected_timer_revalidates_the_heap_head_before_execution() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader(&loader, Vec::new());
        page_vm.vm_mut().eval(
            r#"
globalThis.__revalidatedTimerOrder = [];
setTimeout(() => {
  __revalidatedTimerOrder.push("callback");
  Promise.resolve().then(() => __revalidatedTimerOrder.push("microtask"));
}, 0);
"queued"
"#,
        )?;
        let actual_deadline = page_vm
            .vm()
            .next_timeout_deadline()
            .expect("zero-delay timer must retain a heap deadline");
        let stale_deadline = actual_deadline
            .checked_add(Duration::from_nanos(1))
            .expect("test deadline should be representable");

        let stale = page_vm.apply_selected_page_timer_turn(stale_deadline, ANY_READY_TIMER)?;
        assert_eq!(
            stale.action,
            PageTimerTurnAction::NoLongerRunnable {
                expected_deadline: stale_deadline,
                actual_deadline: Some(actual_deadline),
            }
        );
        assert_eq!(
            page_vm.vm_mut().eval("__revalidatedTimerOrder.join('|')")?,
            "",
            "a stale deadline must not enter V8 or checkpoint another task's reactions"
        );

        run_timer_through_selected_dispatcher(&mut page_vm, actual_deadline, &loader).await?;
        assert_eq!(
            page_vm.vm_mut().eval("__revalidatedTimerOrder.join('|')")?,
            "callback|microtask"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("timer descriptor revalidation test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn timer_deadline_observation_cannot_consume_a_due_timer() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader(&loader, Vec::new());
        page_vm.vm_mut().eval(
            "globalThis.__waitObserverTimerRan = false; \
             setTimeout(() => { __waitObserverTimerRan = true; }, 0);",
        )?;
        assert!(page_vm.vm().has_ready_timeout());

        let executor = page_vm.local_executor.clone();
        crate::runtime::access::run_named_owner_local_task(
            executor,
            "timer deadline observation test task closed",
            async move {
                let timer_deadline = page_vm.vm().ms_to_next_timeout();
                assert_eq!(timer_deadline, Some(0));
                assert_eq!(
                    page_vm.vm_mut().eval("String(__waitObserverTimerRan)")?,
                    "false",
                    "reading the timer deadline must not steal the scheduler-owned timer"
                );
                assert!(
                    page_vm
                        .due_page_timer_ready_descriptor(ANY_READY_TIMER)
                        .is_some()
                );
                Ok(())
            },
        )
        .await?;
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("timer deadline observation test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn timer_is_not_runnable_before_its_deadline() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        page_vm.vm_mut().eval(
            "globalThis.__futureTimerRan = false; \
             setTimeout(() => { __futureTimerRan = true; }, 60000);",
        )?;

        assert!(page_vm.vm().next_timeout_deadline().is_some());
        assert!(
            page_vm
                .due_page_timer_ready_descriptor(ANY_READY_TIMER)
                .is_none(),
            "a future heap entry is a deadline, not a runnable Page task"
        );
        assert_eq!(page_vm.vm_mut().eval("String(__futureTimerRan)")?, "false");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("future timer readiness test should run");
}
