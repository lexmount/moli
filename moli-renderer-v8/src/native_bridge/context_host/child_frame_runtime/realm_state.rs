//! Typed child-Window realm initialization.
//!
//! Context creation stays in `isolated_world`; this module owns the common
//! default/isolated state transaction. Public WebIDL descriptors are installed
//! by the shared Window template and must not be added here.

mod initialize;
mod model;
mod snapshot;

pub(super) use initialize::initialize_child_window_realm_state;
pub(super) use model::{ChildWindowRealmInit, WindowWorldKind};

impl crate::native_bridge::JsContextHost {
    pub(crate) fn rebind_child_window_realm_document_state<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        global: v8::Local<'s, v8::Object>,
        handle: crate::document_runtime::DomHandle,
        browsing_context_id: crate::frame_owner_model::BrowsingContextId,
        expected_retired_owner: crate::frame_owner_model::FrameDocumentTaskOwner,
        current_owner: crate::frame_owner_model::FrameDocumentTaskOwner,
        realm_token: crate::native_bridge::RuntimeObservableContextToken,
    ) -> anyhow::Result<()> {
        initialize::rebind_child_window_realm_document_state(
            self,
            scope,
            global,
            model::ChildWindowRealmRebind {
                handle,
                browsing_context_id,
                expected_retired_owner,
                current_owner,
                realm_token,
            },
        )
    }
}
