use moli_core::browser::DocumentId;

pub const URL_BASE: &str = "chrome://newtab/";

/// DevTools binding to one current or reserved Browser Document.
///
/// `document.open()` restarts the renderer lifecycle inside the same Browser
/// Document; cross-document replacement changes `document_id`. Reservation
/// and installation use that same id, without a second attachment generation.
/// Deferred output may carry renderer Document/epoch metadata separately. A
/// target without a current or reserved Document has no such binding.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct TargetPageResidenceIdentity {
    browser_context_id: String,
    target_id: Option<String>,
    document_id: DocumentId,
}

impl TargetPageResidenceIdentity {
    pub(crate) fn new(
        browser_context_id: String,
        target_id: Option<String>,
        document_id: DocumentId,
    ) -> Self {
        Self {
            browser_context_id,
            target_id,
            document_id,
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(
        browser_context_id: String,
        target_id: Option<String>,
        document_id: u64,
    ) -> Self {
        Self::new(
            browser_context_id,
            target_id,
            DocumentId::from_raw_for_test(document_id),
        )
    }

    pub(crate) fn browser_context_id(&self) -> &str {
        &self.browser_context_id
    }

    pub(crate) fn target_id(&self) -> Option<&str> {
        self.target_id.as_deref()
    }

    pub fn document_id(&self) -> DocumentId {
        self.document_id
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
