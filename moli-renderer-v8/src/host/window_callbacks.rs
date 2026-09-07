//! Typed user-callback payloads that are scheduled on the Window timer heap.
//!
//! The timer heap is only the current delay/deadline transport. It does not
//! turn animation-frame, idle, or Geolocation callbacks into timer callback
//! algorithms.
//! This residence keeps the page-supplied Web IDL callback and its exact
//! callback Realm separate from browser-created timer functions.

use moli_webidl_callback::WebIdlCallbackFunction;

use crate::{
    native_bridge::{JsContextHost, RuntimeObservableContextToken, WindowExecutionContextIdentity},
    window_webidl_callback::{
        WindowWebIdlCallbackFunction, WindowWebIdlCallbackFunctionOutcome,
        invoke_window_webidl_callback_function,
    },
};

/// API-specific body data for one Window Web IDL callback scheduled on the
/// shared timer heap.
///
/// `Timer` consumes the task's stored extra arguments and uses the target
/// Window as `this`. Animation-frame, idle, and Geolocation callbacks use Web
/// IDL's `undefined` callback this value. Geolocation deliberately records its
/// watch identity here because `clearWatch()` owns cancellation while the
/// timer heap remains only the lightweight asynchronous transport.
#[derive(Clone, Copy, Debug)]
pub(super) enum WindowWebIdlCallbackTaskKind {
    Timer,
    AnimationFrame { timestamp: f64 },
    Idle { timeout_deadline_ms: f64 },
    Geolocation { watch_id: Option<i32> },
}

pub(super) struct ScheduledWindowWebIdlCallback {
    callback: WindowWebIdlCallbackFunction,
    target_receiver: v8::Global<v8::Object>,
    kind: WindowWebIdlCallbackTaskKind,
}

impl ScheduledWindowWebIdlCallback {
    pub(super) fn new(
        scope: &mut v8::PinScope<'_, '_>,
        host: &JsContextHost,
        callback: WebIdlCallbackFunction,
        target_receiver: v8::Local<'_, v8::Object>,
        kind: WindowWebIdlCallbackTaskKind,
    ) -> Option<Self> {
        let callback = WindowWebIdlCallbackFunction::new(scope, host, callback);
        callback.relevant_identity()?;
        Some(Self {
            callback,
            target_receiver: v8::Global::new(scope, target_receiver),
            kind,
        })
    }

    pub(super) fn relevant_identity(&self) -> Option<WindowExecutionContextIdentity> {
        self.callback.relevant_identity()
    }

    pub(super) fn relevant_context<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Option<v8::Local<'s, v8::Context>> {
        self.callback.relevant_context(scope)
    }

    pub(super) fn realm_token(&self) -> Option<RuntimeObservableContextToken> {
        self.relevant_identity()
            .map(WindowExecutionContextIdentity::realm_token)
    }

    pub(super) fn is_unexpired_idle_callback_at(&self, now_ms: f64) -> bool {
        match self.kind {
            WindowWebIdlCallbackTaskKind::Idle {
                timeout_deadline_ms,
            } => timeout_deadline_ms < 0.0 || now_ms < timeout_deadline_ms,
            WindowWebIdlCallbackTaskKind::Timer
            | WindowWebIdlCallbackTaskKind::AnimationFrame { .. }
            | WindowWebIdlCallbackTaskKind::Geolocation { .. } => false,
        }
    }

    pub(super) fn is_geolocation_watch<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        geolocation: v8::Local<'s, v8::Object>,
        watch_id: i32,
    ) -> bool {
        matches!(
            self.kind,
            WindowWebIdlCallbackTaskKind::Geolocation {
                watch_id: Some(candidate)
            } if candidate == watch_id
        ) && v8::Local::new(scope, &self.target_receiver).strict_equals(geolocation.into())
    }

    pub(super) fn invoke<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        host_ptr: *mut JsContextHost,
        extra_args: &[v8::Global<v8::Value>],
    ) -> WindowWebIdlCallbackFunctionOutcome {
        let prepared = self.callback.prepare(scope);
        let mut arguments = Vec::new();
        let receiver = match self.kind {
            WindowWebIdlCallbackTaskKind::Timer => {
                arguments.extend(
                    extra_args
                        .iter()
                        .map(|argument| v8::Local::new(scope, argument)),
                );
                v8::Local::new(scope, &self.target_receiver).into()
            }
            WindowWebIdlCallbackTaskKind::AnimationFrame { timestamp } => {
                crate::window_host::finish_animation_frame_callback_batch(scope, timestamp);
                arguments.push(v8::Number::new(scope, timestamp).into());
                v8::undefined(scope).into()
            }
            WindowWebIdlCallbackTaskKind::Idle {
                timeout_deadline_ms,
            } => {
                let did_timeout = timeout_deadline_ms >= 0.0
                    && crate::window_host::current_time_ms() >= timeout_deadline_ms;
                let Some(deadline) =
                    crate::window_host::build_window_idle_deadline(scope, did_timeout)
                else {
                    return WindowWebIdlCallbackFunctionOutcome::Retired;
                };
                arguments.push(deadline.into());
                v8::undefined(scope).into()
            }
            WindowWebIdlCallbackTaskKind::Geolocation { .. } => {
                assert_eq!(
                    extra_args.len(),
                    1,
                    "a Geolocation task must retain exactly one result argument"
                );
                arguments.push(v8::Local::new(scope, &extra_args[0]));
                v8::undefined(scope).into()
            }
        };
        let (callback_kind, log_label, callback_name) = match self.kind {
            WindowWebIdlCallbackTaskKind::Timer => {
                ("callback", "host callback threw", "timer callback")
            }
            WindowWebIdlCallbackTaskKind::AnimationFrame { .. } => (
                "requestAnimationFrame callback",
                "host callback threw",
                "requestAnimationFrame callback",
            ),
            WindowWebIdlCallbackTaskKind::Idle { .. } => (
                "requestIdleCallback callback",
                "host callback threw",
                "requestIdleCallback callback",
            ),
            WindowWebIdlCallbackTaskKind::Geolocation { .. } => (
                "Geolocation callback",
                "Geolocation callback threw",
                "Geolocation callback",
            ),
        };
        invoke_window_webidl_callback_function(
            scope,
            host_ptr,
            callback_kind,
            log_label,
            callback_name,
            &prepared,
            receiver,
            &arguments,
        )
    }
}
