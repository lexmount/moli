use super::*;

impl<'s> ResolvedSimpleEventTarget<'s> {
    pub(crate) fn add_event_listener(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        args: &v8::FunctionCallbackArguments<'s>,
    ) {
        simple_object_event_target_add_listener_for_target(
            scope,
            args,
            self.identity.listener_owner,
            self.registry,
        );
    }

    pub(crate) fn remove_event_listener(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        args: &v8::FunctionCallbackArguments<'s>,
    ) {
        simple_object_event_target_remove_listener_in_registry(scope, args, self.registry);
    }

    pub(crate) fn dispatch_event(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        args: &v8::FunctionCallbackArguments<'s>,
        rv: &mut v8::ReturnValue<'s, v8::Value>,
    ) {
        simple_object_event_target_dispatch(scope, args, self, rv);
    }
}
