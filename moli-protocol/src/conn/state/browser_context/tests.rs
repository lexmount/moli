use super::*;
use std::sync::Arc;

#[test]
fn context_browser_identity_preserves_independent_inputs_and_update_time_fallback() {
    use moli_browser_profile::BrowserIdentityProfile;

    let fallback = BrowserIdentityProfile::new("Base-UA", "en-US");
    let next_fallback = BrowserIdentityProfile::new("Next-UA", "de-DE");
    let combined = BrowserIdentityProfile::new("Context-UA", "fr-FR");
    for locale_first in [false, true] {
        let mut context = BrowserContext::new("CTX-identity".into());
        assert!(context.default_browser_identity_override().is_none());
        if locale_first {
            context.set_default_locale_override(Some("fr-FR".into()), &fallback);
            assert_eq!(
                context.reported_active_user_agent_override(),
                Some("Base-UA")
            );
            context.set_default_user_agent_override(Some("Context-UA".into()), &fallback);
        } else {
            context.set_default_user_agent_override(Some("Context-UA".into()), &fallback);
            context.set_default_locale_override(Some("fr-FR".into()), &fallback);
        }
        let snapshot = context.default_browser_identity_override_owned().unwrap();
        assert_eq!(snapshot, combined);
        assert_eq!(
            context
                .effective_active_browser_identity_override_owned()
                .as_ref(),
            Some(&combined)
        );
        assert_eq!(
            context.emulation_defaults().locale.as_deref(),
            Some("fr-FR")
        );
        assert!(!context.has_active_target());

        context.set_default_user_agent_override(None, &next_fallback);
        assert_eq!(
            context.default_browser_identity_override(),
            Some(&BrowserIdentityProfile::new("Next-UA", "fr-FR"))
        );
        assert_eq!(
            context.reported_active_user_agent_override(),
            Some("Next-UA")
        );
        assert_eq!(
            context.emulation_defaults().locale.as_deref(),
            Some("fr-FR")
        );
        context.set_default_locale_override(None, &next_fallback);
        assert!(context.default_browser_identity_override().is_none());
        assert!(context.reported_active_user_agent_override().is_none());
        assert!(context.emulation_defaults().locale.is_none());

        context.set_default_user_agent_override(Some("Context-UA".into()), &fallback);
        context.set_default_locale_override(Some("fr-FR".into()), &fallback);
        context.set_default_locale_override(None, &next_fallback);
        assert_eq!(
            context.default_browser_identity_override(),
            Some(&BrowserIdentityProfile::new("Context-UA", "de-DE"))
        );
        assert!(context.emulation_defaults().locale.is_none());
        assert_eq!(snapshot, combined);
        context.set_default_user_agent_override(None, &next_fallback);
        assert!(context.default_browser_identity_override().is_none());
    }
}

#[test]
fn context_browser_identity_outlives_projection() {
    use moli_browser_profile::BrowserIdentityProfile;

    let expected = BrowserIdentityProfile::new("Context-UA", "fr-FR");
    let physical = {
        let mut projection =
            BrowserContext::new_with_page_for_test("CTX-identity", "page-identity");
        projection.attach_active_session("session-identity");
        projection.set_default_user_agent_override(Some("Context-UA".into()), &Default::default());
        projection.set_default_locale_override(Some("fr-FR".into()), &Default::default());
        assert_eq!(
            projection
                .effective_active_browser_identity_override_owned()
                .as_ref(),
            Some(&expected)
        );
        assert_eq!(
            projection.detach_active_session().as_deref(),
            Some("session-identity")
        );
        projection.physical
    };
    assert_eq!(physical.browser_identity_override.as_ref(), Some(&expected));
    assert_eq!(physical.emulation_defaults.locale.as_deref(), Some("fr-FR"));
    let other = BrowserContext::new("CTX-other".into());
    assert!(other.default_browser_identity_override().is_none());
}

#[test]
fn context_emulation_defaults_outlive_projection_without_inherited_mirrors() {
    let metrics = EmulatedDeviceMetrics {
        width: 640,
        height: 480,
        device_scale_factor: 2.0,
        screen_width: 1024,
        screen_height: 768,
    };
    let resized = EmulatedDeviceMetrics {
        width: 800,
        ..metrics.clone()
    };
    let (physical, snapshot) = {
        let mut projection =
            BrowserContext::new_with_page_for_test("CTX-emulation", "page-emulation");
        projection.attach_active_session("session-emulation");
        projection.set_default_locale_override(Some("fr-FR".into()), &Default::default());
        projection.set_default_timezone_override(Some("Asia/Tokyo".into()));
        projection.set_default_network_conditions(Some(EmulatedNetworkConditions::offline()));
        projection.set_default_geolocation_override(Some(
            EmulatedGeolocationOverrideState::PositionUnavailable,
        ));
        assert!(!projection.set_default_device_metrics(metrics.clone()));
        let snapshot = projection.emulation_defaults().clone();

        projection.set_default_timezone_override(None);
        projection.set_default_network_conditions(None);
        projection.global_network_conditions = Some(EmulatedNetworkConditions::offline());
        assert!(projection.effective_active_network_offline());
        assert!(projection.set_default_device_metrics(resized.clone()));
        assert_eq!(
            projection.detach_active_session().as_deref(),
            Some("session-emulation")
        );
        (projection.physical, snapshot)
    };
    assert_eq!(
        snapshot,
        ContextEmulationDefaults {
            locale: Some("fr-FR".into()),
            timezone: Some("Asia/Tokyo".into()),
            network_conditions: Some(EmulatedNetworkConditions::offline()),
            geolocation: Some(EmulatedGeolocationOverrideState::PositionUnavailable),
            device_metrics: Some(metrics),
        }
    );
    assert_eq!(
        physical.emulation_defaults,
        ContextEmulationDefaults {
            timezone: None,
            network_conditions: None,
            device_metrics: Some(resized),
            ..snapshot
        }
    );
    let other = BrowserContext::new("CTX-other".into());
    assert_eq!(
        other.emulation_defaults(),
        &ContextEmulationDefaults::default()
    );
}

#[test]
fn physical_storage_operations_outlive_protocol_projection() {
    let origin = url::Url::parse("https://physical.test").unwrap();
    let origin_key = origin.origin().ascii_serialization();
    let key_a = moli_storage_key::partitioned_storage_key(&origin_key, "https://top-a.test");
    let key_b = moli_storage_key::partitioned_storage_key(&origin_key, "https://top-b.test");
    let sibling = url::Url::parse("https://sibling.test").unwrap();
    let sibling_origin = sibling.origin().ascii_serialization();
    let sibling_key = moli_storage_key::MoliStorageKey::first_party_from_url(&sibling, None)
        .serialized_storage_key();
    let mut physical = {
        let projection = BrowserContext::new_with_page_for_test("CTX-storage", "page-storage");
        {
            let mut store = projection.web_storage_store_for_test().lock();
            assert!(store.set_item(&key_a, "local", "aaa"));
            assert!(store.set_item(&key_b, "local", "bb"));
            assert!(store.set_item(&sibling_key, "local", "c"));
        }
        projection.physical
    };
    let partition = &mut physical.storage_partition;
    assert_eq!(
        partition.usage_for_origin(&origin_key).unwrap(),
        OriginStorageUsage {
            local_storage_usage: 5,
            indexed_db_usage: 0,
            storage_buckets_usage: 0,
            total_usage: 5,
        }
    );
    let options = SiteDataClearOptions {
        local_storage: true,
        ..Default::default()
    };
    let key = moli_storage_key::deserialize_serialized_storage_key(&key_a).unwrap();
    partition
        .clear_site_data_for_storage_key(&key, options)
        .unwrap();
    assert_eq!(
        partition.usage_for_origin(&origin_key).unwrap().total_usage,
        2
    );
    {
        let mut store = partition.web_storage_store().lock();
        assert_eq!(store.get_item(&key_a, "local"), None);
        assert_eq!(store.get_item(&key_b, "local"), Some("bb".into()));
        assert!(store.set_item(&key_a, "local", "aaa"));
    }
    partition
        .clear_site_data_for_origin(&origin, options)
        .unwrap();
    assert_eq!(
        partition.usage_for_origin(&origin_key).unwrap().total_usage,
        0
    );
    assert_eq!(
        partition
            .usage_for_origin(&sibling_origin)
            .unwrap()
            .total_usage,
        1
    );
    let mut store = partition.web_storage_store().lock();
    assert_eq!(store.get_item(&key_a, "local"), None);
    assert_eq!(store.get_item(&key_b, "local"), None);
    assert_eq!(store.get_item(&sibling_key, "local"), Some("c".into()));
}

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
