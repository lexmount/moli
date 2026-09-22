use crate::native_bridge::NavigationHistoryEntrySeed;
use crate::{
    document_runtime::{DocumentPolicyContainer, DomHandle},
    frame_owner_model::FrameDocumentTaskOwner,
    native_bridge::{RuntimeObservableContextToken, WindowExecutionContextAccessPolicy},
};
use url::Url;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::native_bridge::context_host::child_frame_runtime) enum WindowWorldKind {
    Default,
    Isolated {
        access_policy: WindowExecutionContextAccessPolicy,
    },
}

impl WindowWorldKind {
    pub(in crate::native_bridge::context_host::child_frame_runtime) fn access_policy(
        self,
    ) -> WindowExecutionContextAccessPolicy {
        match self {
            Self::Default => WindowExecutionContextAccessPolicy::EnforceWebOrigin,
            Self::Isolated { access_policy } => access_policy,
        }
    }

    pub(in crate::native_bridge::context_host::child_frame_runtime) fn is_default(self) -> bool {
        matches!(self, Self::Default)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::native_bridge::context_host::child_frame_runtime) struct ChildWindowRealmInit {
    pub(in crate::native_bridge::context_host::child_frame_runtime) handle: DomHandle,
    pub(in crate::native_bridge::context_host::child_frame_runtime) expected_owner:
        FrameDocumentTaskOwner,
    pub(in crate::native_bridge::context_host::child_frame_runtime) realm_token:
        RuntimeObservableContextToken,
    pub(in crate::native_bridge::context_host::child_frame_runtime) world: WindowWorldKind,
}

#[derive(Debug)]
pub(in crate::native_bridge::context_host::child_frame_runtime) struct ChildWindowRealmSnapshot {
    pub(super) handle: DomHandle,
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
    pub(in crate::native_bridge::context_host::child_frame_runtime) expected_retired_owner:
        FrameDocumentTaskOwner,
    pub(in crate::native_bridge::context_host::child_frame_runtime) current_owner:
        FrameDocumentTaskOwner,
    pub(in crate::native_bridge::context_host::child_frame_runtime) realm_token:
        RuntimeObservableContextToken,
}
