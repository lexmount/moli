use super::{FrameOwnerStore, PageScriptHost};
use crate::{
    document_script_scheduler::FrameDocumentReadyActionRoute,
    frame_owner_model::FrameDocumentTaskRealmCurrentness,
};

impl FrameOwnerStore {
    pub(super) fn frame_document_ready_route_task_is_current(
        &self,
        route: &FrameDocumentReadyActionRoute,
    ) -> bool {
        if route.requires_realm() && route.optional_realm_id().is_none() {
            return false;
        }

        if let Some(child_handle) = route.child_handle() {
            let owner = route.task_owner();
            if !self.child_document_task_owner_is_current(child_handle, owner) {
                return false;
            }
            if !route.requires_realm() {
                return true;
            }
            return route.optional_realm_id().is_some_and(|realm_id| {
                matches!(
                    self.child_document_task_owner_realm_currentness(child_handle, owner, realm_id),
                    FrameDocumentTaskRealmCurrentness::Current { .. }
                )
            });
        }

        let Some(realm_id) = route.optional_realm_id() else {
            return false;
        };
        matches!(
            self.frame_document_owner_realm_currentness(route.document_owner(), realm_id),
            FrameDocumentTaskRealmCurrentness::Current { owner, .. }
                if owner == route.task_owner()
        )
    }
}

impl PageScriptHost {
    pub(crate) fn frame_document_ready_route_task_is_current(
        &self,
        route: &FrameDocumentReadyActionRoute,
    ) -> bool {
        self.frame_owner_store
            .frame_document_ready_route_task_is_current(route)
    }
}
