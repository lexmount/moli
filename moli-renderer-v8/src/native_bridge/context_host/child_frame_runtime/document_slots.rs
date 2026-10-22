use crate::context_bootstrap::sync_selection_owner_document_for_window;
use crate::native_bridge::document::set_document_associated_window;

pub(in crate::native_bridge::context_host::child_frame_runtime) fn sync_child_document_window_slots<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
    window: v8::Local<'s, v8::Object>,
) {
    set_document_associated_window(scope, document, window);
    sync_selection_owner_document_for_window(scope, window, document);
}
