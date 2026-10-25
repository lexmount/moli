//! Private subscription signals live in their producer/consumer's V8 graph.
//! Finalizers only enqueue identity cleanup; they never borrow an abort store.

use std::{cell::RefCell, collections::HashSet, rc::Rc};

#[derive(Default)]
pub(crate) struct SubscriptionSignals {
    collected: Rc<RefCell<Vec<u32>>>,
}

impl SubscriptionSignals {
    pub(crate) fn track(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        signal: v8::Local<'_, v8::Object>,
        id: u32,
    ) -> crate::util::RealmObjectHandle {
        let collected = self.collected.clone();
        crate::util::RealmObjectHandle::Traced(v8::Weak::with_finalizer(
            scope,
            signal,
            Box::new(move |_| collected.borrow_mut().push(id)),
        ))
    }

    pub(crate) fn take_collected(&self) -> HashSet<u32> {
        std::mem::take(&mut *self.collected.borrow_mut())
            .into_iter()
            .collect()
    }
}
