use super::*;

#[test]
fn network_owner_retirement_invalidates_retained_webtransport_connectors() {
    let owner = FetchRuntimeOwner::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    let healthy =
        FetchRuntimeOwner::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    let request_handle = owner.handle();
    let connector = request_handle.webtransport_connector();
    let retained = connector.clone();
    let healthy_connector = healthy.handle().webtransport_connector();
    assert!(!connector.is_closed());
    drop(owner);
    assert!(connector.is_closed());
    assert!(retained.is_closed());
    assert!(!healthy_connector.is_closed());
    drop(healthy);
    assert!(healthy_connector.is_closed());
}

#[test]
fn request_shutdown_retires_quic_before_the_semantic_owner_is_joined() {
    let owner = FetchRuntimeOwner::new(&FetchConfig::default(), new_shared_browser_cookie_store());
    let handle = owner.handle();
    let connector = handle.webtransport_connector();
    handle.request_shutdown();
    assert!(connector.is_closed());
    assert!(owner.shutdown().is_clean());
}
