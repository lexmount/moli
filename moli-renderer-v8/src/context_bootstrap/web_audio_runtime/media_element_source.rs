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
#[webapi(interface = web_api_interfaces::MediaElementAudioSourceNode, enumerable, receiver)]
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

pub(in crate::context_bootstrap) fn constructor<'s>(
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
        crate::util::apply_webidl_constructor_prototype_fallback(
            scope,
            args.this(),
            args.new_target(),
            "MediaElementAudioSourceNode",
        );
        rv.set(args.this().into());
    }
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    MediaElementSourcePrototypeDeclaration::initialize_prototype_template(
        scope,
        template.prototype_template(scope),
    );
}

pub(super) fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<CreateSourceArgs>(scope, &args) else {
        return;
    };
    let Ok(node) = super::super::exposed_interfaces::build_intrinsic_interface_instance(
        scope,
        "MediaElementAudioSourceNode",
    ) else {
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
    let Ok((host, handle)) =
        crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, element)
    else {
        throw_type_error(scope, "The media element has no native backing.");
        return false;
    };
    let already_connected = unsafe { &*host }
        .dom_host()
        .node(handle)
        .and_then(crate::dom::native::Node::as_element)
        .is_some_and(crate::dom::native::Element::media_audio_source_created);
    if already_connected {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "The media element is already connected to an audio source node.",
        );
        return false;
    }
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
    if let Some(element) = unsafe { &mut *host }
        .dom_host_mut()
        .node_mut(handle)
        .and_then(|node| node.data_mut().as_element_mut())
    {
        element.mark_media_audio_source_created();
    }
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
