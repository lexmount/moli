use super::*;
use crate::runtime::owner_local::RendererLayoutWorkingSetObservation;

const CHILD_RETIREMENT_TIMEOUT: Duration = Duration::from_secs(30);

// Preserve the original load/read/remove sequence and normal frame barriers;
// each inspector result is a primitive string, never a retained child object.
const CHILD_CYCLE: &str = r#"async seed => {
  const frames = () => new Promise(resolve => requestAnimationFrame(() =>
    requestAnimationFrame(() => setTimeout(resolve, 0))));
  const frame = document.createElement('iframe');
  frame.style.cssText = 'width:300px;height:100px;border:0';
  const loaded = new Promise(resolve => { frame.onload = resolve; });
  frame.srcdoc = '<!doctype html><style>p{width:123px;margin:0;font:16px serif}</style>' +
    Array.from({length:64}, (_, i) => '<p>child ' + seed + ' row ' + i + '</p>').join('');
  document.body.append(frame);
  await loaded;
  const width = frame.contentDocument.querySelector('p').offsetWidth;
  const count = frame.contentDocument.querySelectorAll('p').length;
  await frames();
  frame.remove();
  await frames();
  return JSON.stringify({width, count, remaining:document.querySelectorAll('iframe').length});
}"#;

async fn primitive_json(page: &RendererPageHandle, expression: String) -> serde_json::Value {
    let (reply, _) = tokio::time::timeout(
        CHILD_RETIREMENT_TIMEOUT,
        page.run_async_command(RendererPageCommand::EvaluateExpression {
            expression,
            await_promise: true,
        }),
    )
    .await
    .expect("child retirement command should not stall")
    .expect("child retirement command should succeed");
    // The unbound Page state is dropped here, before the next lifecycle step.
    let RendererPageReply::RuntimeEvaluationResult(result) = reply else {
        panic!("expected a runtime evaluation result");
    };
    let payload = result.into_protocol_payload();
    assert!(
        payload.get("exception").is_none() && payload.get("exceptionDetails").is_none(),
        "child retirement script failed: {payload}"
    );
    assert_eq!(payload["type"], serde_json::json!("string"));
    serde_json::from_str(
        payload["value"]
            .as_str()
            .expect("JSON must be an owned string"),
    )
    .expect("the child result must be valid JSON")
}

async fn heap(page: &RendererPageHandle) -> serde_json::Value {
    tokio::time::timeout(CHILD_RETIREMENT_TIMEOUT, runtime_heap_usage_for_test(page))
        .await
        .expect("child retirement counters should not stall")
}

async fn observation(page: &RendererPageHandle) -> RendererLayoutWorkingSetObservation {
    tokio::time::timeout(
        CHILD_RETIREMENT_TIMEOUT,
        RendererPageTestingHandle::new_for_testing(page).layout_working_set_observation_async(),
    )
    .await
    .expect("owner observation should not stall")
    .expect("the acknowledged Page should be resident")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repeated_child_retirement_releases_logical_owners_in_all_layout_policies() {
    for policy in [
        LayoutPolicy::Mock,
        LayoutPolicy::OnDemand,
        LayoutPolicy::Realtime,
    ] {
        let pool = policy.is_realtime().then(LayoutWorkingSetBudget::default);
        let runtime = JsRuntime::initialize();
        configure(&runtime, policy, pool.clone());
        assert_eq!(
            runtime
                .renderer_owner_handle()
                .layout_working_set_budget()
                .is_some(),
            policy.is_realtime(),
        );
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        let mut page = page(&runtime, &loader).await;
        let initial = primitive_json(&page, "JSON.stringify(initialWidth)".to_owned()).await;
        let root = observation(&page).await;
        let root_account = pool.as_ref().map(|pool| account(pool, &page));
        if let Some(account) = &root_account {
            assert_eq!(initial, serde_json::json!(120));
            assert_eq!((root.working_sets, root.tickets), (1, 1));
            assert!(
                account.charged_bytes > 0,
                "exercise an admitted root working set"
            );
            assert_eq!(account.documents, 1);
        }
        let root_registrations =
            heap(&page).await["moli"]["runtime"]["inspectorContextRegistrationCount"]
                .as_u64()
                .expect("the root registration count must be present");
        assert!(root_registrations > 0);

        for seed in 0..4 {
            let result = primitive_json(&page, format!("({CHILD_CYCLE})({seed})")).await;
            assert_eq!(
                result["count"],
                serde_json::json!(64),
                "{policy:?}: {result}"
            );
            assert_eq!(
                result["remaining"],
                serde_json::json!(0),
                "{policy:?}: {result}"
            );
            if policy.is_realtime() {
                assert_eq!(result["width"], serde_json::json!(123));
            }
            let counters = heap(&page).await;
            let counters = &counters["moli"]["runtime"];
            for name in [
                "childDefaultContextCount",
                "childBrowsingContextCount",
                "completedChildFrameNavigationLoads",
            ] {
                assert_eq!(counters[name], serde_json::json!(0), "{policy:?}: {name}");
            }
            assert_eq!(
                counters["inspectorContextRegistrationCount"],
                serde_json::json!(root_registrations),
                "retiring a child must remove its inspector registration",
            );
            let current = observation(&page).await;
            assert_eq!(current.vm_creation_id, root.vm_creation_id);
            assert_eq!(current.document, root.document);
            if let Some(pool) = &pool {
                // Reclaiming the root cache is legitimate. No child slot or
                // ticket should accumulate after its real removal completes.
                assert!(current.working_sets <= 1);
                assert_eq!(current.tickets, 1);
                let state = pool.snapshot();
                assert_eq!(state.open_pages, 1);
                assert!(state.live_leases <= 1);
                assert_eq!(state.queued_pages, 0);
                assert_eq!(state.ready_bytes, 0);
                assert!(state.charged_bytes <= pool.limits().document_bytes);
                assert_eq!(state.charged_bytes == 0, state.live_leases == 0);
                let [active] = state.pages.as_slice() else {
                    panic!("only the original Page should remain: {state:?}");
                };
                let original = root_account.as_ref().unwrap();
                assert_eq!(active.token, original.token);
                assert_eq!(active.registration, original.registration);
                assert!(active.open);
                assert_eq!(active.documents, 1);
                assert!(active.demand.is_none());
            } else {
                assert_eq!((current.working_sets, current.tickets), (0, 0));
                assert!(
                    runtime
                        .renderer_owner_handle()
                        .layout_working_set_budget()
                        .is_none()
                );
            }
        }
        tokio::time::timeout(CHILD_RETIREMENT_TIMEOUT, page.close_async())
            .await
            .expect("the Page close should not stall")
            .expect("the Page should close normally");
        if let Some(pool) = &pool {
            let state = pool.snapshot();
            assert_eq!(state.charged_bytes, 0);
            assert_eq!(state.live_leases, 0);
            assert_eq!(state.open_pages, 0);
            assert_eq!(state.queued_pages, 0);
            assert_eq!(state.ready_bytes, 0);
        }
    }
}
