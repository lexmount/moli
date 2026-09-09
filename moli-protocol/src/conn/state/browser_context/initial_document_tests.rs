use super::BrowserContext;
use moli_core::{
    browser::{
        BrowserHandle, BrowserInitialDocumentWaiter, BrowserService, DocumentDecisionProvider,
        DocumentHandle,
        web_contents::{InitialDocumentBuildKey, InitialDocumentInspectionStage},
    },
    page::{DocumentStartScript, PermissionOverrideRegistration},
};
use std::{
    future::Future,
    task::{Context, Poll, Waker},
};

const TARGET: &str = "initial-projection";

fn context() -> (BrowserContext, BrowserHandle, DocumentDecisionProvider) {
    let browser = BrowserService::start().unwrap().handle();
    let provider = browser.register_document_decision_provider().unwrap();
    let mut context = BrowserContext::new_with_browser_for_test(&browser, "initial-context");
    context.bind_page_navigation_engines(Default::default(), None);
    context.set_active_target_id(TARGET);
    context.begin_active_target_initial_empty_document("about:blank".into());
    (context, browser, provider)
}

fn observe(context: &mut BrowserContext) -> BrowserInitialDocumentWaiter {
    context
        .start_initial_document_for_target(TARGET, Default::default(), &Default::default())
        .unwrap()
        .expect("pending native initial construction")
}

async fn inspect(
    context: &BrowserContext,
    browser: &BrowserHandle,
    key: InitialDocumentBuildKey,
    configuration: moli_renderer_v8::RendererPreparedDocumentInspectionConfiguration,
) {
    let (_, mut events) = browser.subscribe().unwrap();
    let contents = context
        .browser_context
        .selected_web_contents_handle()
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Some(claim) = context
                .browser_context
                .claim_initial_document_inspection(contents, key)
                .unwrap()
            {
                if let InitialDocumentInspectionStage::Prepared(endpoint) = &claim.stage {
                    endpoint.start_configure(configuration).await.unwrap();
                    drop(claim);
                    return;
                }
                drop(claim);
            }
            events.recv().await.unwrap();
        }
    })
    .await
    .expect("exact native initial inspection phases");
}

#[tokio::test]
async fn dropping_an_unpolled_observer_preserves_native_construction_and_joiners() {
    let (mut context, browser, _provider) = context();
    let first = observe(&mut context);
    let key = first.key();
    let joined = observe(&mut context);
    assert_eq!(joined.key(), key);
    drop(first);
    inspect(&context, &browser, key, Default::default()).await;
    assert!(joined.wait().await.unwrap().is_none());
    assert_eq!(context.target_document_id(TARGET), Some(key.document()));
    assert!(
        context
            .start_initial_document_for_target(TARGET, Default::default(), &Default::default())
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn navigation_cancels_initial_construction_and_notifies_every_observer() {
    let (mut context, _browser, _provider) = context();
    let first = observe(&mut context);
    let joined = observe(&mut context);
    context.begin_target_document_navigation(TARGET, "next".into());
    assert_eq!(
        joined.wait().await.err().as_deref(),
        Some("InitialDocumentPageBuildCancelled")
    );
    assert_eq!(
        first.wait().await.err().as_deref(),
        Some("InitialDocumentPageBuildCancelled")
    );
    assert!(!context.target_has_loaded_page(TARGET));
}

#[tokio::test]
async fn late_initial_inspection_claim_cannot_resume_or_complete_a_retry() {
    let (mut context, browser, _provider) = context();
    let first = observe(&mut context);
    let first_key = first.key();
    let contents = context.web_contents_handle_for_target(TARGET).unwrap();
    let old_claim = context
        .browser_context
        .claim_initial_document_inspection(contents, first_key)
        .unwrap()
        .unwrap();
    context.clear_document_navigation_state_for_active_target();
    assert_eq!(
        first.wait().await.err().as_deref(),
        Some("InitialDocumentPageBuildCancelled")
    );
    let second = observe(&mut context);
    let second_key = second.key();
    assert_ne!(first_key, second_key);
    let mut joined = Box::pin(observe(&mut context).wait());
    drop(old_claim);
    assert!(matches!(
        joined
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop())),
        Poll::Pending
    ));
    assert!(
        context
            .browser_context
            .claim_initial_document_inspection(contents, first_key)
            .unwrap()
            .is_none()
    );
    inspect(&context, &browser, second_key, Default::default()).await;
    let committed = second.wait().await.unwrap().unwrap();
    assert_eq!(committed.key, second_key);
    context.project_initial_document_commit(committed);
    assert!(joined.await.unwrap().is_none());
    assert_eq!(
        context.target_document_id(TARGET),
        Some(second_key.document())
    );
}

#[tokio::test]
async fn native_initial_commit_survives_projection_loss_and_has_complete_lifecycle() {
    let (mut context, browser, _provider) = context();
    let observation = observe(&mut context);
    let key = observation.key();
    let joined = observe(&mut context);
    inspect(&context, &browser, key, Default::default()).await;
    let projection = context.page_targets.remove(TARGET).unwrap();
    let committed = observation.wait().await.unwrap().unwrap();
    assert!(joined.wait().await.unwrap().is_none());
    assert!(context.owns_web_contents(key.web_contents()));
    let contents = context
        .browser_context
        .selected_web_contents_handle()
        .unwrap();
    assert_eq!(contents.id(), key.web_contents());
    assert_eq!(
        context
            .browser_context
            .document_renderer_residence(DocumentHandle::new(contents, key.document()))
            .unwrap(),
        key.renderer()
    );
    assert!(context.page_targets.insert(projection));
    context.project_initial_document_commit(committed);
    assert_eq!(context.target_document_id(TARGET), Some(key.document()));
    assert!(
        context
            .renderer_document_lifecycle_authoritative_snapshot_for_target(TARGET)
            .unwrap()
            .load
            .is_some()
    );
}

#[tokio::test]
async fn foreign_initial_commit_receipt_cannot_bind_a_peer_projection() {
    let (mut source, browser, _provider) = context();
    let (mut peer, _peer_browser, _peer_provider) = context();
    let observation = observe(&mut source);
    let key = observation.key();
    inspect(&source, &browser, key, Default::default()).await;
    let committed = observation.wait().await.unwrap().unwrap();
    peer.project_initial_document_commit(committed);
    assert!(!peer.target_has_loaded_page(TARGET));
    assert_eq!(peer.target_document_id(TARGET), None);
    assert_eq!(source.target_document_id(TARGET), Some(key.document()));
}

#[tokio::test]
async fn initial_preload_observes_native_admission_policy_before_materialization() {
    let (mut context, browser, _provider) = context();
    let permission = |setting: &str| PermissionOverrideRegistration {
        permission: serde_json::json!({"name": "geolocation"}),
        setting: setting.into(),
        origin: None,
        embedded_origin: None,
    };
    context
        .set_permission_override(permission("granted"))
        .unwrap();
    let observation = observe(&mut context);
    context
        .set_permission_override(permission("denied"))
        .unwrap();
    inspect(&context, &browser, observation.key(),
        moli_renderer_v8::RendererPreparedDocumentInspectionConfiguration {
            root_frame_projection_id: Some(TARGET.into()),
            document_start_scripts: vec![DocumentStartScript {
                registry_key: None, devtools_session: None,
                source: "globalThis.__initialPermission = navigator.permissions.query({name:'geolocation'}).then(permission => permission.state);".into(),
                world_name: None, has_bidi_channel_argument: false, bidi_channel_handoffs: Vec::new(),
            }],
            ..Default::default()
        }).await;
    context.project_initial_document_commit(observation.wait().await.unwrap().unwrap());
    assert_eq!(
        context
            .evaluate_target_expression_for_test(TARGET, "globalThis.__initialPermission", true)
            .await
            .unwrap()["value"],
        "granted",
        "preload must observe the policy frozen by Browser admission, not the later Context override"
    );
}
