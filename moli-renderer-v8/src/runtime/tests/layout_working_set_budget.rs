use super::*;
use crate::layout_working_set_budget::{
    BudgetClock, BudgetLimits, BudgetTiming, LayoutWorkingSetBudget, PageBudgetSnapshot,
};
use moli_page_types::{LayoutConfiguration, LayoutPolicy};
use std::sync::Arc;
use std::time::Instant;

mod child_retirement;
mod close;
mod owner_pressure;

// Accounting assertions must not race a real five-second idle deadline when
// the workspace suite is busy. Rendering still uses its normal owner clock.
#[derive(Debug)]
struct FixedBudgetClock(Instant);

impl BudgetClock for FixedBudgetClock {
    fn now(&self) -> Instant {
        self.0
    }
}

fn budget() -> LayoutWorkingSetBudget {
    LayoutWorkingSetBudget::with_clock(
        BudgetLimits::default(),
        BudgetTiming::default(),
        Arc::new(FixedBudgetClock(Instant::now())),
    )
    .unwrap()
}

fn realtime_runtime(pool: &LayoutWorkingSetBudget) -> JsRuntimeOwner {
    let runtime = JsRuntime::initialize();
    configure(&runtime, LayoutPolicy::Realtime, Some(pool.clone()));
    runtime
}

fn configure(runtime: &JsRuntime, policy: LayoutPolicy, pool: Option<LayoutWorkingSetBudget>) {
    runtime
        .renderer_owner_handle()
        .configure_layout_configuration_with_budget(
            LayoutConfiguration {
                policy,
                ..LayoutConfiguration::default()
            },
            pool,
        )
        .unwrap();
}

const DOCUMENT: &str = "<!doctype html><body style='margin:0'><div id=target style='width:120px;height:20px'></div><script>window.initialWidth=document.getElementById('target').offsetWidth</script>";

async fn page(runtime: &JsRuntime, loader: &ResourceRequestClient) -> RendererPageHandle {
    create_test_html_page(
        runtime,
        loader,
        url::Url::parse("https://working-set-budget.test/page").unwrap(),
        DOCUMENT,
    )
    .await
}

async fn evaluate(page: &RendererPageHandle, expression: &str) -> serde_json::Value {
    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: expression.to_owned(),
            await_promise: false,
        })
        .await
        .unwrap();
    renderer_json_value(reply).expect("evaluation should return an owned value")
}

async fn json_result(page: &RendererPageHandle, body: &str) -> serde_json::Value {
    let result = evaluate(page, &format!("JSON.stringify((()=>{{{body}}})())")).await;
    serde_json::from_str(result.as_str().expect("JSON result must be a string")).unwrap()
}

fn account(pool: &LayoutWorkingSetBudget, page: &RendererPageHandle) -> PageBudgetSnapshot {
    pool.snapshot()
        .pages
        .into_iter()
        .find(|entry| {
            entry.token.local_host_id() == page.owner_local_host_id()
                && entry.token.page_id() == page.renderer_page_id()
        })
        .expect("the complete renderer Page token must be registered")
}

async fn completed_body(source: &str) -> ExternalRawDocumentBodyStream {
    let (completion_tx, completion_rx) = oneshot::channel();
    let (body_tx, body) = ExternalRawDocumentBodyStream::channel(completion_rx);
    body_tx.send(source.as_bytes().to_vec()).await.unwrap();
    drop(body_tx);
    completion_tx.send(Ok(())).unwrap();
    body
}

async fn replacement(
    runtime: &JsRuntime,
    page: &RendererPageHandle,
    loader: &ResourceRequestClient,
    source: &str,
) -> PreparedRendererDocument {
    let reservation = page
        .reserve_replacement_document_for_navigation()
        .await
        .unwrap();
    let url = url::Url::parse("https://working-set-budget.test/replacement").unwrap();
    runtime
        .prepare_streaming_raw_document_from_external_body(
            reservation,
            url.clone(),
            url,
            None,
            false,
            0,
            Vec::new(),
            200,
            vec![("content-type".to_owned(), b"text/html".to_vec())],
            loader,
            crate::RendererWebStorageHandles::ephemeral(),
            completed_body(source).await,
            false,
            PageVmInitStage::Load,
            RendererReplyBoundary::DocumentCommit,
            RendererTopLevelNavigationDispatch::DelegateToBrowser,
            RendererNavigationReplyPolicy::ReturnWithPendingNavigation,
            None,
            None,
            crate::RendererDocumentOptions::default(),
        )
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn realtime_working_set_registration_tracks_real_pages_and_owner_identity() {
    let pool = budget();
    let context = crate::runtime::RendererBrowserContextRuntime::new();
    let access = context.owner_access();
    let first = JsRuntime::initialize_with_browser_context_owner_access(&access).unwrap();
    let second = JsRuntime::initialize_with_browser_context_owner_access(&access).unwrap();
    let mock = JsRuntime::initialize_with_browser_context_owner_access(&access).unwrap();
    let on_demand = JsRuntime::initialize_with_browser_context_owner_access(&access).unwrap();
    configure(&first, LayoutPolicy::Realtime, Some(pool.clone()));
    configure(&second, LayoutPolicy::Realtime, None);
    configure(&mock, LayoutPolicy::Mock, None);
    configure(&on_demand, LayoutPolicy::OnDemand, None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut first_page = page(&first, &loader).await;
    let mut second_page = page(&second, &loader).await;
    let mut mock_page = page(&mock, &loader).await;
    let mut on_demand_page = page(&on_demand, &loader).await;
    assert_eq!(
        first_page.renderer_page_id(),
        second_page.renderer_page_id()
    );
    assert_ne!(
        first_page.owner_local_host_id(),
        second_page.owner_local_host_id()
    );
    assert_eq!(pool.snapshot().open_pages, 2);
    let first_account = account(&pool, &first_page);
    let second_account = account(&pool, &second_page);
    assert_ne!(first_account.registration, second_account.registration);
    assert!(first_account.charged_bytes > 0);
    assert!(second_account.charged_bytes > 0);
    for page in [&first_page, &second_page] {
        assert_eq!(evaluate(page, "initialWidth").await, serde_json::json!(120));
        assert_eq!(
            RendererPageTestingHandle::new_for_testing(page)
                .layout_working_set_state_async()
                .await
                .unwrap(),
            (1, 1)
        );
    }
    for page in [&mock_page, &on_demand_page] {
        assert_eq!(
            RendererPageTestingHandle::new_for_testing(page)
                .layout_working_set_state_async()
                .await
                .unwrap(),
            (0, 0),
            "a legacy owner must not register just because its Context has a pool"
        );
    }
    mock_page.close_async().await.unwrap();
    on_demand_page.close_async().await.unwrap();
    assert_eq!(pool.snapshot().open_pages, 2);
    first_page.close_async().await.unwrap();
    assert_eq!(pool.snapshot().open_pages, 1);
    assert_eq!(
        account(&pool, &second_page).registration,
        second_account.registration
    );
    second_page.close_async().await.unwrap();
    let closed = pool.snapshot();
    assert_eq!(closed.open_pages, 0);
    assert_eq!(closed.live_leases, 0);
    assert_eq!(closed.charged_bytes, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn realtime_working_set_initial_reservation_cancellation_releases_registration() {
    let pool = budget();
    let runtime = realtime_runtime(&pool);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let prepared = prepare_test_external_raw_document(
        &runtime,
        &loader,
        url::Url::parse("https://working-set-budget.test/uncommitted").unwrap(),
        completed_body(DOCUMENT).await,
    )
    .await;
    let snapshot = pool.snapshot();
    assert_eq!(snapshot.open_pages, 1);
    assert_eq!(
        snapshot.pages[0].token.local_host_id(),
        prepared.token().local_host_id()
    );
    assert_eq!(
        snapshot.pages[0].token.page_id(),
        prepared.token().page_id()
    );
    prepared.cancel().await.unwrap();
    let canceled = pool.snapshot();
    assert_eq!(canceled.open_pages, 0);
    assert_eq!(canceled.charged_bytes, 0);
    assert_eq!(canceled.live_leases, 0);
    let mut survivor = page(&runtime, &loader).await;
    assert_eq!(
        evaluate(&survivor, "initialWidth").await,
        serde_json::json!(120)
    );
    assert_eq!(pool.snapshot().open_pages, 1);
    survivor.close_async().await.unwrap();
    assert_eq!(pool.snapshot().open_pages, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn realtime_working_set_registration_survives_replacement_and_document_open() {
    let pool = budget();
    let runtime = realtime_runtime(&pool);
    let (tx, mut output) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(tx);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&runtime, &loader).await;
    let testing = RendererPageTestingHandle::new_for_testing(&page);
    let original_view = testing.renderer_page_view_async().await.unwrap();
    let original_account = account(&pool, &page);
    assert!(original_account.charged_bytes > 0);
    let prepared = replacement(&runtime, &page, &loader, DOCUMENT).await;
    assert_eq!(pool.snapshot().open_pages, 1);
    assert_eq!(
        account(&pool, &page).registration,
        original_account.registration
    );
    prepared.cancel().await.unwrap();
    assert_eq!(
        account(&pool, &page).registration,
        original_account.registration
    );
    assert_eq!(
        testing
            .renderer_page_view_async()
            .await
            .unwrap()
            .vm_creation_id,
        original_view.vm_creation_id
    );

    let prepared = replacement(&runtime, &page, &loader, DOCUMENT).await;
    let permit = prepared.issue_commit_permit();
    let committed = prepared.commit_page_replacement(permit).await.unwrap();
    let (_, _, artifacts) = page.adopt_document_replacement(committed).unwrap();
    if let Some(continuation) = page.take_committed_document_post_response_continuation() {
        continuation.release();
    }
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let publication = output.recv().await.expect("replacement output stays open");
            if publication_document_lifecycle_events(&publication).any(|event| {
                event.document == artifacts.active_document
                    && event.kind
                        == RendererDocumentLifecycleEventKind::Milestone(
                            RendererDocumentLifecycleMilestone::Load,
                        )
            }) {
                break;
            }
        }
    })
    .await
    .expect("replacement should reach Load on its stable Page");
    assert_ne!(
        testing
            .renderer_page_view_async()
            .await
            .unwrap()
            .vm_creation_id,
        original_view.vm_creation_id
    );
    assert_eq!(pool.snapshot().open_pages, 1);
    assert_eq!(
        account(&pool, &page).registration,
        original_account.registration
    );
    assert_eq!(
        json_result(&page, "const el=document.getElementById('target');const before=el.offsetWidth;el.style.width='230px';return [before,el.offsetWidth]").await,
        serde_json::json!([120, 230])
    );
    assert_eq!(
        testing.layout_working_set_state_async().await.unwrap(),
        (1, 1)
    );
    assert_eq!(
        json_result(&page, "document.open();document.write('<!doctype html><div id=fresh style=width:180px;height:20px></div>');document.close();const el=document.getElementById('fresh');const before=el.offsetWidth;el.style.width='250px';return [before,el.offsetWidth]").await,
        serde_json::json!([180, 250])
    );
    assert_eq!(
        account(&pool, &page).registration,
        original_account.registration
    );
    assert_eq!(
        testing.layout_working_set_state_async().await.unwrap(),
        (1, 1)
    );
    page.close_async().await.unwrap();
    let closed = pool.snapshot();
    assert_eq!(closed.open_pages, 0);
    assert_eq!(closed.charged_bytes, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn related_initial_popup_uses_the_owners_layout_configuration_before_adoption() {
    for (policy, expected) in [
        (LayoutPolicy::Mock, serde_json::json!([100, 100, 100])),
        (LayoutPolicy::OnDemand, serde_json::json!([170, 170, 170])),
        (LayoutPolicy::Realtime, serde_json::json!([170, 230, 230])),
    ] {
        let runtime = JsRuntime::initialize();
        runtime
            .renderer_owner_handle()
            .configure_layout_configuration(LayoutConfiguration {
                policy,
                scrollbars_hidden: policy.uses_real_layout(),
            })
            .unwrap();
        runtime
            .browser_context_runtime()
            .delegate_auxiliary_document_responses_to_browser();
        let (tx, mut output) = renderer_external_activity_test_channel();
        runtime.set_renderer_output_transport_sender(tx);
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        let mut parent = create_test_html_page_with_navigation_dispatch(
            &runtime,
            &loader,
            url::Url::parse("https://working-set-budget.test/creator").unwrap(),
            DOCUMENT,
            RendererTopLevelNavigationDispatch::DelegateToBrowser,
        )
        .await;
        output.drain();
        // The popup has no protocol target yet. The first read establishes
        // its own layout; a second write/read distinguishes OnDemand's sample
        // from Realtime, and clientWidth observes the owner's scrollbar option.
        assert_eq!(
            json_result(&parent, "const child=open('about:blank','initial-policy');const el=child.document.createElement('div');el.style.cssText='width:170px;height:20px;overflow:scroll';child.document.body.append(el);const before=el.offsetWidth;el.style.width='230px';return [before,el.offsetWidth,el.clientWidth]").await,
            expected,
            "initial popup configuration for {policy:?}",
        );
        let publications = output.drain();
        assert_eq!(popup_activations_for_page(&publications, &parent).len(), 1);
        drop(publications);
        parent.close_async().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn realtime_working_set_popup_has_its_own_registration_before_adoption() {
    let pool = budget();
    let runtime = realtime_runtime(&pool);
    runtime
        .browser_context_runtime()
        .delegate_auxiliary_document_responses_to_browser();
    let (tx, mut output) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(tx);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut parent = create_test_html_page_with_navigation_dispatch(
        &runtime,
        &loader,
        url::Url::parse("https://working-set-budget.test/creator").unwrap(),
        DOCUMENT,
        RendererTopLevelNavigationDispatch::DelegateToBrowser,
    )
    .await;
    let parent_account = account(&pool, &parent);
    output.drain();
    assert_eq!(
        json_result(&parent, "window.child=open('about:blank','budget-child');const el=child.document.createElement('div');el.id='child-box';el.style.cssText='width:170px;height:20px';child.document.body.append(el);return [el.offsetWidth,child.closed]").await,
        serde_json::json!([170, false])
    );
    let publications = output.drain();
    let activations = popup_activations_for_page(&publications, &parent);
    assert_eq!(activations.len(), 1);
    let pending = activations[0].pending_auxiliary_page().unwrap();
    let initial = pool.snapshot();
    assert_eq!(initial.open_pages, 2);
    let child_account = initial
        .pages
        .iter()
        .find(|entry| {
            entry.token.local_host_id() == pending.page_reservation().local_host_id()
                && entry.token.page_id() == pending.page_reservation().page_id()
        })
        .unwrap()
        .clone();
    assert_ne!(child_account.registration, parent_account.registration);
    assert!(
        child_account.charged_bytes > 0,
        "creator-side geometry must already use the popup's managed budget"
    );
    let crate::RendererPopupActivationParts {
        session_storage_store,
        initial_empty_document_storage_key,
        ..
    } = activations[0].clone().into_parts();
    let blank = url::Url::parse("about:blank").unwrap();
    let (mut child, _, _, _, _) = runtime
        .start_create_html_page_from_response(
            pending.page_reservation(),
            blank.clone(),
            blank,
            None,
            false,
            0,
            200,
            vec![("content-type".to_owned(), b"text/html".to_vec())],
            &loader,
            crate::RendererWebStorageHandles::new(
                crate::new_shared_web_storage_store(),
                session_storage_store.unwrap(),
            ),
            "<!doctype html><body></body>".to_owned(),
            initial_empty_document_storage_key,
            RendererTopLevelNavigationDispatch::DelegateToBrowser,
            crate::RendererDocumentOptions::default(),
        )
        .unwrap()
        .await_ready()
        .await
        .unwrap();
    assert_eq!(pool.snapshot().open_pages, 2);
    assert_eq!(
        account(&pool, &child).registration,
        child_account.registration
    );
    assert_eq!(
        evaluate(&child, "document.getElementById('child-box').offsetWidth").await,
        serde_json::json!(170)
    );
    parent.close_async().await.unwrap();
    assert_eq!(pool.snapshot().open_pages, 1);
    assert_eq!(
        account(&pool, &child).registration,
        child_account.registration
    );
    assert_eq!(
        json_result(&child, "const el=document.getElementById('child-box');el.style.width='210px';return el.offsetWidth").await,
        serde_json::json!(210)
    );
    child.close_async().await.unwrap();
    let closed = pool.snapshot();
    assert_eq!(closed.open_pages, 0);
    assert_eq!(closed.charged_bytes, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn realtime_working_set_abandoned_popup_releases_only_its_registration() {
    let pool = budget();
    let runtime = realtime_runtime(&pool);
    runtime
        .browser_context_runtime()
        .delegate_auxiliary_document_responses_to_browser();
    let (tx, mut output) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(tx);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut parent = create_test_html_page_with_navigation_dispatch(
        &runtime,
        &loader,
        url::Url::parse("https://working-set-budget.test/creator").unwrap(),
        DOCUMENT,
        RendererTopLevelNavigationDispatch::DelegateToBrowser,
    )
    .await;
    let original = account(&pool, &parent);
    output.drain();
    assert_eq!(
        evaluate(
            &parent,
            "window.abandoned=open('about:blank','abandoned');abandoned.closed"
        )
        .await,
        serde_json::json!(false)
    );
    let publications = output.drain();
    let activations = popup_activations_for_page(&publications, &parent);
    assert_eq!(activations.len(), 1);
    let pending = activations[0].pending_auxiliary_page().unwrap();
    let retained = pending.clone();
    drop(activations);
    drop(publications);
    drop(pending);
    assert_eq!(pool.snapshot().open_pages, 2);
    drop(retained);
    // Final capability Drop enqueues cancellation on this owner's command
    // queue. The following acknowledged command is a deterministic barrier.
    RendererPageTestingHandle::new_for_testing(&parent)
        .current_page_state_async()
        .await
        .unwrap();
    assert_eq!(pool.snapshot().open_pages, 1);
    assert_eq!(account(&pool, &parent).registration, original.registration);
    assert_eq!(
        evaluate(&parent, "abandoned.closed").await,
        serde_json::json!(true)
    );
    parent.close_async().await.unwrap();
    assert_eq!(pool.snapshot().open_pages, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn realtime_working_set_registration_exhaustion_disables_retention_not_geometry() {
    let pool = budget();
    pool.set_next_id_for_test(u64::MAX);
    let runtime = realtime_runtime(&pool);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&runtime, &loader).await;
    let testing = RendererPageTestingHandle::new_for_testing(&page);
    assert_eq!(
        json_result(&page, "const el=document.getElementById('target');el.style.width='210px';return [initialWidth,el.offsetWidth,el.offsetWidth]").await,
        serde_json::json!([120, 210, 210])
    );
    assert_eq!(
        testing.layout_working_set_state_async().await.unwrap(),
        (0, 0)
    );
    assert_eq!(
        json_result(&page, "document.open();document.write('<!doctype html><div id=fresh style=width:180px;height:20px></div>');document.close();const el=document.getElementById('fresh');const before=el.offsetWidth;el.style.width='250px';return [before,el.offsetWidth]").await,
        serde_json::json!([180, 250])
    );
    assert_eq!(
        testing.layout_working_set_state_async().await.unwrap(),
        (0, 0)
    );
    let snapshot = pool.snapshot();
    assert_eq!(snapshot.open_pages, 0);
    assert_eq!(snapshot.live_leases, 0);
    assert_eq!(snapshot.charged_bytes, 0);
    page.close_async().await.unwrap();
}
