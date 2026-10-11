//! Static sensor permission requests use the calling function's Window and the
//! device motion and orientation task source. Event payload construction does
//! not confer permission or create a sensor backend.

use moli_webapi_declare::WebApiFunctionTemplate;

use crate::{
    context_bootstrap::new_dom_exception_value,
    native_bridge::JsContextHost,
    permissions_policy::SensorPermission,
    util::{context_host_ptr_from_global_bridge, v8str},
    web_api_interfaces, webidl,
};

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::DeviceOrientationEvent, enumerable)]
struct OrientationConstructor {
    #[webapi(static_method, length = 0, returns_promise, callback = request_orientation)]
    request_permission: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::DeviceMotionEvent, enumerable)]
struct MotionConstructor {
    #[webapi(static_method, length = 0, returns_promise, callback = request_motion)]
    request_permission: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "DeviceOrientationEvent.requestPermission")]
struct OrientationArgs {
    #[webidl(default = false)]
    absolute: bool,
}

pub(super) fn install_orientation<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    OrientationConstructor::initialize_template(scope, template);
}

pub(super) fn install_motion<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    MotionConstructor::initialize_template(scope, template);
}

fn request_orientation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<OrientationArgs>(scope, &args) else {
        return;
    };
    request(scope, rv, parsed.absolute);
}

fn request_motion<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    request(scope, rv, false);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PermissionState {
    Granted,
    Denied,
    Prompt,
}

impl PermissionState {
    fn from_label(label: &str) -> Self {
        match label {
            "granted" => Self::Granted,
            "denied" => Self::Denied,
            _ => Self::Prompt,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PermissionResult {
    Granted,
    Denied,
    ActivationRequired,
}

fn permission_result(states: &[PermissionState], transient_activation: bool) -> PermissionResult {
    // The activation scan precedes requesting any permission: a denied
    // permission must not conceal another permission still in prompt state.
    if !transient_activation && states.contains(&PermissionState::Prompt) {
        return PermissionResult::ActivationRequired;
    }
    if states
        .iter()
        .all(|state| *state == PermissionState::Granted)
    {
        PermissionResult::Granted
    } else {
        // There is no interactive permission prompt backend. Unanswered
        // prompts cannot manufacture a grant or return the state "prompt".
        PermissionResult::Denied
    }
}

fn request(
    scope: &mut v8::PinScope<'_, '_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    absolute: bool,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    rv.set(resolver.get_promise(scope).into());
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    // SAFETY: the current Window's bridge owns this isolate's host. Static
    // methods ignore this; the entered function realm determines ownership.
    let host: &mut JsContextHost = unsafe { &mut *host_ptr };
    let Some(identity) = host.current_runtime_window_execution_context_identity(scope) else {
        return;
    };
    let Some(storage) = host.storage_context_for_window_execution_context_identity(identity) else {
        return;
    };
    let serialized = storage.storage_key().serialized_storage_key();
    let Some(key) = moli_storage_key::deserialize_serialized_storage_key(&serialized) else {
        return;
    };
    let embedding = host.main_document_origin();
    let policy = host.document_permissions_policy_for_owner(identity.dispatch_scope());
    let states = SensorPermission::ALL.map(|sensor| {
        if key.origin() == "null"
            || !policy
                .as_ref()
                .is_some_and(|policy| policy.sensor_enabled(sensor))
        {
            PermissionState::Denied
        } else {
            PermissionState::from_label(host.permission_state_for_origins(
                sensor.name(),
                key.origin(),
                &embedding,
            ))
        }
    });
    let result = permission_result(
        &states[..if absolute { 3 } else { 2 }],
        host.window_has_transient_user_activation(identity.dispatch_scope()),
    );
    host.queue_device_sensor_permission_task(
        scope,
        DevicePermissionTask {
            resolver: v8::Global::new(scope, resolver),
            result,
        },
    );
}

pub(crate) struct DevicePermissionTask {
    resolver: v8::Global<v8::PromiseResolver>,
    result: PermissionResult,
}

impl DevicePermissionTask {
    pub(crate) fn invoke(self, scope: &mut v8::PinScope<'_, '_>) -> bool {
        let resolver = v8::Local::new(scope, self.resolver);
        let Some(context) = resolver.get_creation_context(scope) else {
            return false;
        };
        let scope = &mut v8::ContextScope::new(scope, context);
        let settled = match self.result {
            PermissionResult::Granted | PermissionResult::Denied => {
                let label = if self.result == PermissionResult::Granted {
                    "granted"
                } else {
                    "denied"
                };
                let value = v8str(scope, label);
                resolver.resolve(scope, value.into())
            }
            PermissionResult::ActivationRequired => {
                let error = new_dom_exception_value(
                    scope,
                    "Requesting sensor permission requires transient user activation.",
                    "NotAllowedError",
                );
                resolver.reject(scope, error)
            }
        };
        settled == Some(true)
    }
}

#[cfg(test)]
mod tests {
    use super::{PermissionResult, PermissionState::*, permission_result};

    #[test]
    fn sensor_permission_activation_scan_precedes_denial_and_never_fabricates_grants() {
        let cases: &[(&[super::PermissionState], bool, PermissionResult)] = &[
            (&[Granted, Granted], false, PermissionResult::Granted),
            (
                &[Granted, Granted, Granted],
                false,
                PermissionResult::Granted,
            ),
            (&[Denied, Granted], false, PermissionResult::Denied),
            (&[Granted, Granted, Denied], false, PermissionResult::Denied),
            (
                &[Denied, Prompt],
                false,
                PermissionResult::ActivationRequired,
            ),
            (
                &[Prompt, Denied],
                false,
                PermissionResult::ActivationRequired,
            ),
            (
                &[Granted, Granted, Prompt],
                false,
                PermissionResult::ActivationRequired,
            ),
            (&[Prompt, Granted], true, PermissionResult::Denied),
            (&[Granted, Prompt, Granted], true, PermissionResult::Denied),
        ];
        for (states, activation, expected) in cases {
            assert_eq!(permission_result(states, *activation), *expected);
        }
    }
}
