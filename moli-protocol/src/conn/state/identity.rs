use moli_core::{PageId, RendererOutputResidenceIdentity, RendererOwnerLocalHostId, page::Page};

use super::TargetPageAttachmentId;

pub const URL_BASE: &str = "chrome://newtab/";

/// Exact renderer Page residence captured with one deferred-load owner action.
///
/// A protocol session can survive a Page replacement, so session identity is
/// not precise enough to decide whether a later renderer publication is a
/// prerequisite of this action. The renderer owner and Page ids are allocated
/// monotonically by the renderer runtime and let the scheduler reject output
/// from a replacement Page without consulting mutable protocol routing state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RendererPageResidenceIdentity {
    owner_local_host_id: RendererOwnerLocalHostId,
    page_id: PageId,
}

impl RendererPageResidenceIdentity {
    pub(crate) const fn from_parts(
        owner_local_host_id: RendererOwnerLocalHostId,
        page_id: PageId,
    ) -> Self {
        Self {
            owner_local_host_id,
            page_id,
        }
    }

    pub(crate) fn from_page(page: &Page) -> Self {
        Self::from_parts(page.renderer_owner_local_host_id(), page.renderer_page_id())
    }

    pub(crate) const fn owner_local_host_id(self) -> RendererOwnerLocalHostId {
        self.owner_local_host_id
    }

    pub(crate) const fn page_id(self) -> PageId {
        self.page_id
    }

    pub(crate) const fn from_residence(residence: RendererOutputResidenceIdentity) -> Option<Self> {
        match residence {
            RendererOutputResidenceIdentity::Page {
                owner_local_host_id,
                page_id,
            } => Some(Self::from_parts(owner_local_host_id, page_id)),
            RendererOutputResidenceIdentity::SharedWorker { .. }
            | RendererOutputResidenceIdentity::ServiceWorker { .. } => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn new(owner_local_host_id: RendererOwnerLocalHostId, page_id: PageId) -> Self {
        Self::from_parts(owner_local_host_id, page_id)
    }

    pub(crate) fn matches_residence(self, residence: RendererOutputResidenceIdentity) -> bool {
        matches!(
            residence,
            RendererOutputResidenceIdentity::Page {
                owner_local_host_id,
                page_id,
            } if owner_local_host_id == self.owner_local_host_id && page_id == self.page_id
        )
    }
}

/// Identifies one current or reserved Page attachment within a protocol target.
///
/// This identity deliberately does not include the renderer Document. A
/// `document.open()` replacement keeps the same Page residence, while taking,
/// replacing, or retiring the Page changes `page_attachment_id`. Deferred
/// protocol work that belongs to a Page should therefore use this identity for
/// authorization and may carry a renderer Document identity separately as
/// causal metadata. The attachment id is allocated when the Page is reserved,
/// so work emitted before installation and work emitted after commit retain the
/// same identity without predicting a numeric generation. A target without a
/// current or reserved Page has no Page residence identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct TargetPageResidenceIdentity {
    browser_context_id: String,
    target_id: Option<String>,
    page_attachment_id: TargetPageAttachmentId,
}

impl TargetPageResidenceIdentity {
    pub(crate) fn new(
        browser_context_id: String,
        target_id: Option<String>,
        page_attachment_id: TargetPageAttachmentId,
    ) -> Self {
        Self {
            browser_context_id,
            target_id,
            page_attachment_id,
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(
        browser_context_id: String,
        target_id: Option<String>,
        page_attachment_id: u64,
    ) -> Self {
        Self::new(
            browser_context_id,
            target_id,
            TargetPageAttachmentId::from_raw_for_test(page_attachment_id),
        )
    }

    pub(crate) fn browser_context_id(&self) -> &str {
        &self.browser_context_id
    }

    pub(crate) fn target_id(&self) -> Option<&str> {
        self.target_id.as_deref()
    }

    pub(crate) fn page_attachment_id(&self) -> TargetPageAttachmentId {
        self.page_attachment_id
    }
}

/// Identifies one protocol attachment to one target Page residence.
///
/// A Page residence can be observed by more than one CDP session over its
/// lifetime. Deferred output must therefore retain both the Page attachment
/// and the exact session that captured it. Explicit session ids are allocated
/// monotonically by one `CdpConnection` and are never reused. `None` denotes
/// the connection's implicit Page attachment; the embedded Page identity keeps
/// that route from following a later active target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TargetPageProtocolAttachmentIdentity {
    page_owner: TargetPageResidenceIdentity,
    session_id: Option<String>,
}

impl TargetPageProtocolAttachmentIdentity {
    pub(crate) fn new(page_owner: TargetPageResidenceIdentity, session_id: Option<String>) -> Self {
        Self {
            page_owner,
            session_id,
        }
    }

    pub(crate) fn page_owner(&self) -> &TargetPageResidenceIdentity {
        &self.page_owner
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }
}

/// Identifies one root renderer Document as observed through one exact Page
/// protocol attachment.
///
/// Page identity alone is insufficient for deferred child-frame activity:
/// `document.open()` preserves the installed Page while replacing the root
/// Document and its entire child frame tree. Session identity alone is also
/// insufficient because a detached attachment must not deliver held output to
/// another attachment of the same target. Keeping the two authorities in one
/// value makes a prepared child-frame batch impossible to apply through a
/// drain-time "current session" lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TargetRootDocumentProtocolAttachmentIdentity {
    attachment: TargetPageProtocolAttachmentIdentity,
    root_document: moli_core::RendererDocumentLifecycleIdentity,
}

impl TargetRootDocumentProtocolAttachmentIdentity {
    pub(crate) fn new(
        attachment: TargetPageProtocolAttachmentIdentity,
        root_document: moli_core::RendererDocumentLifecycleIdentity,
    ) -> Self {
        Self {
            attachment,
            root_document,
        }
    }

    pub(crate) fn attachment(&self) -> &TargetPageProtocolAttachmentIdentity {
        &self.attachment
    }

    pub(crate) fn root_document(&self) -> moli_core::RendererDocumentLifecycleIdentity {
        self.root_document
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        self.attachment.session_id()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TargetIdentityState {
    url: String,
    security_origin: String,
    secure_context_type: String,
}

impl TargetIdentityState {
    pub(crate) fn new(url: String, security_origin: String, secure_context_type: String) -> Self {
        Self {
            url,
            security_origin,
            secure_context_type,
        }
    }

    pub(crate) fn new_tab() -> Self {
        Self::new(URL_BASE.into(), URL_BASE.into(), "Secure".into())
    }

    pub(crate) fn about_blank() -> Self {
        Self::new("about:blank".into(), URL_BASE.into(), "Secure".into())
    }

    pub(crate) fn with_url(url: String) -> Self {
        let parsed_url = url::Url::parse(&url).ok();
        let inherits_initial_origin = parsed_url.as_ref().is_some_and(moli_url::is_about_blank);
        let security_origin = if inherits_initial_origin {
            URL_BASE.to_owned()
        } else {
            parsed_url
                .as_ref()
                .map(moli_url::origin_ascii_serialization)
                .unwrap_or_else(|| URL_BASE.to_owned())
        };
        let secure_context_type = if inherits_initial_origin
            || parsed_url
                .as_ref()
                .is_some_and(moli_url::is_potentially_trustworthy_url)
        {
            "Secure"
        } else {
            "InsecureScheme"
        };
        Self::new(url, security_origin, secure_context_type.into())
    }

    pub(crate) fn url(&self) -> &str {
        &self.url
    }

    pub(crate) fn security_origin(&self) -> &str {
        &self.security_origin
    }

    pub(crate) fn secure_context_type(&self) -> &str {
        &self.secure_context_type
    }

    pub(crate) fn set_url(&mut self, url: String) {
        self.url = url;
    }

    pub(crate) fn set_security_origin(&mut self, security_origin: String) {
        self.security_origin = security_origin;
    }

    pub(crate) fn set_secure_context_type(&mut self, secure_context_type: String) {
        self.secure_context_type = secure_context_type;
    }
}

impl Default for TargetIdentityState {
    fn default() -> Self {
        Self::new_tab()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_identity_with_url_derives_origin_for_real_urls() {
        let identity = TargetIdentityState::with_url("http://example.test/path".to_owned());
        assert_eq!(identity.url(), "http://example.test/path");
        assert_eq!(identity.security_origin(), "http://example.test");
        assert_eq!(identity.secure_context_type(), "InsecureScheme");
    }

    #[test]
    fn target_identity_with_url_keeps_initial_origin_for_about_blank() {
        let identity = TargetIdentityState::with_url("about:blank".to_owned());
        assert_eq!(identity.url(), "about:blank");
        assert_eq!(identity.security_origin(), URL_BASE);
        assert_eq!(identity.secure_context_type(), "Secure");
    }
}
