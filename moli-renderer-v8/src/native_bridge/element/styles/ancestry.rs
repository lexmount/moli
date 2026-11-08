use std::{collections::HashMap, ops::ControlFlow};

use crate::document_runtime::DomHandle;

/// Memoized ancestor queries for one synchronous, read-only style observation.
/// Neither node contents nor tree edges are copied. Stop at the first ancestor
/// already resolved by this observation, independent of the caller's node order.
#[derive(Default)]
pub(super) struct AncestorQueries<T> {
    resolved: HashMap<DomHandle, T>,
    pending: Vec<DomHandle>,
}

impl<T: Copy> AncestorQueries<T> {
    pub(super) fn resolve(
        &mut self,
        handle: DomHandle,
        mut step: impl FnMut(DomHandle) -> ControlFlow<T, DomHandle>,
    ) -> T {
        self.pending.clear();
        let mut current = handle;
        let result = loop {
            if let Some(&result) = self.resolved.get(&current) {
                break result;
            }
            self.pending.push(current);
            match step(current) {
                ControlFlow::Break(result) => break result,
                ControlFlow::Continue(parent) => current = parent,
            }
        };
        for node in self.pending.drain(..) {
            self.resolved.insert(node, result);
        }
        result
    }
}
