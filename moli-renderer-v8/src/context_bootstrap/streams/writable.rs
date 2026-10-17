use super::constructors::{
    TEXT_DECODER_STREAM_ENCODING_SLOT, TEXT_DECODER_STREAM_FATAL_SLOT,
    TEXT_DECODER_STREAM_IGNORE_BOM_SLOT, TEXT_ENCODER_STREAM_ENCODING_SLOT,
};
use super::*;
use crate::context_bootstrap::stream_adapter::{stream_slot_object, writable_stream_abort_promise};
use crate::util::{get_private_value, v8str};
use crate::web_api_interfaces;
use crate::webidl;
use moli_streams::writable::{AcquireWriterPlan, UnlockedCloseEntryPlan, UnlockedEntryPlan};
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "WritableStream.abort")]
struct WritableStreamAbortArgs<'s> {
    #[webidl(converter = "raw")]
    reason: Option<v8::Local<'s, v8::Value>>,
}

pub(in crate::context_bootstrap) fn writable_stream_get_writer_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let stream = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("validated WritableStream receiver");
    if writable_stream_snapshot(scope, stream).plan_acquire_writer()
        == AcquireWriterPlan::RejectLocked
    {
        throw_type_error(scope, "WritableStream is locked");
        return;
    }
    let Some(writer) = new_writable_stream_writer_object(scope, stream) else {
        rv.set_undefined();
        return;
    };
    rv.set(writer.into());
}

pub(in crate::context_bootstrap) fn writable_stream_abort_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let stream = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("validated WritableStream receiver");
    let Some(parsed) = webidl::parse_args::<WritableStreamAbortArgs<'s>>(scope, &args) else {
        return;
    };
    if writable_stream_snapshot(scope, stream).plan_unlocked_abort_entry()
        == UnlockedEntryPlan::RejectLocked
    {
        let message = v8::String::new(scope, "WritableStream is locked").unwrap();
        let error = v8::Exception::type_error(scope, message);
        if let Some(promise) = rejected_promise_value(scope, error) {
            rv.set(promise);
        }
        return;
    }
    let reason = parsed.reason.unwrap_or_else(|| v8::undefined(scope).into());
    if let Some(promise) = writable_stream_abort_promise(scope, stream, reason) {
        rv.set(promise);
    } else {
        rv.set_undefined();
    }
}

pub(in crate::context_bootstrap) fn writable_stream_close_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let stream = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("validated WritableStream receiver");
    match writable_stream_snapshot(scope, stream).plan_unlocked_close_entry() {
        UnlockedCloseEntryPlan::RejectLocked => {
            let message = v8::String::new(scope, "WritableStream is locked").unwrap();
            let error = v8::Exception::type_error(scope, message);
            if let Some(promise) = rejected_promise_value(scope, error) {
                rv.set(promise);
            }
            return;
        }
        UnlockedCloseEntryPlan::RejectErrored => {
            let message = v8::String::new(scope, "WritableStream is errored").unwrap();
            let error = v8::Exception::type_error(scope, message);
            if let Some(promise) = rejected_promise_value(scope, error) {
                rv.set(promise);
            }
            return;
        }
        UnlockedCloseEntryPlan::Continue => {}
    }
    if let Some(close_result) = writable_stream_close_internal(scope, stream) {
        rv.set(close_result);
        return;
    }
    set_resolved_promise(scope, &mut rv, v8::undefined(scope).into());
}

pub(in crate::context_bootstrap) fn writable_stream_locked_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let stream = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("validated WritableStream receiver");
    let locked = writable_stream_snapshot(scope, stream).locked();
    rv.set(v8::Boolean::new(scope, locked).into());
}

fn transform_stream_readable_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(readable) = stream_slot_object(scope, args.this(), TRANSFORM_STREAM_READABLE_SLOT)
    else {
        throw_type_error(
            scope,
            "TransformStream.readable called on incompatible receiver",
        );
        return;
    };
    rv.set(readable.into());
}

fn transform_stream_writable_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(writable) = stream_slot_object(scope, args.this(), TRANSFORM_STREAM_WRITABLE_SLOT)
    else {
        throw_type_error(
            scope,
            "TransformStream.writable called on incompatible receiver",
        );
        return;
    };
    rv.set(writable.into());
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::TransformStream, enumerable)]
struct TransformStreamPrototypeAttributesDeclaration {
    #[webapi(accessor_property, getter = transform_stream_readable_getter_callback)]
    readable: (),

    #[webapi(accessor_property, getter = transform_stream_writable_getter_callback)]
    writable: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::TextEncoderStream, enumerable, receiver)]
struct TextEncoderStreamPrototypeAttributesDeclaration {
    #[webapi(accessor_property, getter = text_stream_attribute_getter, data = v8str(scope, TEXT_ENCODER_STREAM_ENCODING_SLOT))]
    encoding: (),

    #[webapi(accessor_property, getter = text_stream_attribute_getter, data = v8str(scope, TRANSFORM_STREAM_READABLE_SLOT))]
    readable: (),

    #[webapi(accessor_property, getter = text_stream_attribute_getter, data = v8str(scope, TRANSFORM_STREAM_WRITABLE_SLOT))]
    writable: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::TextDecoderStream, enumerable, receiver)]
struct TextDecoderStreamPrototypeAttributesDeclaration {
    #[webapi(accessor_property, getter = text_stream_attribute_getter, data = v8str(scope, TEXT_DECODER_STREAM_ENCODING_SLOT))]
    encoding: (),

    #[webapi(accessor_property, getter = text_stream_attribute_getter, data = v8str(scope, TEXT_DECODER_STREAM_FATAL_SLOT))]
    fatal: (),

    #[webapi(accessor_property = "ignoreBOM", getter = text_stream_attribute_getter, data = v8str(scope, TEXT_DECODER_STREAM_IGNORE_BOM_SLOT))]
    ignore_bom: (),

    #[webapi(accessor_property, getter = text_stream_attribute_getter, data = v8str(scope, TRANSFORM_STREAM_READABLE_SLOT))]
    readable: (),

    #[webapi(accessor_property, getter = text_stream_attribute_getter, data = v8str(scope, TRANSFORM_STREAM_WRITABLE_SLOT))]
    writable: (),
}

fn text_stream_attribute_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let target = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("validated text stream receiver");
    let slot = args.data().to_rust_string_lossy(scope);
    rv.set(get_private_value(scope, target, &slot).expect("initialized text stream attribute"));
}

pub(super) fn install_transform_stream_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
    interface_name: &str,
) {
    match interface_name {
        "TransformStream" => {
            TransformStreamPrototypeAttributesDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "TextEncoderStream" => {
            TextEncoderStreamPrototypeAttributesDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "TextDecoderStream" => {
            TextDecoderStreamPrototypeAttributesDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        _ => {}
    }
}
