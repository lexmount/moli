use super::webassembly_realm::install_child_webassembly_realm_adapter;
use crate::{
    custom_elements, document_runtime::DomHandle, util::context_host_ptr_from_global_bridge,
};

/// Creates realm-scoped and document-scoped native backing for a new child
/// Window realm. Public WebIDL properties remain owned by shared metadata.
pub(in crate::native_bridge) fn initialize_child_window_realm_environment<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    handle: DomHandle,
) -> anyhow::Result<()> {
    install_child_webassembly_realm_adapter(scope, window);
    rebind_child_window_document_environment(scope, window, handle)
}

/// Refreshes only Document-scoped backing when an initial child LocalWindow
/// keeps an isolated realm across its first commit.
pub(in crate::native_bridge) fn rebind_child_window_document_environment<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    handle: DomHandle,
) -> anyhow::Result<()> {
    install_child_css_runtime_state(scope, window, handle)?;
    custom_elements::rebind_materialized_child_custom_elements_registry(scope, window, handle)
}

fn install_child_css_runtime_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    handle: DomHandle,
) -> anyhow::Result<()> {
    let owner_document = context_host_ptr_from_global_bridge(scope)
        .and_then(|host_ptr| unsafe { &*host_ptr }.child_browsing_context_document_handle(handle));
    crate::context_bootstrap::install_css_runtime_state_for_document(scope, window, owner_document)
}
