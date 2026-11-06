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
    let element = parsed.media_element;
    if get_private_value(scope, element, SOURCE).is_some() {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "The media element is already connected to an audio source node.",
        );
        return;
    }
    let node = MediaElementSourceObjectDeclaration::new(element)
        .bind(scope)
        .expect("MediaElementAudioSourceNode declaration should bind");
    graph::initialize_node(scope, node, args.this());
    // A source has no input port. It is driven by its media element, not by
    // AudioScheduledSourceNode.start() or the synthetic offline oscillator.
    graph::initialize_source(scope, node);
    set_private_value(scope, element, SOURCE, node.into());
    rv.set(node.into());
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
