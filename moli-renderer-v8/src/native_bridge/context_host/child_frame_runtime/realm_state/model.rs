use crate::{
    browsing_context_model::{BrowsingContextId, RealmHostProjection, RealmWorldKind},
    document_runtime::{DocumentPolicyContainer, DomHandle},
    frame_owner_model::FrameDocumentTaskOwner,
    native_bridge::{RuntimeObservableContextToken, WindowExecutionContextAccessPolicy},
};
use moli_page_types::NavigationHistoryEntrySeed;
use url::Url;

pub(in crate::native_bridge::context_host::child_frame_runtime) type WindowWorldKind =
    RealmWorldKind<WindowExecutionContextAccessPolicy>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::native_bridge::context_host::child_frame_runtime) struct ChildWindowRealmInit {
    pub(in crate::native_bridge::context_host::child_frame_runtime) handle: DomHandle,
    pub(in crate::native_bridge::context_host::child_frame_runtime) projection: RealmHostProjection<
        BrowsingContextId,
        FrameDocumentTaskOwner,
        RuntimeObservableContextToken,
        WindowWorldKind,
    >,
}

#[derive(Debug)]
pub(in crate::native_bridge::context_host::child_frame_runtime) struct ChildWindowRealmSnapshot {
    pub(super) handle: DomHandle,
    pub(super) browsing_context_id: BrowsingContextId,
    pub(super) owner: FrameDocumentTaskOwner,
    pub(super) current_url: Url,
    pub(super) origin: String,
    pub(super) window_name: String,
    pub(super) navigation_seed: NavigationHistoryEntrySeed,
    pub(super) navigation_type: String,
    pub(super) performance_time_origin: f64,
    pub(super) policy: DocumentPolicyContainer,
}

pub(in crate::native_bridge::context_host::child_frame_runtime) struct ChildWindowRealmProjection<
    's,
> {
    pub(in crate::native_bridge::context_host::child_frame_runtime) parent:
        v8::Local<'s, v8::Object>,
    pub(in crate::native_bridge::context_host::child_frame_runtime) top: v8::Local<'s, v8::Object>,
    pub(in crate::native_bridge::context_host::child_frame_runtime) document:
        v8::Local<'s, v8::Object>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::native_bridge::context_host::child_frame_runtime) struct ChildWindowRealmRebind {
    pub(in crate::native_bridge::context_host::child_frame_runtime) handle: DomHandle,
    pub(in crate::native_bridge::context_host::child_frame_runtime) browsing_context_id:
        BrowsingContextId,
    pub(in crate::native_bridge::context_host::child_frame_runtime) expected_retired_owner:
        FrameDocumentTaskOwner,
    pub(in crate::native_bridge::context_host::child_frame_runtime) current_owner:
        FrameDocumentTaskOwner,
    pub(in crate::native_bridge::context_host::child_frame_runtime) realm_token:
        RuntimeObservableContextToken,
}
