//! Wrapper-world membership, separate from a concrete Realm's authority.

use std::{
    num::NonZeroU64,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
};

/// An isolated wrapper world can have a distinct Context in several Windows.
/// Names are inspector metadata, and a Context token identifies only one
/// Realm. Neither is a suitable identity for that shared wrapper world.
///
/// Allocation is process-wide because related Pages can share an isolate but
/// have separate Hosts and Host-local Realm-token counters. This identity is
/// never used to authorize a Window operation or to revive a retired Realm.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct IsolatedWindowWorldId(NonZeroU64);

impl IsolatedWindowWorldId {
    pub(crate) fn allocate() -> anyhow::Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let value = NEXT
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| anyhow::anyhow!("isolated Window world identity exhausted"))?;
        Ok(Self(
            NonZeroU64::new(value).expect("isolated world identities start at one"),
        ))
    }
}

pub(crate) fn install_isolated_window_world_for_context(
    context: v8::Local<'_, v8::Context>,
    world: IsolatedWindowWorldId,
) {
    assert!(
        context.get_slot::<IsolatedWindowWorldId>().is_none(),
        "a Context's wrapper-world identity is immutable"
    );
    let previous = context.set_slot(Rc::new(world));
    debug_assert!(previous.is_none());
}

/// Main-world Contexts carry no isolated-world slot. This is a native slot
/// read, with no property lookup, wrapper creation, or layout side effect.
pub(crate) fn isolated_window_world_for_context(
    context: v8::Local<'_, v8::Context>,
) -> Option<IsolatedWindowWorldId> {
    context
        .get_slot::<IsolatedWindowWorldId>()
        .as_deref()
        .copied()
}
