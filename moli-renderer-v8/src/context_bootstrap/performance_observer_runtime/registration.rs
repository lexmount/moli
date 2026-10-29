//! PerformanceObserver's private callback residence and common registration.
//! The timeline owns batching; the common runtime owns identity lookup,
//! activation and cleanup for both Window and Worker.

use std::rc::Rc;

use moli_webidl_callback::WebIdlCallbackFunction;

use super::*;
use crate::common_runtime::{
    CallbackRegistrationId, CallbackRegistrationKind, CallbackResidence, CommonRuntimeState,
};
use crate::util::get_private_value;
use crate::webidl_callback_execution::PreparedWebIdlCallbackExecution;

pub(super) struct PerformanceObserverRegistration<'s> {
    observer: v8::Local<'s, v8::Object>,
    id: CallbackRegistrationId,
    common: Rc<CommonRuntimeState>,
}

pub(super) fn register_performance_observer_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
    callback: WebIdlCallbackFunction,
) -> Option<(
    u32,
    v8::Local<'s, v8::Object>,
    v8::Local<'s, v8::Object>,
    v8::Local<'s, v8::Object>,
)> {
    let common = CommonRuntimeState::from_scope(scope)?;
    let (id, callback, relevant, incumbent) = common
        .callbacks
        .register(
            scope,
            observer,
            callback,
            CallbackRegistrationKind::PerformanceObserver,
        )?
        .into_parts();
    Some((id.as_u32(), callback, relevant, incumbent))
}

impl<'s> PerformanceObserverRegistration<'s> {
    pub(super) fn from_observer(
        scope: &mut v8::PinScope<'s, '_>,
        observer: v8::Local<'s, v8::Object>,
    ) -> Option<Self> {
        let id = get_private_value(scope, observer, PERFORMANCE_OBSERVER_CALLBACK_ID_SLOT)?
            .number_value(scope)?;
        let context = observer.get_creation_context(scope)?;
        Some(Self {
            observer,
            id: CallbackRegistrationId::from_number(id)?,
            common: CommonRuntimeState::from_context(context)?,
        })
    }

    pub(super) fn is_current(&self, scope: &mut v8::PinScope<'s, '_>) -> bool {
        self.common.callbacks.is_current(
            scope,
            self.id,
            self.observer,
            CallbackRegistrationKind::PerformanceObserver,
        )
    }

    pub(super) fn activate(&self, scope: &mut v8::PinScope<'s, '_>) -> bool {
        self.common
            .callbacks
            .activate_performance_observer(scope, self.id, self.observer)
    }

    pub(super) fn deactivate(&self, scope: &mut v8::PinScope<'s, '_>) {
        self.common
            .callbacks
            .deactivate_performance_observer(scope, self.id, self.observer);
    }

    pub(super) fn prepare_delivery(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Option<PreparedWebIdlCallbackExecution> {
        let object = |scope: &mut v8::PinScope<'s, '_>, slot| {
            get_private_value(scope, self.observer, slot)
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        };
        let callback = object(scope, PERFORMANCE_OBSERVER_CALLBACK_VALUE_SLOT)?;
        let relevant = object(scope, PERFORMANCE_OBSERVER_CALLBACK_RELEVANT_GLOBAL_SLOT)?;
        let incumbent = object(scope, PERFORMANCE_OBSERVER_CALLBACK_INCUMBENT_GLOBAL_SLOT)?;
        let residence = CallbackResidence::from_parts(self.id, callback, relevant, incumbent);
        self.common.callbacks.prepare(
            scope,
            self.observer,
            CallbackRegistrationKind::PerformanceObserver,
            residence,
        )
    }
}

pub(super) fn performance_observer_callback_is_current<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) -> bool {
    PerformanceObserverRegistration::from_observer(scope, observer)
        .is_some_and(|registration| registration.is_current(scope))
}

pub(super) fn active_performance_observers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    timeline_context: v8::Local<'s, v8::Context>,
) -> Vec<v8::Local<'s, v8::Object>> {
    CommonRuntimeState::from_context(timeline_context)
        .map(|common| {
            common
                .callbacks
                .active_performance_observers(scope, timeline_context)
        })
        .unwrap_or_default()
}
