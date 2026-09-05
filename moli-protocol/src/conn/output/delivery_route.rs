use crate::conn::{CdpConnection, TargetRootDocumentProtocolAttachmentIdentity};

/// Exact Page-domain subscription that authorized one projected event.
///
/// Page.disable followed by Page.enable creates a new generation. Holding the
/// generation here prevents a delayed event from an older subscription from
/// being delivered to the replacement subscription even when both use the
/// same wire session id.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PageDomainSubscriptionRoute {
    session_id: Option<String>,
    generation: u64,
}

/// Exact Browser-domain download subscription that authorized one event.
///
/// Browser.setDownloadBehavior can replace the observer for the same wire
/// session. The generation prevents already queued download events from being
/// inherited by that replacement observer.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BrowserDownloadSubscriptionRoute {
    session_id: Option<String>,
    generation: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ProtocolDeliveryCapabilities {
    page_subscription: Option<PageDomainSubscriptionRoute>,
    browser_download_subscription: Option<BrowserDownloadSubscriptionRoute>,
    root_document: Option<TargetRootDocumentProtocolAttachmentIdentity>,
}

/// Final delivery authority carried beside one concrete protocol payload.
///
/// `wire_session` is initialized from an explicitly routed payload, or filled
/// exactly once when a target-owned fact is fanned out to a concrete session.
/// Optional capabilities narrow that route further; they never rediscover a
/// destination from current Page state. A root-Document binding already
/// contains the exact Page attachment, so it covers both attachment and
/// Document replacement in one authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProtocolDeliveryRoute {
    wire_session: Option<String>,
    // Most protocol events need neither capability. Keep the uncommon,
    // identity-rich guards behind one indirection so adding another guard does
    // not inflate every Console/Runtime/Network queue item.
    capabilities: Option<Box<ProtocolDeliveryCapabilities>>,
}

impl ProtocolDeliveryRoute {
    pub(super) fn for_wire_session(wire_session: Option<&str>) -> Self {
        Self {
            wire_session: wire_session.map(str::to_owned),
            capabilities: None,
        }
    }

    fn capabilities_mut(&mut self) -> &mut ProtocolDeliveryCapabilities {
        self.capabilities
            .get_or_insert_with(|| Box::new(ProtocolDeliveryCapabilities::default()))
    }

    pub(super) fn wire_session_id(&self) -> Option<&str> {
        self.wire_session.as_deref()
    }

    pub(super) fn navigation_gate_target_id(&self) -> Option<&str> {
        self.capabilities
            .as_deref()?
            .root_document
            .as_ref()?
            .attachment()
            .page_owner()
            .target_id()
    }

    pub(super) fn ensure_wire_session_id(&mut self, session_id: &str) {
        match self.wire_session.as_deref() {
            Some(existing) => assert_eq!(
                existing, session_id,
                "a protocol delivery route cannot change wire sessions"
            ),
            None => self.wire_session = Some(session_id.to_owned()),
        }
    }

    pub(super) fn bind_page_subscription(&mut self, session_id: Option<&str>, generation: u64) {
        assert!(
            self.capabilities
                .as_deref()
                .is_none_or(|capabilities| capabilities.page_subscription.is_none()),
            "a protocol event cannot change Page subscription authority"
        );
        assert_eq!(
            self.wire_session_id(),
            session_id,
            "Page subscription and wire-session routes must agree"
        );
        self.capabilities_mut().page_subscription = Some(PageDomainSubscriptionRoute {
            session_id: session_id.map(str::to_owned),
            generation,
        });
    }

    pub(super) fn bind_browser_download_subscription(
        &mut self,
        session_id: Option<&str>,
        generation: u64,
    ) {
        assert!(
            self.capabilities
                .as_deref()
                .is_none_or(|capabilities| capabilities.browser_download_subscription.is_none()),
            "a protocol event cannot change Browser download subscription authority"
        );
        assert_eq!(
            self.wire_session_id(),
            session_id,
            "Browser download subscription and wire-session routes must agree"
        );
        self.capabilities_mut().browser_download_subscription =
            Some(BrowserDownloadSubscriptionRoute {
                session_id: session_id.map(str::to_owned),
                generation,
            });
    }

    pub(super) fn bind_root_document(
        &mut self,
        binding: TargetRootDocumentProtocolAttachmentIdentity,
    ) {
        assert!(
            self.capabilities
                .as_deref()
                .is_none_or(|capabilities| capabilities.root_document.is_none()),
            "a protocol event cannot change root-Document authority"
        );
        assert_eq!(
            self.wire_session_id(),
            binding.session_id(),
            "root-Document attachment and wire-session routes must agree"
        );
        self.capabilities_mut().root_document = Some(binding);
    }

    pub(super) fn is_current(&self, conn: &CdpConnection) -> bool {
        let Some(capabilities) = self.capabilities.as_deref() else {
            return true;
        };
        if let Some(subscription) = capabilities.page_subscription.as_ref()
            && !conn.page_domain_subscription_is_current(
                subscription.session_id.as_deref(),
                subscription.generation,
            )
        {
            return false;
        }
        if let Some(subscription) = capabilities.browser_download_subscription.as_ref()
            && !conn
                .download_subscriptions
                .browser_event_subscription_is_current(
                    subscription.session_id.as_deref(),
                    subscription.generation,
                )
        {
            return false;
        }
        if let Some(root_document) = capabilities.root_document.as_ref()
            && !conn.target_root_document_protocol_attachment_identity_is_current(root_document)
        {
            return false;
        }
        true
    }
}
