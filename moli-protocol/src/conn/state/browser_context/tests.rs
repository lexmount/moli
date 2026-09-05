use super::*;
use std::sync::Arc;

#[test]
fn context_network_policy_outlives_page_and_protocol_projection() {
    let expected = ContextNetworkPolicy {
        http_proxy: Some("http://proxy.example:8080".into()),
        http_no_proxy: Some(String::new()),
        tls_verify_host: Some(false),
    };
    let (physical, before_tls_change) = {
        let mut projection = BrowserContext::new_with_page_for_test("CTX-policy", "page-policy");
        projection.attach_active_session("session-policy");
        projection.set_network_policy(expected.clone());
        let before_tls_change = projection.network_policy().clone();
        // A field-level update must not reinstall an old proxy snapshot.
        projection.set_tls_verify_host_override(true);
        assert_eq!(
            projection.detach_active_session().as_deref(),
            Some("session-policy")
        );
        (projection.physical, before_tls_change)
    };
    assert_eq!(before_tls_change, expected);
    assert_eq!(
        physical.network_policy,
        ContextNetworkPolicy {
            tls_verify_host: Some(true),
            ..expected
        }
    );
    let other = BrowserContext::new("CTX-other".into());
    assert_ne!(physical.id, other.browser_context_id());
    assert_eq!(other.network_policy(), &ContextNetworkPolicy::default());
}

#[test]
fn physical_context_storage_and_runtime_outlive_protocol_projection() {
    let (mut physical, id, runtime_id, local_storage) = {
        let mut projection = BrowserContext::new_with_page_for_test("CTX-owner", "page-owner");
        projection.bind_page_navigation_engines(NavigationRuntimeConfig::default(), None);
        projection.set_storage_quota_override("https://example.test".into(), 123.0);
        let id = projection.browser_context_id();
        let runtime_id = projection.renderer_runtime().id();
        let local_storage = projection.web_storage_store_for_test().clone();
        // Moving the sole Browser owner out lets the protocol shell and its
        // legacy embedded page/engine go away without retiring the context.
        (projection.physical, id, runtime_id, local_storage)
    };
    assert_eq!(physical.id, id);
    assert_eq!(physical.renderer_runtime().id(), runtime_id);
    assert!(Arc::ptr_eq(
        physical.storage_partition.web_storage_store(),
        &local_storage
    ));
    assert_eq!(
        physical
            .storage_partition
            .storage_quota_for_origin("https://example.test"),
        (123.0, true)
    );
    physical
        .storage_partition
        .clear_storage_quota_override("https://example.test");
    assert!(
        !physical
            .storage_partition
            .storage_quota_for_origin("https://example.test")
            .1
    );
    let engine = physical.new_page_navigation_engine(
        physical
            .page_navigation_runtime_config
            .clone()
            .expect("Browser-owned configuration"),
    );
    let access = physical.renderer_runtime_owner_access();
    drop(engine);
    drop(physical);
    assert!(
        NavigationEngine::new_with_runtime_config_and_browser_context_access(
            NavigationRuntimeConfig::default(),
            access,
        )
        .is_err(),
        "a retained runtime handle must not resurrect a retired Browser context"
    );
}

#[test]
fn context_runtime_teardown_handoff_retains_exactly_one_owner() {
    let mut projection = BrowserContext::new("CTX-teardown".into());
    let access = projection.renderer_runtime_owner_access();
    let mut root = projection
        .take_renderer_runtime_owner_for_teardown()
        .unwrap();
    assert!(
        projection
            .take_renderer_runtime_owner_for_teardown()
            .is_none()
    );
    drop(projection);

    let engine = NavigationEngine::new_with_runtime_config_and_browser_context_access(
        NavigationRuntimeConfig::default(),
        access.clone(),
    )
    .expect("the moved teardown participant is still the live owner");
    drop(engine);
    root.shutdown_and_join();
    assert!(
        NavigationEngine::new_with_runtime_config_and_browser_context_access(
            NavigationRuntimeConfig::default(),
            access,
        )
        .is_err()
    );
}
