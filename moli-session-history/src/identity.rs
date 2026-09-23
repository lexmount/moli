use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_NAVIGATION_HISTORY_DOCUMENT_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_NAVIGATION_HISTORY_ENTRY_KEY: AtomicU64 = AtomicU64::new(1);

/// Opaque identity shared by session-history entries that belong to the same
/// `Document`.
///
/// The serialized token is carried through the renderer's hidden Navigation
/// slots, but its contents have no meaning. In particular, callers must not
/// derive a new identity from a URL, a history index, or a previous token.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NavigationHistoryDocumentId(String);

impl NavigationHistoryDocumentId {
    pub fn allocate() -> Self {
        allocate_navigation_history_document_id(&NEXT_NAVIGATION_HISTORY_DOCUMENT_ID)
    }

    /// Restores an identity previously stored in a renderer-owned runtime
    /// slot. Equality remains opaque; the token is never parsed.
    pub fn from_serialized(token: String) -> Self {
        Self(token)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn allocate_navigation_history_document_id(counter: &AtomicU64) -> NavigationHistoryDocumentId {
    let raw = counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1)
        })
        .expect("Navigation History Document id allocator exhausted");
    NavigationHistoryDocumentId(format!("document-{raw}"))
}

/// Opaque identity for one session-history slot exposed to Navigation API.
///
/// Same-origin replacement retains the key; push and cross-origin
/// replacement allocate a fresh key. The token is never derived from a URL,
/// history index, or Document id.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NavigationHistoryEntryKey(String);

impl NavigationHistoryEntryKey {
    pub fn allocate() -> Self {
        allocate_navigation_history_entry_key(&NEXT_NAVIGATION_HISTORY_ENTRY_KEY)
    }

    pub fn from_serialized(token: String) -> Self {
        Self(token)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::ops::Deref for NavigationHistoryEntryKey {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

fn allocate_navigation_history_entry_key(counter: &AtomicU64) -> NavigationHistoryEntryKey {
    let raw = counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1)
        })
        .expect("Navigation History entry key allocator exhausted");
    NavigationHistoryEntryKey(format!("key-{raw}"))
}

static NEXT_CONTEXT: AtomicU64 = AtomicU64::new(1);
static NEXT_STEP: AtomicU64 = AtomicU64::new(1);
static NEXT_REVISION: AtomicU64 = AtomicU64::new(1);

fn allocate(counter: &AtomicU64) -> u64 {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .expect("session history identity allocator exhausted")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionHistoryContextId(u64);

impl SessionHistoryContextId {
    pub const ROOT: Self = Self(0);

    pub fn allocate() -> Self {
        Self(allocate(&NEXT_CONTEXT))
    }

    /// Opaque equality token for an adapter's private admission snapshot.
    pub fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionHistoryStepId(u64);

impl SessionHistoryStepId {
    pub(crate) fn allocate() -> Self {
        Self(allocate(&NEXT_STEP))
    }

    pub fn raw(self) -> u64 {
        self.0
    }

    /// Restore an opaque identity held by a renderer's private task slot.
    pub fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
}

/// Opaque version of the joint timeline and entry identities. Context topology
/// is checked against the plan's transitions separately: attaching an unchanged
/// context must not cancel an admitted navigation in another frame. Independent
/// timeline mutations get distinct revisions even if they return to the same cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionHistoryRevision(u64);

impl SessionHistoryRevision {
    pub(crate) fn allocate() -> Self {
        Self(allocate(&NEXT_REVISION))
    }

    /// Store the token in an adapter's private continuation slot. It is only
    /// meaningful for equality, never as a cursor, counter, or time value.
    pub fn raw(self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_allocators_reject_exhaustion_without_wrapping() {
        for allocate_id in [
            |counter: &AtomicU64| {
                allocate(counter);
            },
            |counter: &AtomicU64| {
                allocate_navigation_history_document_id(counter);
            },
            |counter: &AtomicU64| {
                allocate_navigation_history_entry_key(counter);
            },
        ] {
            let counter = AtomicU64::new(u64::MAX);
            assert!(std::panic::catch_unwind(|| allocate_id(&counter)).is_err());
            assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
        }
    }
}
