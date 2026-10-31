//! Private subscription signals live in their producer/consumer's V8 graph.
//! Finalizers only enqueue identity cleanup; they never borrow an abort store.

use std::{cell::RefCell, collections::HashSet, rc::Rc};

use crate::util::{get_private_value, set_private_value};

const ALGORITHMS: &str = "__moliSubscriptionAbortAlgorithms";
const DEPENDENTS: &str = "__moliSubscriptionAbortDependents";

#[derive(Default)]
pub(crate) struct SubscriptionSignals {
    collected: Rc<RefCell<Vec<u32>>>,
}

impl SubscriptionSignals {
    pub(crate) fn track<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        signal: v8::Local<'s, v8::Object>,
        id: u32,
    ) -> crate::util::RealmObjectHandle {
        let algorithms = v8::Set::new(scope);
        set_private_value(scope, signal, ALGORITHMS, algorithms.into());
        let collected = self.collected.clone();
        crate::util::RealmObjectHandle::Traced(v8::Weak::with_finalizer(
            scope,
            signal,
            Box::new(move |_| collected.borrow_mut().push(id)),
        ))
    }

    fn algorithms<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        signal: v8::Local<'s, v8::Object>,
    ) -> Option<v8::Local<'s, v8::Set>> {
        get_private_value(scope, signal, ALGORITHMS)
            .and_then(|value| v8::Local::<v8::Set>::try_from(value).ok())
    }

    /// A live private signal owns ordinary cancellation callbacks through V8,
    /// so callback data may point back to the signal without becoming a Rust root.
    pub(crate) fn retain_algorithm<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        signal: v8::Local<'s, v8::Object>,
        algorithm: v8::Local<'s, v8::Function>,
        sources: &[v8::Local<'s, v8::Object>],
    ) -> bool {
        let Some(algorithms) = Self::algorithms(scope, signal) else {
            return false;
        };
        algorithms
            .add(scope, algorithm.into())
            .expect("subscription abort callback should be retained");
        Self::retain_in_sources(scope, signal, sources);
        true
    }

    pub(crate) fn release_algorithm<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        signal: v8::Local<'s, v8::Object>,
        algorithm: v8::Local<'s, v8::Function>,
        sources: &[v8::Local<'s, v8::Object>],
    ) {
        if let Some(algorithms) = Self::algorithms(scope, signal) {
            let _ = algorithms.delete(scope, algorithm.into());
            if algorithms.size() == 0 {
                Self::release_from_sources(scope, signal, sources);
            }
        }
    }

    /// Abort dispatch must first prepare all callbacks as scope-local handles.
    /// Earlier steps can unregister later ones and trigger GC while reentering JS.
    pub(crate) fn clear_algorithms<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        signal: v8::Local<'s, v8::Object>,
        sources: &[v8::Local<'s, v8::Object>],
    ) {
        if let Some(algorithms) = Self::algorithms(scope, signal) {
            algorithms.clear();
            Self::release_from_sources(scope, signal, sources);
        }
    }

    // Native dependency IDs are weak. Only pending ordinary callbacks need a
    // source-to-dependent edge, e.g. first({signal}) with a discarded Promise.
    // Keeping this edge in V8 lets abandoned private source cycles be collected;
    // removing it with the last callback avoids retaining completed inner signals.
    fn retain_in_sources<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        signal: v8::Local<'s, v8::Object>,
        sources: &[v8::Local<'s, v8::Object>],
    ) {
        for &source in sources {
            let dependents = get_private_value(scope, source, DEPENDENTS)
                .and_then(|value| v8::Local::<v8::Set>::try_from(value).ok())
                .unwrap_or_else(|| {
                    let dependents = v8::Set::new(scope);
                    set_private_value(scope, source, DEPENDENTS, dependents.into());
                    dependents
                });
            dependents
                .add(scope, signal.into())
                .expect("pending subscription dependent should be retained");
        }
    }

    fn release_from_sources<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        signal: v8::Local<'s, v8::Object>,
        sources: &[v8::Local<'s, v8::Object>],
    ) {
        for &source in sources {
            if let Some(dependents) = get_private_value(scope, source, DEPENDENTS)
                .and_then(|value| v8::Local::<v8::Set>::try_from(value).ok())
            {
                let _ = dependents.delete(scope, signal.into());
                if dependents.size() == 0 {
                    set_private_value(scope, source, DEPENDENTS, v8::undefined(scope).into());
                }
            }
        }
    }

    pub(crate) fn take_collected(&self) -> HashSet<u32> {
        std::mem::take(&mut *self.collected.borrow_mut())
            .into_iter()
            .collect()
    }
}
