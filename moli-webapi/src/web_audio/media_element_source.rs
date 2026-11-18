//! Media-element source ownership and graph wiring for the existing partial
//! audio backend. Decoded audio transport remains with the media backend.

use super::*;

const ELEMENT: &str = "__moliAudioSourceMediaElement";
const SOURCE: &str = "__moliMediaElementAudioSource";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::MediaElementAudioSourceNode)]
struct MediaElementSourceObjectDeclaration<'scope> {
    #[webapi(slot = ELEMENT)]
    element: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MediaElementAudioSourceNode, enumerable)]
struct MediaElementSourcePrototypeDeclaration {
    #[webapi(accessor_property, getter = media_element)]
    media_element: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "AudioContext.createMediaElementSource")]
struct CreateSourceArgs<'s> {
    #[webidl(required, interface = web_api_interfaces::HTMLMediaElement)]
    media_element: v8::Local<'s, v8::Object>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "MediaElementAudioSourceOptions")]
struct SourceOptions<'s> {
    #[webidl(required, interface = web_api_interfaces::HTMLMediaElement)]
    media_element: v8::Local<'s, v8::Object>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaElementAudioSourceNode")]
struct ConstructorArgs<'s> {
    #[webidl(required, interface = web_api_interfaces::AudioContext)]
    context: v8::Local<'s, v8::Object>,
    #[webidl(required, dictionary)]
    options: SourceOptions<'s>,
}

pub fn constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "MediaElementAudioSourceNode constructor requires 'new'.",
        );
        return;
    }
    let Some(parsed) = webidl::parse_args::<ConstructorArgs>(scope, &args) else {
        return;
    };
    if initialize(
        scope,
        args.this(),
        parsed.context,
        parsed.options.media_element,
    ) {
        moli_v8_util::apply_webidl_constructor_prototype_fallback(
            scope,
            args.this(),
            args.new_target(),
            "MediaElementAudioSourceNode",
        );
        rv.set(args.this().into());
    }
}

pub fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    MediaElementSourcePrototypeDeclaration::initialize_prototype_template(
        scope,
        template.prototype_template(scope),
    );
}

pub fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<CreateSourceArgs>(scope, &args) else {
        return;
    };
    let Some(node) =
        moli_v8_util::new_intrinsic_interface_instance(scope, "MediaElementAudioSourceNode")
    else {
        return;
    };
    if initialize(scope, node, args.this(), parsed.media_element) {
        rv.set(node.into());
    }
}

fn initialize<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    context: v8::Local<'s, v8::Object>,
    element: v8::Local<'s, v8::Object>,
) -> bool {
    let initialize = scope
        .get_current_context()
        .get_slot::<AudioRuntime>()
        .expect("install Audio capabilities before creating media sources")
        .initialize_media_source;
    initialize(scope, node, context, element)
}

/// Initializes traced JS state after the host has checked native element ownership.
pub fn bind_native_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    context: v8::Local<'s, v8::Object>,
    element: v8::Local<'s, v8::Object>,
) -> bool {
    if MediaElementSourceObjectDeclaration::new(element)
        .initialize(scope, node)
        .is_err()
    {
        return false;
    }
    graph::initialize_node(scope, node, context);
    // A source has no input port. It is driven by its media element, not by
    // AudioScheduledSourceNode.start() or the synthetic offline oscillator.
    graph::initialize_source(scope, node);
    // The native element owns uniqueness across wrapper worlds and adoption.
    // Its wrapper keeps the existing traced edge back to the source node.
    set_private_value(scope, element, SOURCE, node.into());
    true
}

fn media_element<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(value) = get_private_value(scope, args.this(), ELEMENT) {
        rv.set(value);
    }
}
