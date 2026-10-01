//! The old document's part of preparing a replacement document.
//!
//! Capturing a replacement is inert. The renderer preparation boundary starts
//! the scope before queuing owner work, since the owner may be inside V8's pause
//! loop. Prepared-document ownership keeps it alive until commit or cancellation.

use std::sync::Arc;

use crate::devtools::pause::RendererInspectorPauseBridge;

/// Whether a failed commit still permits commands against the old Document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RendererPageReplacementFailureDisposition {
    DocumentPreserved,
    DocumentUnavailable,
}

/// The browser may restore the previous Inspector attachment only when the
/// renderer explicitly confirms that the old Document survived the failure.
#[derive(Debug)]
pub struct RendererPageReplacementError {
    disposition: RendererPageReplacementFailureDisposition,
    source: anyhow::Error,
}

impl std::fmt::Display for RendererPageReplacementError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.source, formatter)
    }
}

impl std::error::Error for RendererPageReplacementError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}

impl RendererPageReplacementError {
    pub fn document_preserved(source: anyhow::Error) -> Self {
        Self {
            disposition: RendererPageReplacementFailureDisposition::DocumentPreserved,
            source,
        }
    }

    pub fn document_unavailable(source: anyhow::Error) -> Self {
        Self {
            disposition: RendererPageReplacementFailureDisposition::DocumentUnavailable,
            source,
        }
    }

    pub fn disposition(&self) -> RendererPageReplacementFailureDisposition {
        self.disposition
    }

    pub(super) fn from_dispatch_error(error: anyhow::Error) -> Self {
        // A closed owner channel does not prove that commit never started.
        // Preserve the explicit disposition from the owner, otherwise fail
        // closed instead of restoring a potentially detached Document.
        error
            .downcast::<Self>()
            .unwrap_or_else(Self::document_unavailable)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RendererDocumentReplacement {
    pause: RendererInspectorPauseBridge,
    cancellation: moli_fetch::FetchCancelHandle,
    reload_preserves_navigation_referrer: Option<bool>,
    initial_document_environment: Option<super::RendererCapturedDocumentEnvironment>,
}

impl RendererDocumentReplacement {
    pub(super) fn new(
        pause: RendererInspectorPauseBridge,
        cancellation: moli_fetch::FetchCancelHandle,
    ) -> Self {
        Self {
            pause,
            cancellation,
            reload_preserves_navigation_referrer: None,
            initial_document_environment: None,
        }
    }

    pub(super) fn begin(self) -> anyhow::Result<Arc<RendererDocumentReplacementScope>> {
        anyhow::ensure!(
            !self.cancellation.is_cancelled() && self.pause.begin_document_replacement(),
            moli_fetch::FetchCancelled,
        );
        Ok(Arc::new(RendererDocumentReplacementScope {
            pause: self.pause,
            reload_preserves_navigation_referrer: self.reload_preserves_navigation_referrer,
            initial_document_environment: self.initial_document_environment,
        }))
    }
}

/// Owned by the prepare command and its returned handle, not by protocol or by
/// clones of the inert replacement input. Overlapping prepares for one old
/// document remain independent; completing either one must not release the other.
pub(crate) struct RendererDocumentReplacementScope {
    pause: RendererInspectorPauseBridge,
    pub(super) reload_preserves_navigation_referrer: Option<bool>,
    pub(super) initial_document_environment: Option<super::RendererCapturedDocumentEnvironment>,
}

impl Drop for RendererDocumentReplacementScope {
    fn drop(&mut self) {
        self.pause.finish_document_replacement();
    }
}

/// A response may initialize a new Page or replace the Document of an existing
/// Page. Capturing the latter performs no owner command and never enters V8.
#[derive(Clone)]
pub enum RendererDocumentPreparationTarget {
    NewPage(super::RendererPageReservationToken),
    ExistingPage(RendererPageReplacementTarget),
}

impl From<super::RendererPageReservationToken> for RendererDocumentPreparationTarget {
    fn from(token: super::RendererPageReservationToken) -> Self {
        Self::NewPage(token)
    }
}

impl From<RendererPageReplacementTarget> for RendererDocumentPreparationTarget {
    fn from(target: RendererPageReplacementTarget) -> Self {
        Self::ExistingPage(target)
    }
}

/// Non-owning identity captured by the browser before fetching a response.
/// The owning Page handle remains in its target slot throughout preparation.
#[derive(Clone)]
pub struct RendererPageReplacementTarget {
    pub(super) render_runtime: crate::render_runtime::RenderRuntimeHandle,
    pub(super) token: super::RendererPageToken,
    pub(super) replacement: RendererDocumentReplacement,
}

pub struct RendererPageReplacementReservationRequest {
    pub(super) token: super::RendererPageToken,
    pub(super) reservation_nonce: u64,
    pub(super) replacement_scope: Arc<RendererDocumentReplacementScope>,
}

struct PendingPageReplacementReservation {
    render_runtime: crate::render_runtime::RenderRuntimeHandle,
    token: super::RendererPageToken,
    reservation_nonce: u64,
    cancel_on_drop: bool,
}

impl Drop for PendingPageReplacementReservation {
    fn drop(&mut self) {
        if self.cancel_on_drop {
            let _ = self.render_runtime.enqueue(
                super::RendererOwnerCommand::CancelLivePageReplacementReservation {
                    token: self.token,
                    reservation_nonce: self.reservation_nonce,
                },
            );
        }
    }
}

impl RendererPageReplacementTarget {
    pub fn with_initial_document_environment(
        mut self,
        environment: Option<super::RendererCapturedDocumentEnvironment>,
    ) -> Self {
        self.replacement.initial_document_environment = environment;
        self
    }

    /// A reload retains an about:blank Document's origin and fallback base URL.
    /// Browser reload also reuses the previous navigation's referrer; script
    /// reload computes its referrer from the reloading Document instead.
    pub fn with_document_reload(mut self, browser_initiated: bool) -> Self {
        self.replacement.reload_preserves_navigation_referrer = Some(browser_initiated);
        self
    }

    pub fn page_id(&self) -> super::PageId {
        self.token.page_id
    }
    pub fn local_host_id(&self) -> super::RendererOwnerLocalHostId {
        self.token.local_host_id
    }

    pub(super) async fn reserve(
        self,
        scope: Arc<RendererDocumentReplacementScope>,
    ) -> anyhow::Result<super::RendererPageReservationToken> {
        static NEXT_REQUEST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let reservation_nonce = NEXT_REQUEST
            .fetch_update(
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
                |next| next.checked_add(1),
            )
            .expect("Page replacement reservation IDs exhausted");
        let mut pending = PendingPageReplacementReservation {
            render_runtime: self.render_runtime.clone(),
            token: self.token,
            reservation_nonce,
            cancel_on_drop: true,
        };
        let reply = self
            .render_runtime
            .dispatch(super::RendererOwnerCommand::ReserveLivePageReplacement(
                RendererPageReplacementReservationRequest {
                    token: self.token,
                    reservation_nonce,
                    replacement_scope: scope,
                },
            ))
            .await?;
        let super::RendererOwnerReply::LivePageReplacementReserved(reservation) = reply else {
            anyhow::bail!("renderer returned a non-reservation reply for Page replacement");
        };
        pending.cancel_on_drop = false;
        Ok(reservation)
    }
}

impl RendererDocumentPreparationTarget {
    pub fn page_id(&self) -> super::PageId {
        match self {
            Self::NewPage(token) => token.page_id(),
            Self::ExistingPage(target) => target.page_id(),
        }
    }
    pub fn local_host_id(&self) -> super::RendererOwnerLocalHostId {
        match self {
            Self::NewPage(token) => token.local_host_id(),
            Self::ExistingPage(target) => target.local_host_id(),
        }
    }

    pub(super) async fn begin_and_reserve(
        self,
        owner: &super::RendererOwnerHandle,
    ) -> anyhow::Result<(
        super::RendererPageReservationToken,
        Option<Arc<RendererDocumentReplacementScope>>,
    )> {
        match self {
            Self::NewPage(token) => Ok((token, None)),
            Self::ExistingPage(target) => {
                anyhow::ensure!(
                    target.local_host_id() == owner.state.owner_local_host_id,
                    "replacement target belongs to another renderer owner"
                );
                // The accepted response is now at the provisional-load boundary.
                // Wake a paused old Document before any owner command is queued.
                let scope = target.replacement.clone().begin()?;
                let token = target.reserve(scope.clone()).await?;
                Ok((token, Some(scope)))
            }
        }
    }
}

#[cfg(test)]
mod tests;
