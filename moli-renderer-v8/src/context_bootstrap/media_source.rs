//! Host-owned MediaSource identity for object URL lifetimes.
//! The interface shape and MIME support remain shared value bindings.
mod state;

pub(crate) use state::{MediaSourceObject, media_source_object};
pub(super) use moli_webapi::media_source::install_media_source_template_bindings;

pub(super) fn media_source_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    state::initialize(scope, args.this());
    rv.set(args.this().into());
}
