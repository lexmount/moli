use super::{
    ImageDecodeRequestId, ImageDecodeRetirementOutcome, JsContextHost, OwnerDispatchScope,
    PendingImageDecodeRequest, PendingImageDecodeRequestState, RuntimeObservableContextToken,
};
use crate::{
    document_runtime::DomHandle,
    frame_owner_model::FrameDocumentTaskOwner,
    native_bridge::element::{image_intrinsic_dimensions, image_selected_request_key},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ImageDecodeCompletion {
    Resolve,
    Reject,
}

impl JsContextHost {
    pub(crate) fn register_image_decode_request(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        element: DomHandle,
        resolver: v8::Local<'_, v8::PromiseResolver>,
    ) -> Option<ImageDecodeRequestId> {
        let owner_document_handle = self.dom_host().owner_document_handle(element)?;
        let element_owner = self.current_image_decode_element_owner(owner_document_handle)?;
        let request_key = image_selected_request_key(self, element)?;
        let dispatch_scope = active_window_dispatch_scope(scope);
        let relevant_context =
            self.current_window_execution_context_binding(scope, dispatch_scope)?;
        let id = ImageDecodeRequestId::new(self.next_image_decode_id);
        self.next_image_decode_id = self
            .next_image_decode_id
            .checked_add(1)
            .expect("image decode request id overflow");
        let previous = self.pending_image_decode_requests.insert(
            id,
            PendingImageDecodeRequest {
                id,
                element,
                owner_document_handle,
                element_owner,
                relevant_context,
                resolver: v8::Global::new(scope, resolver),
                request_key,
                state: PendingImageDecodeRequestState::PendingMicrotask,
            },
        );
        debug_assert!(previous.is_none());
        tracing::debug!(
            request = id.get(),
            image = element.index(),
            ?element_owner,
            relevant_context_owner = ?self
                .pending_image_decode_requests
                .get(&id)
                .map(|request| request.relevant_context.owner()),
            relevant_context_token = ?self
                .pending_image_decode_requests
                .get(&id)
                .map(|request| request.relevant_context.realm_token()),
            "registered image decode request"
        );
        Some(id)
    }

    pub(crate) fn process_image_decode_request(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        id: ImageDecodeRequestId,
    ) -> bool {
        let Some(mut request) = self.pending_image_decode_requests.remove(&id) else {
            return false;
        };
        let previous_state = request.state;
        if !self.image_decode_element_owner_is_current(&request) {
            tracing::debug!(
                request = id.get(),
                image = request.element.index(),
                element_owner = ?request.element_owner,
                ?previous_state,
                "rejected image decode request for retired element document"
            );
            let _ = self.settle_image_decode_request(scope, request, ImageDecodeCompletion::Reject);
            return true;
        }

        let current_request_key = image_selected_request_key(self, request.element);
        if current_request_key.as_ref() != Some(&request.request_key) {
            tracing::debug!(
                request = id.get(),
                image = request.element.index(),
                request_changed = current_request_key.as_ref() != Some(&request.request_key),
                ?previous_state,
                "rejected image decode request after current request changed"
            );
            let _ = self.settle_image_decode_request(scope, request, ImageDecodeCompletion::Reject);
            return true;
        }
        if image_source_can_decode(self, request.element, request.request_key.url()) {
            let _ =
                self.settle_image_decode_request(scope, request, ImageDecodeCompletion::Resolve);
            return true;
        }
        let waits_for_current_load =
            self.pending_image_load_events
                .iter()
                .any(|(pending_element, pending)| {
                    self.pending_image_load_event_is_current(*pending_element, pending)
                        && pending.request_key() == Some(&request.request_key)
                });
        if waits_for_current_load {
            request.state = PendingImageDecodeRequestState::PendingLoad;
            self.pending_image_decode_requests.insert(id, request);
            tracing::debug!(
                request = id.get(),
                ?previous_state,
                "image decode request is waiting for the current image load"
            );
            return true;
        }
        let _ = self.settle_image_decode_request(scope, request, ImageDecodeCompletion::Reject);
        true
    }

    pub(crate) fn process_image_decode_requests_for_element(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        element: DomHandle,
    ) -> usize {
        let ids = self
            .pending_image_decode_requests
            .iter()
            .filter_map(|(id, request)| (request.element == element).then_some(*id))
            .collect::<Vec<_>>();
        for id in &ids {
            let _ = self.process_image_decode_request(scope, *id);
        }
        ids.len()
    }

    pub(crate) fn process_pending_image_decode_requests(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
    ) -> usize {
        let ids = self
            .pending_image_decode_requests
            .keys()
            .copied()
            .collect::<Vec<_>>();
        for id in &ids {
            let _ = self.process_image_decode_request(scope, *id);
        }
        ids.len()
    }

    pub(crate) fn reject_image_decode_request(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        id: ImageDecodeRequestId,
    ) -> bool {
        let Some(request) = self.pending_image_decode_requests.remove(&id) else {
            return false;
        };
        let _ = self.settle_image_decode_request(scope, request, ImageDecodeCompletion::Reject);
        true
    }

    pub(crate) fn retire_image_decode_requests_for_document_owner(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        owner: FrameDocumentTaskOwner,
    ) -> ImageDecodeRetirementOutcome {
        let ids = self
            .pending_image_decode_requests
            .iter()
            .filter_map(|(id, request)| (request.element_owner == owner).then_some(*id))
            .collect::<Vec<_>>();
        let mut outcome = ImageDecodeRetirementOutcome::default();
        for id in ids {
            let request = self
                .pending_image_decode_requests
                .remove(&id)
                .expect("collected image decode request must remain registered");
            if self.settle_image_decode_request(scope, request, ImageDecodeCompletion::Reject) {
                outcome.rejected_count += 1;
            } else {
                outcome.dropped_context_count += 1;
            }
        }
        outcome
    }

    pub(crate) fn retire_image_decode_requests_for_context_token(
        &mut self,
        context_token: RuntimeObservableContextToken,
    ) -> usize {
        let ids = self
            .pending_image_decode_requests
            .iter()
            .filter_map(|(id, request)| {
                (request.relevant_context.realm_token() == context_token).then_some(*id)
            })
            .collect::<Vec<_>>();
        let retired_count = ids.len();
        for id in ids {
            self.pending_image_decode_requests.remove(&id);
        }
        retired_count
    }

    pub(crate) fn retire_image_decode_requests_for_execution_context_owner(
        &mut self,
        owner: super::WindowExecutionContextOwner,
    ) -> usize {
        let ids = self
            .pending_image_decode_requests
            .iter()
            .filter_map(|(id, request)| (request.relevant_context.owner() == owner).then_some(*id))
            .collect::<Vec<_>>();
        let retired_count = ids.len();
        for id in ids {
            self.pending_image_decode_requests.remove(&id);
        }
        retired_count
    }

    #[cfg(test)]
    pub(crate) fn pending_image_decode_request_owners_for_test(
        &self,
    ) -> Vec<(FrameDocumentTaskOwner, super::WindowExecutionContextOwner)> {
        self.pending_image_decode_requests
            .values()
            .map(|request| (request.element_owner, request.relevant_context.owner()))
            .collect()
    }

    fn current_image_decode_element_owner(
        &self,
        owner_document_handle: DomHandle,
    ) -> Option<FrameDocumentTaskOwner> {
        if owner_document_handle == self.document_handle() {
            return self.current_main_document_task_owner();
        }
        let child_handle =
            self.child_browsing_context_host_for_document_handle(owner_document_handle)?;
        let snapshot = self.frame_owner_current_child_snapshot(child_handle)?;
        if snapshot.document_handle != owner_document_handle {
            return None;
        }
        Some(FrameDocumentTaskOwner::new(
            snapshot.scheduler_lane_id,
            snapshot.local_window_id,
            snapshot.document_id,
        ))
    }

    fn image_decode_element_owner_is_current(&self, request: &PendingImageDecodeRequest) -> bool {
        self.frame_owner_store
            .document_task_owner_is_current(request.element_owner)
            && self.dom_host().owner_document_handle(request.element)
                == Some(request.owner_document_handle)
    }

    fn settle_image_decode_request(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        request: PendingImageDecodeRequest,
        completion: ImageDecodeCompletion,
    ) -> bool {
        let relevant_owner = request.relevant_context.owner();
        let dispatch_scope = request.relevant_context.dispatch_scope();
        if !self.window_execution_context_owner_is_current(relevant_owner, dispatch_scope) {
            tracing::debug!(
                request = request.id.get(),
                image = request.element.index(),
                ?relevant_owner,
                ?dispatch_scope,
                "dropped image decode settlement for retired relevant context"
            );
            return false;
        }
        let realm_token = request.relevant_context.realm_token();
        let context = request.relevant_context.context(scope);
        let scope = &mut v8::ContextScope::new(scope, context);
        if super::current_runtime_observable_context_token(scope) != Some(realm_token) {
            tracing::debug!(
                request = request.id.get(),
                image = request.element.index(),
                ?realm_token,
                "dropped image decode settlement for mismatched relevant realm"
            );
            return false;
        }
        let previous_dispatch_scope = dispatch_scope.enter(scope);
        let resolver = v8::Local::new(scope, &request.resolver);
        let settled = match completion {
            ImageDecodeCompletion::Resolve => {
                resolver.resolve(scope, v8::undefined(scope).into()) == Some(true)
            }
            ImageDecodeCompletion::Reject => {
                let exception = crate::context_bootstrap::new_dom_exception_value(
                    scope,
                    "The source image cannot be decoded.",
                    "EncodingError",
                );
                resolver.reject(scope, exception) == Some(true)
            }
        };
        if settled {
            dispatch_scope.defer_restore(scope, previous_dispatch_scope);
        } else {
            dispatch_scope.restore(scope, previous_dispatch_scope);
        }
        tracing::debug!(
            request = request.id.get(),
            image = request.element.index(),
            ?completion,
            settled,
            "settled image decode request in relevant realm"
        );
        settled
    }
}

fn active_window_dispatch_scope(scope: &mut v8::PinScope<'_, '_>) -> OwnerDispatchScope {
    if let Some(child_handle) = crate::native_bridge::active_child_window_handle(scope) {
        return OwnerDispatchScope::Child(child_handle);
    }
    if let Some(popup_id) = crate::native_bridge::active_lightweight_popup_id(scope) {
        return OwnerDispatchScope::LightweightPopup(popup_id);
    }
    OwnerDispatchScope::Top
}

fn image_source_can_decode(runtime: &JsContextHost, handle: DomHandle, source: &str) -> bool {
    if source.is_empty() {
        return false;
    }
    if source.trim_start().starts_with("data:")
        && !moli_web_mime::data_url_mime_type(source.trim_start())
            .is_some_and(|mime| moli_web_mime::is_image_mime(&mime))
    {
        return false;
    }
    (runtime.image_resource_is_ready(handle)
        && image_intrinsic_dimensions(runtime, handle).is_some())
        || image_selected_request_key(runtime, handle)
            .is_some_and(|request_key| runtime.has_ready_image_request(&request_key))
}
