use crate::{document_runtime::DomHandle, native_bridge::NativeDomHostId};
use std::{cell::Cell, collections::HashSet, rc::Rc};

struct PendingConstruction {
    constructor: v8::Global<v8::Function>,
    wrapper: Option<v8::Global<v8::Object>>,
    view: Rc<SharedPendingConstruction>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PendingConstructionKind {
    ExistingElementUpgrade,
    SynchronousCreateElement,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ConstructionState {
    Available,
    Consumed,
    Finished,
}

#[derive(Clone, Copy)]
pub(crate) struct CustomElementConstructionId {
    pub(crate) owner: NativeDomHostId,
    pub(crate) handle: DomHandle,
}

/// A weak view of a construction owned by one native DOM host. The isolate
/// may locate this view while calling super() in another realm; only the
/// owning stack roots values and stores unqualified node state.
pub(super) struct SharedPendingConstruction {
    id: CustomElementConstructionId,
    constructor: v8::Weak<v8::Function>,
    registry_realm: v8::Weak<v8::Context>,
    wrapper: v8::Weak<v8::Object>,
    kind: PendingConstructionKind,
    state: Cell<ConstructionState>,
}

pub(crate) enum PendingCustomElementConstruction<'s> {
    Wrapper(v8::Local<'s, v8::Object>, CustomElementConstructionId),
    AlreadyConstructed(CustomElementConstructionId),
}

impl SharedPendingConstruction {
    pub(super) fn matches(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        new_target: v8::Local<'_, v8::Function>,
    ) -> bool {
        self.state.get() != ConstructionState::Finished
            && self.registry_realm.to_local(scope) == Some(scope.get_current_context())
            && self
                .constructor
                .to_local(scope)
                .is_some_and(|constructor| constructor.strict_equals(new_target.into()))
    }

    pub(super) fn take<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Option<PendingCustomElementConstruction<'s>> {
        if self.state.get() == ConstructionState::Consumed {
            return Some(PendingCustomElementConstruction::AlreadyConstructed(
                self.id,
            ));
        }
        if self.state.get() == ConstructionState::Finished {
            return None;
        }
        let wrapper = self.wrapper.to_local(scope)?;
        self.state.set(match self.kind {
            PendingConstructionKind::ExistingElementUpgrade => ConstructionState::Consumed,
            PendingConstructionKind::SynchronousCreateElement => ConstructionState::Finished,
        });
        Some(PendingCustomElementConstruction::Wrapper(wrapper, self.id))
    }
}

#[derive(Default)]
pub(super) struct CustomElementConstructionStack {
    stack: Vec<PendingConstruction>,
    constructing_handles: HashSet<DomHandle>,
    failed_handles: HashSet<DomHandle>,
}

impl CustomElementConstructionStack {
    pub(super) fn begin_existing_element_upgrade(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        owner: NativeDomHostId,
        registry_realm: v8::Local<'_, v8::Context>,
        constructor: v8::Local<'_, v8::Function>,
        wrapper: v8::Local<'_, v8::Object>,
        handle: DomHandle,
    ) {
        self.begin_with_kind(
            scope,
            CustomElementConstructionId { owner, handle },
            registry_realm,
            constructor,
            wrapper,
            PendingConstructionKind::ExistingElementUpgrade,
        );
    }

    pub(super) fn begin_synchronous_create_element(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        owner: NativeDomHostId,
        registry_realm: v8::Local<'_, v8::Context>,
        constructor: v8::Local<'_, v8::Function>,
        wrapper: v8::Local<'_, v8::Object>,
        handle: DomHandle,
    ) {
        self.begin_with_kind(
            scope,
            CustomElementConstructionId { owner, handle },
            registry_realm,
            constructor,
            wrapper,
            PendingConstructionKind::SynchronousCreateElement,
        );
    }

    fn begin_with_kind(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        id: CustomElementConstructionId,
        registry_realm: v8::Local<'_, v8::Context>,
        constructor: v8::Local<'_, v8::Function>,
        wrapper: v8::Local<'_, v8::Object>,
        kind: PendingConstructionKind,
    ) {
        let view = Rc::new(SharedPendingConstruction {
            id,
            constructor: v8::Weak::new(scope, constructor),
            registry_realm: v8::Weak::new(scope, registry_realm),
            wrapper: v8::Weak::new(scope, wrapper),
            kind,
            state: Cell::new(ConstructionState::Available),
        });
        super::CustomElementRegistryBindings::for_isolate(scope).register_construction(&view);
        self.constructing_handles.insert(id.handle);
        self.stack.push(PendingConstruction {
            constructor: v8::Global::new(scope, constructor),
            wrapper: Some(v8::Global::new(scope, wrapper)),
            view,
        });
    }

    pub(super) fn discard(&mut self, handle: DomHandle) {
        self.constructing_handles.remove(&handle);
        if let Some(index) = self
            .stack
            .iter()
            .rposition(|pending| pending.view.id.handle == handle)
        {
            let pending = self.stack.remove(index);
            pending.view.state.set(ConstructionState::Finished);
        }
    }

    pub(super) fn take_pending_wrapper_for<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        new_target: v8::Local<'_, v8::Function>,
    ) -> Option<PendingCustomElementConstruction<'s>> {
        let index = self.stack.iter().rposition(|pending| {
            pending.view.state.get() != ConstructionState::Finished
                && v8::Local::new(scope, &pending.constructor).strict_equals(new_target.into())
        })?;
        let pending = &mut self.stack[index];
        let result = pending.view.take(scope);
        pending.wrapper.take();
        if pending.view.kind == PendingConstructionKind::SynchronousCreateElement {
            self.stack.remove(index);
        }
        result
    }

    pub(super) fn has_pending_wrapper_for(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        new_target: v8::Local<'_, v8::Function>,
    ) -> bool {
        self.stack.iter().any(|pending| {
            pending.view.state.get() != ConstructionState::Finished
                && v8::Local::new(scope, &pending.constructor).strict_equals(new_target.into())
        })
    }

    pub(super) fn is_pending_handle(&self, handle: DomHandle) -> bool {
        self.constructing_handles.contains(&handle)
    }
    pub(super) fn is_already_constructed(&self, handle: DomHandle) -> bool {
        self.stack.iter().any(|pending| {
            pending.view.id.handle == handle
                && pending.view.state.get() == ConstructionState::Consumed
        })
    }
    pub(super) fn mark_failed(&mut self, handle: DomHandle) {
        self.failed_handles.insert(handle);
    }
    pub(super) fn clear_failed(&mut self, handle: DomHandle) {
        self.failed_handles.remove(&handle);
    }
    pub(super) fn is_failed(&self, handle: DomHandle) -> bool {
        self.failed_handles.contains(&handle)
    }
    pub(super) fn owns_handle(&self, handle: DomHandle) -> bool {
        self.constructing_handles.contains(&handle)
            || self.failed_handles.contains(&handle)
            || self
                .stack
                .iter()
                .any(|pending| pending.view.id.handle == handle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn constructor<'s>(
        _scope: &mut v8::PinScope<'s, '_>,
        _args: v8::FunctionCallbackArguments<'s>,
        _rv: v8::ReturnValue<'s>,
    ) {
    }

    #[test]
    fn shared_construction_views_keep_native_ids_qualified_and_follow_nested_order() {
        crate::ensure_v8_for_test();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let other_context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let function = v8::Function::new(scope, constructor).unwrap();
        let wrapper = v8::Object::new(scope);
        let source_owner = NativeDomHostId::default();
        let target_owner = NativeDomHostId::default();
        let handle = DomHandle::new(7);
        let mut source = CustomElementConstructionStack::default();
        let mut target = CustomElementConstructionStack::default();
        target.begin_existing_element_upgrade(
            scope,
            target_owner,
            context,
            function,
            wrapper,
            handle,
        );
        assert!(!source.has_pending_wrapper_for(scope, function));
        let bindings = super::super::CustomElementRegistryBindings::for_isolate(scope);
        {
            let scope = &mut v8::ContextScope::new(scope, other_context);
            assert!(
                !bindings.has_pending_construction(scope, function),
                "a foreign super constructor must not consume another realm's construction"
            );
        }
        source.begin_existing_element_upgrade(
            scope,
            source_owner,
            context,
            function,
            wrapper,
            handle,
        );
        let Some(PendingCustomElementConstruction::Wrapper(_, id)) =
            bindings.take_pending_construction(scope, function)
        else {
            panic!("nested source construction");
        };
        assert_eq!(id.owner, source_owner);
        assert_eq!(id.handle, handle);
        assert!(source.is_already_constructed(handle));
        assert!(!target.is_already_constructed(handle));
        source.discard(handle);
        let Some(PendingCustomElementConstruction::Wrapper(value, id)) =
            bindings.take_pending_construction(scope, function)
        else {
            panic!("foreign target construction");
        };
        assert_eq!(value, wrapper);
        assert_eq!(id.owner, target_owner);
        assert_eq!(id.handle, handle);
        assert!(target.is_already_constructed(handle));
        assert!(!source.owns_handle(handle));
        assert!(matches!(
            bindings.take_pending_construction(scope, function),
            Some(PendingCustomElementConstruction::AlreadyConstructed(_))
        ));
        target.discard(handle);
        assert!(!bindings.has_pending_construction(scope, function));
        target.begin_synchronous_create_element(
            scope,
            target_owner,
            other_context,
            function,
            wrapper,
            handle,
        );
        assert!(!bindings.has_pending_construction(scope, function));
        {
            let scope = &mut v8::ContextScope::new(scope, other_context);
            assert!(bindings.has_pending_construction(scope, function));
            assert!(matches!(
                bindings.take_pending_construction(scope, function),
                Some(PendingCustomElementConstruction::Wrapper(_, _))
            ));
        }
        target.discard(handle);
    }
}
