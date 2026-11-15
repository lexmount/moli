use super::super::PageScriptHost;
use super::discovery::ChildFrameOwnerElementKind;
use crate::document_runtime::{DocumentSubresourceCspKind, DomHandle};
use url::Url;

#[derive(Debug, PartialEq, Eq)]
struct EmbeddedContentRequest {
    document: DomHandle,
    url: Url,
    type_hint: Option<String>,
}

/// One resource-selection attempt, including image resources which do not
/// create a child navigable. Wrapper discovery must not repeat its CSP reports
/// or apply a later meta policy to an already admitted request.
pub(in crate::native_bridge::context_host) struct EmbeddedContentCspAdmission {
    request: EmbeddedContentRequest,
    allowed: bool,
}

impl PageScriptHost {
    fn embedded_content_csp_request(&self, handle: DomHandle) -> Option<EmbeddedContentRequest> {
        let kind = self.child_frame_owner_element_kind(handle)?;
        if !matches!(
            kind,
            ChildFrameOwnerElementKind::Object | ChildFrameOwnerElementKind::Embed
        ) || !self.child_browsing_context_host_is_active(handle)
            || self.embedded_content_is_inside_media_element(handle)
        {
            return None;
        }
        let source = self
            .dom_host()
            .get_attribute(handle, kind.navigation_attribute())?;
        if source.trim().is_empty() {
            return None;
        }
        Some(EmbeddedContentRequest {
            document: self.dom_host().owner_document_handle(handle)?,
            url: self.parse_child_frame_owner_attribute_url(handle, source.trim())?,
            type_hint: self.dom_host().get_attribute(handle, "type"),
        })
    }

    pub(super) fn refresh_embedded_content_csp_admission(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
    ) {
        let Some(request) = self.embedded_content_csp_request(handle) else {
            self.embedded_content_csp_admissions.remove(&handle);
            return;
        };
        if let Some(admission) = self.embedded_content_csp_admissions.get(&handle)
            && admission.request == request
        {
            return;
        }
        let allowed = self.bypass_content_security_policy()
            || !self
                .check_element_subresource_csp(
                    scope,
                    handle,
                    &request.url,
                    DocumentSubresourceCspKind::Object,
                )
                .blocks_request();
        self.embedded_content_csp_admissions
            .insert(handle, EmbeddedContentCspAdmission { request, allowed });
    }

    pub(super) fn embedded_content_request_is_csp_blocked(&self, handle: DomHandle) -> bool {
        self.embedded_content_csp_admissions
            .get(&handle)
            .is_some_and(|admission| {
                !admission.allowed
                    && self.embedded_content_csp_request(handle).as_ref()
                        == Some(&admission.request)
            })
    }

    pub(in crate::native_bridge::context_host) fn embedded_content_request_was_csp_checked(
        &self,
        handle: DomHandle,
        url: &Url,
    ) -> bool {
        self.embedded_content_csp_admissions
            .get(&handle)
            .is_some_and(|admission| {
                admission.request.url == *url
                    && self.embedded_content_csp_request(handle).as_ref()
                        == Some(&admission.request)
            })
    }
}
