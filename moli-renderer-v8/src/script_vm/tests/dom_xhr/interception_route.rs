use super::*;
use crate::native_bridge::OwnerDispatchScope;
use crate::network_host::controlled_xhr::XhrInterceptionRoute;
use crate::runtime::{
    PageId, RendererDocumentLifecycleJournalHandle, RendererOutputStreamIdentity,
    RendererTurnOutputJournal,
};

fn bind_native_xhr_output(vm: &mut ScriptVm) {
    let page = PageId::new_for_testing(2729);
    vm._context_host
        .borrow_mut()
        .bind_output_journal(RendererTurnOutputJournal::new(
            RendererOutputStreamIdentity::new_page_for_protocol_test(page),
        ));
}

#[test]
fn interception_uses_live_target_policy_after_document_loader_retirement() {
    use crate::SubresourceInterceptionClasses as Classes;
    use crate::types::SubresourceResourceType as Resource;

    let vm = new_storage_test_vm("https://xhr-route.test/");
    let mut host = vm._context_host.borrow_mut();
    let policy = host
        .current_main_document_resource_loader()
        .unwrap()
        .request_client()
        .page_network_policy()
        .clone();
    policy.set_subresource_interception(Classes::IMAGE);
    assert!(host.should_intercept_subresource(Resource::Image));
    host.retire_all_document_resource_loaders();
    assert!(host.current_main_document_resource_loader().is_none());

    policy.set_subresource_interception(Classes::FETCH_LIKE);
    assert!(host.should_intercept_subresource(Resource::Xhr));
    assert!(!host.should_intercept_subresource(Resource::Image));
    policy.set_subresource_interception(Classes::NONE);
    assert!(!host.should_intercept_subresource(Resource::Xhr));
}

#[test]
fn reused_window_adopts_the_new_document_interception_policy() {
    use crate::SubresourceInterceptionClasses as Classes;
    use crate::network::{ResourceRequestClient, context::DocumentResourceLoader};
    use crate::types::SubresourceResourceType as Resource;

    let vm = new_storage_test_vm("https://xhr-route.test/");
    let mut host = vm._context_host.borrow_mut();
    let original = host.current_main_document_resource_loader().unwrap();
    let old_policy = original.request_client().page_network_policy().clone();
    old_policy.set_subresource_interception(Classes::IMAGE);
    let replacement_client =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let replacement_policy = replacement_client.page_network_policy().clone();
    replacement_policy.set_subresource_interception(Classes::FETCH_LIKE);
    let replacement = DocumentResourceLoader::new(
        replacement_client.handle(),
        original.task_runner(),
        original.fetch_context(),
    );

    host.retire_all_document_resource_loaders();
    host.register_main_document_resource_loader(&replacement);
    assert!(host.should_intercept_subresource(Resource::Xhr));
    assert!(!host.should_intercept_subresource(Resource::Image));
    old_policy.set_subresource_interception(Classes::ALL);
    replacement_policy.set_subresource_interception(Classes::NONE);
    assert!(!host.should_intercept_subresource(Resource::Xhr));
    assert!(!host.should_intercept_subresource(Resource::Image));
}

#[test]
fn unrelated_classes_do_not_create_a_native_xhr_controller() {
    use crate::SubresourceInterceptionClasses as Classes;

    let mut vm = new_storage_test_vm("https://xhr-route.test/");
    vm.set_root_document_lifecycle(RendererDocumentLifecycleJournalHandle::new_initial(
        PageId::new_for_testing(2729),
    ));
    bind_native_xhr_output(&mut vm);
    vm.set_fetch_subresource_interception(Classes::IMAGE | Classes::MEDIA | Classes::FONT);
    for owner in [
        OwnerDispatchScope::Top,
        OwnerDispatchScope::LightweightPopup(u64::MAX),
    ] {
        assert!(matches!(
            vm._context_host.borrow_mut().xhr_interception_route(owner),
            Ok(XhrInterceptionRoute::Ordinary)
        ));
    }
    assert_eq!(vm.take_pending_subresource_fetch_infos().len(), 0);

    vm.set_fetch_subresource_interception(Classes::IMAGE | Classes::FETCH_LIKE);
    assert!(matches!(
        vm._context_host
            .borrow_mut()
            .xhr_interception_route(OwnerDispatchScope::Top),
        Ok(XhrInterceptionRoute::Native(_))
    ));
    assert!(matches!(
        vm._context_host
            .borrow_mut()
            .xhr_interception_route(OwnerDispatchScope::LightweightPopup(u64::MAX)),
        Err("XHR interception Document policy is unavailable")
    ));
}

#[test]
fn native_xhr_route_rejects_missing_document_policy_instead_of_network_fallback() {
    let mut vm = new_storage_test_vm("https://xhr-route.test/");
    vm.set_root_document_lifecycle(RendererDocumentLifecycleJournalHandle::new_initial(
        PageId::new_for_testing(2729),
    ));
    bind_native_xhr_output(&mut vm);
    vm.set_fetch_subresource_interception(crate::SubresourceInterceptionClasses::FETCH_LIKE);
    let mut host = vm._context_host.borrow_mut();
    assert!(matches!(
        host.xhr_interception_route(OwnerDispatchScope::Top),
        Ok(XhrInterceptionRoute::Native(_))
    ));
    // Native output exists, but this retired popup no longer has a policy.
    assert!(matches!(
        host.xhr_interception_route(OwnerDispatchScope::LightweightPopup(u64::MAX)),
        Err("XHR interception Document policy is unavailable")
    ));
}

#[test]
fn native_xhr_route_rejects_missing_document_identity() {
    let mut vm = new_storage_test_vm_without_page_residence("https://xhr-route.test/");
    bind_native_xhr_output(&mut vm);
    vm.set_fetch_subresource_interception(crate::SubresourceInterceptionClasses::FETCH_LIKE);
    assert!(matches!(
        vm._context_host
            .borrow_mut()
            .xhr_interception_route(OwnerDispatchScope::Top),
        Err("XHR interception Document is no longer active")
    ));
}

#[test]
fn standalone_xhr_uses_explicit_async_compatibility_without_network_dispatch() {
    let mut vm = new_storage_test_vm("https://xhr-route.test/");
    vm.set_fetch_subresource_interception(crate::SubresourceInterceptionClasses::FETCH_LIKE);
    assert!(matches!(
        vm._context_host
            .borrow_mut()
            .xhr_interception_route(OwnerDispatchScope::Top),
        Ok(XhrInterceptionRoute::Compatibility)
    ));
    vm.eval("globalThis.xhr = new XMLHttpRequest(); xhr.open('GET', '/held'); xhr.send();")
        .unwrap();
    let pending = vm.take_pending_subresource_fetch_infos();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].url.as_str(), "https://xhr-route.test/held");
    assert!(
        vm._context_host
            .borrow()
            .pending_subresource_native_inputs(pending[0].internal_id)
            .is_some()
    );
    assert_eq!(vm.eval("String(xhr.readyState)").unwrap(), "1");
    vm.eval("xhr.abort()").unwrap();
}
