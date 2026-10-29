use std::cell::RefCell;

use moli_layout::DocumentLayoutServices;

/// Workers have no DOM layout sidecar. Keep their font collection scoped to
/// the realm instead; a FontFace and its OffscreenCanvas share this service.
type RealmFontServices = RefCell<DocumentLayoutServices>;

pub(in crate::context_bootstrap) fn with_font_services<T>(
    scope: &mut v8::PinScope<'_, '_>,
    consume: impl FnOnce(&mut DocumentLayoutServices) -> T,
) -> T {
    if let Some(host_ptr) = crate::util::context_host_ptr_from_global_bridge(scope) {
        // SAFETY: the bridge belongs to this live ScriptVm. No V8/author calls
        // occur while the document's Rust-only font service is borrowed.
        let host = unsafe { &*host_ptr };
        let child =
            crate::context_bootstrap::child_browsing_context_handle_for_current_realm_scope(scope);
        let document = child
            .and_then(|handle| host.child_browsing_context_document_handle(handle))
            .unwrap_or_else(|| host.document_handle());
        return host.with_font_services(document, consume);
    }
    let context = scope.get_current_context();
    let services = context.get_slot::<RealmFontServices>().unwrap_or_else(|| {
        context.set_slot(RefCell::new(DocumentLayoutServices::new()).into());
        context
            .get_slot::<RealmFontServices>()
            .expect("initialized realm font service")
    });
    consume(&mut services.borrow_mut())
}
