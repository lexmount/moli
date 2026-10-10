//! Constrainable-pattern conversion and task commits for the existing native
//! sources. Capture-device selection and measured settings belong to a producer
//! backend; this module never manufactures either.

use crate::{
    native_bridge::JsContextHost,
    util::{context_host_ptr_from_global_bridge, get_private_value, v8str},
    webidl,
};

use model::{ConstraintSet, Constraints, Snapshot};

mod model;
mod state;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaStreamTrack.applyConstraints")]
struct ApplyArgs {
    #[webidl(dictionary, default = Constraints::default())]
    constraints: Constraints,
}

pub(super) use state::initialize;

pub(super) fn copy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    original: v8::Local<'s, v8::Object>,
    clone: v8::Local<'s, v8::Object>,
) {
    let constraints = state::get(scope, original).borrow().clone();
    *state::get(scope, clone).borrow_mut() = constraints;
}

pub(super) fn get_constraints<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let track = super::target(scope, args.this());
    let constraints = state::get(scope, track).borrow().clone();
    rv.set(constraints.snapshot(scope));
}

pub(in crate::context_bootstrap) fn supported_constraints<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    let result = v8::Object::new(scope);
    // This is the UA's recognized constraint vocabulary, independent of the
    // capabilities of a particular source or the presence of capture devices.
    // Read-only settings from extensions are not advertised as constraints.
    for (name, _) in ConstraintSet::default().entries() {
        result.create_data_property(
            scope,
            v8str(scope, name).into(),
            v8::Boolean::new(scope, true).into(),
        );
    }
    result
}

pub(super) fn apply_constraints<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<ApplyArgs>(scope, &args) else {
        return;
    };
    let track = super::target(scope, args.this());
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    rv.set(resolver.get_promise(scope).into());
    // WebIDL conversion precedes the method's ended-track short circuit.
    if get_private_value(scope, track, super::READY_STATE)
        .expect("track state")
        .strict_equals(v8str(scope, "ended").into())
    {
        resolver.resolve(scope, v8::undefined(scope).into());
        return;
    }
    let kind = super::track_kind(scope, track);
    let remote = super::source_value(scope, track, super::REMOTE).is_true();
    let failed_constraint = initial_source_failure(&parsed.constraints, &kind, remote);
    let task = MediaTrackConstraintsTask {
        track: v8::Global::new(scope, track),
        resolver: v8::Global::new(scope, resolver),
        constraints: parsed.constraints,
        failed_constraint,
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    // SAFETY: native track creation and this binding use the isolate's host.
    let host: &mut JsContextHost = unsafe { &mut *host_ptr };
    host.queue_media_track_constraints_task(scope, track, task);
}

fn initial_source_failure(
    constraints: &Constraints,
    kind: &str,
    remote: bool,
) -> Option<&'static str> {
    if remote {
        // WebRTC defines four constrainable properties for remote video and
        // requires applyConstraints to reject them, even ideal constraints.
        // Remote audio has no constrainable properties defined by WebRTC.
        if kind != "video" {
            return None;
        }
        return std::iter::once(&constraints.base)
            .chain(constraints.advanced.iter().flatten())
            .flat_map(ConstraintSet::entries)
            .find_map(|(name, value)| {
                (value.is_some()
                    && matches!(name, "width" | "height" | "frameRate" | "aspectRatio"))
                .then_some(name)
            });
    }
    // The inert test source has exactly one configuration, with no settings.
    // Required applicable basic constraints cannot match it; optional basic
    // constraints and unsatisfiable advanced sets do not remove that candidate.
    // A future producer must supply real candidates before using this branch.
    constraints
        .base
        .entries()
        .into_iter()
        .find_map(|(name, value)| {
            (applicable(name, kind) && value.is_some_and(|value| value.required())).then_some(name)
        })
}

fn applicable(name: &str, kind: &str) -> bool {
    match name {
        "deviceId" | "groupId" => true,
        "autoGainControl"
        | "channelCount"
        | "echoCancellation"
        | "latency"
        | "noiseSuppression"
        | "restrictOwnAudio"
        | "sampleRate"
        | "sampleSize"
        | "suppressLocalAudioPlayback"
        | "voiceIsolation" => kind == "audio",
        _ => kind == "video",
    }
}

/// Owned converted request. The exact Window/Document queue determines
/// currentness and invocation order; Promise settlement occurs only at dispatch.
pub(crate) struct MediaTrackConstraintsTask {
    track: v8::Global<v8::Object>,
    resolver: v8::Global<v8::PromiseResolver>,
    constraints: Constraints,
    failed_constraint: Option<&'static str>,
}

impl MediaTrackConstraintsTask {
    pub(crate) fn invoke(self, scope: &mut v8::PinScope<'_, '_>) -> bool {
        let track = v8::Local::new(scope, self.track);
        let resolver = v8::Local::new(scope, self.resolver);
        let Some(context) = resolver.get_creation_context(scope) else {
            return false;
        };
        let scope = &mut v8::ContextScope::new(scope, context);
        if let Some(failed) = self.failed_constraint {
            let Some(error) = crate::context_bootstrap::constructors::build_overconstrained_error(
                scope,
                failed,
                "The source cannot satisfy the requested constraint.",
            ) else {
                return false;
            };
            resolver.reject(scope, error.into()) == Some(true)
        } else {
            *state::get(scope, track).borrow_mut() = self.constraints;
            resolver.resolve(scope, v8::undefined(scope).into()) == Some(true)
        }
    }
}
