//! Protocol-neutral identities and lifecycle state for browser-owned objects.
//!
//! These identities name physical browser object incarnations. They are not
//! CDP wire identifiers and their numeric representation does not define an
//! ordering relationship.

mod document_lifecycle;
mod document_lifetime;
mod downloads;
mod permissions;
mod renderer_residence;
mod service_workers;

pub use document_lifecycle::DocumentLifecycle;
pub use document_lifetime::{DocumentLifetime, DocumentLifetimeObserver, DocumentRetirement};
pub use downloads::{
    DownloadAccessError, DownloadBehavior, DownloadBody, DownloadManager, DownloadMetadata,
    DownloadObservation, DownloadPolicy, DownloadSnapshot, DownloadState,
};
pub use permissions::{PermissionDefaults, PermissionOverrides};
pub use renderer_residence::RendererPageResidenceIdentity;
pub use service_workers::ServiceWorkerCommand;

/// Navigation semantics, independent of the protocol that requested the load.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum NavigationRequestLoadPolicy {
    #[default]
    DocumentInitiated,
    BrowserInitiated,
    Reload,
}

use std::{
    num::NonZeroU64,
    sync::atomic::{AtomicU64, Ordering},
};

macro_rules! define_browser_identity {
    ($name:ident, $counter:ident, $label:literal, $documentation:literal) => {
        static $counter: AtomicU64 = AtomicU64::new(1);

        #[doc = $documentation]
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(NonZeroU64);

        impl $name {
            /// Allocates a process-unique incarnation of this browser object.
            pub fn allocate() -> Self {
                Self(allocate_nonzero_u64(&$counter, $label))
            }

            /// Returns the opaque value for diagnostics and migration bridges.
            pub const fn get(self) -> u64 {
                self.0.get()
            }
        }
    };
}

define_browser_identity!(
    BrowserContextId,
    NEXT_BROWSER_CONTEXT_ID,
    "browser context id",
    "Identity of one physical BrowserContext incarnation."
);
define_browser_identity!(
    WebContentsId,
    NEXT_WEB_CONTENTS_ID,
    "WebContents id",
    "Identity of one stable WebContents incarnation."
);
define_browser_identity!(
    MainFrameSlotId,
    NEXT_MAIN_FRAME_SLOT_ID,
    "main frame slot id",
    "Identity of one stable main-frame slot within a WebContents."
);
define_browser_identity!(
    DocumentId,
    NEXT_DOCUMENT_ID,
    "Document id",
    "Identity of one replaceable browser-owned Document incarnation."
);
define_browser_identity!(
    NavigationId,
    NEXT_NAVIGATION_ID,
    "navigation id",
    "Identity of one browser-owned navigation attempt."
);
define_browser_identity!(
    BrowserRequestId,
    NEXT_BROWSER_REQUEST_ID,
    "browser request id",
    "Identity of one browser-owned request decision, independent of protocol request IDs."
);

/// Stable routing capability for one browser-owned WebContents.
///
/// This carries only physical Browser identities. Frontend target and session
/// identifiers are deliberately resolved before this capability is created.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct WebContentsHandle {
    context: BrowserContextId,
    web_contents: WebContentsId,
}

impl WebContentsHandle {
    pub const fn new(context: BrowserContextId, web_contents: WebContentsId) -> Self {
        Self {
            context,
            web_contents,
        }
    }

    pub const fn context(self) -> BrowserContextId {
        self.context
    }

    pub const fn id(self) -> WebContentsId {
        self.web_contents
    }
}

/// Exact routing capability for one replaceable browser-owned Document.
///
/// A delayed operation must present this complete capability again at commit;
/// matching only the stable WebContents is insufficient after navigation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DocumentHandle {
    web_contents: WebContentsHandle,
    document: DocumentId,
}

impl DocumentHandle {
    pub const fn new(web_contents: WebContentsHandle, document: DocumentId) -> Self {
        Self {
            web_contents,
            document,
        }
    }

    pub const fn web_contents(self) -> WebContentsHandle {
        self.web_contents
    }

    pub const fn id(self) -> DocumentId {
        self.document
    }
}

static NEXT_BROWSER_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// Process-monotonic sequence of completed Browser semantic occurrences.
///
/// Unlike the object identities above, this value defines publication order.
/// It is allocated only after the Browser mutation represented by an
/// occurrence has completed successfully.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BrowserSequence(NonZeroU64);

impl BrowserSequence {
    /// Allocates the next process-wide Browser occurrence sequence.
    pub fn allocate() -> Self {
        Self(allocate_nonzero_u64(
            &NEXT_BROWSER_SEQUENCE,
            "Browser sequence",
        ))
    }

    /// Returns the ordered value for diagnostics and projection fences.
    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

#[cfg(any(test, feature = "test-support"))]
impl DocumentId {
    /// Constructs a deterministic identity for cross-crate tests.
    #[doc(hidden)]
    pub fn from_raw_for_test(raw: u64) -> Self {
        Self(NonZeroU64::new(raw).expect("test Document id must be nonzero"))
    }
}

fn allocate_nonzero_u64(counter: &AtomicU64, name: &str) -> NonZeroU64 {
    let raw = counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1)
        })
        .unwrap_or_else(|_| panic!("{name} exhausted"));
    NonZeroU64::new(raw).unwrap_or_else(|| panic!("{name} allocator returned zero"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn browser_object_incarnations_are_unique() {
        assert_ne!(BrowserContextId::allocate(), BrowserContextId::allocate());
        assert_ne!(WebContentsId::allocate(), WebContentsId::allocate());
        assert_ne!(MainFrameSlotId::allocate(), MainFrameSlotId::allocate());
        assert_ne!(DocumentId::allocate(), DocumentId::allocate());
        assert_ne!(NavigationId::allocate(), NavigationId::allocate());
        assert_ne!(BrowserRequestId::allocate(), BrowserRequestId::allocate());
        assert!(BrowserSequence::allocate() < BrowserSequence::allocate());
    }

    #[test]
    fn optional_browser_identities_preserve_the_nonzero_niche() {
        assert_eq!(
            size_of::<Option<BrowserContextId>>(),
            size_of::<BrowserContextId>()
        );
        assert_eq!(
            size_of::<Option<WebContentsId>>(),
            size_of::<WebContentsId>()
        );
        assert_eq!(
            size_of::<Option<MainFrameSlotId>>(),
            size_of::<MainFrameSlotId>()
        );
        assert_eq!(size_of::<Option<DocumentId>>(), size_of::<DocumentId>());
        assert_eq!(size_of::<Option<NavigationId>>(), size_of::<NavigationId>());
        assert_eq!(
            size_of::<Option<BrowserRequestId>>(),
            size_of::<BrowserRequestId>()
        );
        assert_eq!(
            size_of::<Option<BrowserSequence>>(),
            size_of::<BrowserSequence>()
        );
    }

    #[test]
    fn document_handle_preserves_the_complete_physical_route() {
        let context = BrowserContextId::allocate();
        let contents = WebContentsId::allocate();
        let document = DocumentId::allocate();
        let handle = DocumentHandle::new(WebContentsHandle::new(context, contents), document);

        assert_eq!(handle.web_contents().context(), context);
        assert_eq!(handle.web_contents().id(), contents);
        assert_eq!(handle.id(), document);
    }
}
