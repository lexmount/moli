use super::fetch::{ResolvedWorkerFetchInput, start_worker_resource_fetch};
use super::*;
use crate::worker::timer_callback::WorkerTimerCallback;

const WORKER_FONTS_SLOT: &str = "__moliWorkerFonts";

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::WorkerGlobalScope, enumerable, receiver)]
struct WorkerFontSourceDeclaration {
    #[webapi(accessor_property, getter = worker_fonts_getter)]
    fonts: (),
}

pub(super) fn install_worker_font_source_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    WorkerFontSourceDeclaration::initialize_prototype_template(scope, prototype);
}

fn worker_fonts_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(fonts) = get_private_value(scope, args.this(), WORKER_FONTS_SLOT) {
        rv.set(fonts);
    } else if let Some(fonts) = crate::context_bootstrap::new_font_face_set(scope) {
        set_private_value(scope, args.this(), WORKER_FONTS_SLOT, fonts.into());
        rv.set(fonts.into());
    }
}

/// Font Loading has its own task source, independent of author timer IDs.
pub(crate) fn queue_worker_font_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    callback: v8::Local<'s, v8::Function>,
) -> bool {
    let Some(state) = get_worker_state(scope) else {
        return false;
    };
    let mut state = state.borrow_mut();
    if !state.closed && !state.termination_requested.load(Ordering::Acquire) {
        state
            .font_tasks
            .push_back(WorkerTimerCallback::browser_function(scope, callback));
        let _ = state
            .worker_wake_tx
            .send(super::super::handle::WorkerMessage::RunFontLoadingTask);
    }
    true
}

pub(crate) fn start_worker_font_face_fetch<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
    url: Url,
) -> Result<(), String> {
    let promise =
        start_worker_resource_fetch(scope, ResolvedWorkerFetchInput::font(url), Some(face))
            .ok_or("Failed to create worker font request")?;
    let callback = v8::Function::builder(worker_font_fetch_failed)
        .data(face.into())
        .build(scope)
        .ok_or("Failed to create worker font completion callback")?;
    promise
        .catch(scope, callback)
        .ok_or("Failed to observe worker font request")?;
    Ok(())
}

fn worker_font_fetch_failed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok(face) = v8::Local::<v8::Object>::try_from(args.data()) else {
        return;
    };
    crate::context_bootstrap::complete_font_face_resource(scope, face, None);
}
