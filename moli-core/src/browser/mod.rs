//! Protocol-neutral identities for browser-owned objects.
//!
//! These identities name physical browser object incarnations. They are not
//! CDP wire identifiers and their numeric representation does not define an
//! ordering relationship.

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
    }
}
