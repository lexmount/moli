use std::{
    num::NonZeroU64,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_TARGET_PAGE_ATTACHMENT_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_NAVIGATION_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

/// Opaque identity for one current or reserved target Page attachment.
///
/// The id is allocated when renderer Page construction is reserved and remains
/// stable when that exact Page is installed. It also keys the attachment's
/// directly terminable residence token.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct TargetPageAttachmentId(NonZeroU64);

impl TargetPageAttachmentId {
    pub(crate) fn allocate() -> Self {
        Self(allocate_nonzero_u64(
            &NEXT_TARGET_PAGE_ATTACHMENT_ID,
            "target Page attachment id",
        ))
    }

    pub(crate) fn get(self) -> u64 {
        self.0.get()
    }

    #[cfg(test)]
    pub(crate) fn from_raw_for_test(raw: u64) -> Self {
        Self(NonZeroU64::new(raw).expect("test Page attachment id must be nonzero"))
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct NavigationRequestId(NonZeroU64);

impl NavigationRequestId {
    pub(crate) fn allocate() -> Self {
        Self(allocate_nonzero_u64(
            &NEXT_NAVIGATION_REQUEST_ID,
            "navigation request id",
        ))
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
    fn attachment_and_navigation_ids_are_nonzero_and_distinct() {
        let first_attachment = TargetPageAttachmentId::allocate();
        let second_attachment = TargetPageAttachmentId::allocate();
        let first_navigation = NavigationRequestId::allocate();
        let second_navigation = NavigationRequestId::allocate();

        assert_ne!(first_attachment, second_attachment);
        assert_ne!(first_navigation, second_navigation);
        assert_ne!(first_attachment.get(), 0);
        assert_ne!(first_navigation.0.get(), 0);
    }

    #[test]
    fn optional_ids_preserve_the_nonzero_niche() {
        assert_eq!(
            size_of::<Option<TargetPageAttachmentId>>(),
            size_of::<TargetPageAttachmentId>()
        );
        assert_eq!(
            size_of::<Option<NavigationRequestId>>(),
            size_of::<NavigationRequestId>()
        );
    }
}
