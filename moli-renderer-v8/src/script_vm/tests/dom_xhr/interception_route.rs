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
fn native_xhr_route_rejects_missing_document_policy_instead_of_network_fallback() {
    let mut vm = new_storage_test_vm("https://xhr-route.test/");
    vm.set_root_document_lifecycle(RendererDocumentLifecycleJournalHandle::new_initial(
        PageId::new_for_testing(2729),
    ));
    bind_native_xhr_output(&mut vm);
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Xhr));
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
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Xhr));
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
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Xhr));
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
