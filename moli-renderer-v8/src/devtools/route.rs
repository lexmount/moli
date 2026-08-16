//! Isolate-local routing identity shared by DevTools ingress and its renderer executor.

use std::num::NonZeroUsize;

/// Identifies one isolate-local Inspector session executor.
///
/// MainThread owner wakes and IO interrupts deliberately share only this
/// final executor identity. Their queues, admission and wake mechanisms stay
/// physically separate.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct RendererInspectorSessionExecutorRouteId(NonZeroUsize);

impl RendererInspectorSessionExecutorRouteId {
    pub(crate) fn new(raw: usize) -> Self {
        Self(NonZeroUsize::new(raw).expect("Inspector session executor route ID must be non-zero"))
    }
}
