use super::*;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct DetachedBridgePrototypeDeclaration<'scope> {
    #[webapi(prototype)]
    prototype: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", scope_lifetime = 'scope)]
struct DetachedObjectWithPrototypeDeclaration<'scope, 'tag> {
    #[webapi(prototype)]
    prototype: v8::Local<'scope, v8::Object>,

    #[webapi(to_string_tag)]
    to_string_tag: Option<&'tag str>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct DetachedObjectTagDeclaration {
    #[webapi(to_string_tag)]
    to_string_tag: String,
}

pub(in crate::native_bridge::document) use crate::util::global_constructor_prototype;

pub(in crate::native_bridge::document) fn bridge_prototype_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let bridge = global_bridge_object(scope)?;
    object_property_as_object(scope, bridge, name)
}

pub(in crate::native_bridge::document) fn create_bridge_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    prototype: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    DetachedBridgePrototypeDeclaration::new(prototype)
        .bind(scope)
        .expect("detached bridge prototype declaration should bind")
}

pub(in crate::native_bridge::document) fn ensure_bridge_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bridge: v8::Local<'s, v8::Object>,
    slot: &str,
    prototype: v8::Local<'s, v8::Object>,
) {
    if object_property_as_object(scope, bridge, slot).is_some() {
        return;
    }
    let object = create_bridge_prototype(scope, prototype);
    let _ = bridge.define_own_property(
        scope,
        v8_string(scope, slot)
            .map(Into::<v8::Local<'_, v8::Name>>::into)
            .unwrap_or_else(|| v8::String::empty(scope).into()),
        object.into(),
        v8::PropertyAttribute::DONT_ENUM,
    );
}

pub(in crate::native_bridge::document) fn ensure_detached_bridge_prototypes(
    scope: &mut v8::PinScope<'_, '_>,
) {
    let Some(bridge) = global_bridge_object(scope) else {
        return;
    };
    let Some(node_prototype) = global_constructor_prototype(scope, "Node") else {
        return;
    };
    if let Some(prototype) = global_constructor_prototype(scope, "DocumentType") {
        ensure_bridge_prototype(scope, bridge, "__detachedDocumentTypePrototype", prototype);
    }
    if let Some(prototype) = global_constructor_prototype(scope, "DocumentFragment") {
        ensure_bridge_prototype(
            scope,
            bridge,
            "__detachedDocumentFragmentPrototype",
            prototype,
        );
    }
    if let Some(prototype) = global_constructor_prototype(scope, "ShadowRoot") {
        ensure_bridge_prototype(scope, bridge, "__detachedShadowRootPrototype", prototype);
    }
    if let Some(prototype) = global_constructor_prototype(scope, "Text") {
        ensure_bridge_prototype(scope, bridge, "__detachedTextPrototype", prototype);
    }
    if let Some(prototype) = global_constructor_prototype(scope, "Comment") {
        ensure_bridge_prototype(scope, bridge, "__detachedCommentPrototype", prototype);
    }
    let processing_instruction_prototype =
        global_constructor_prototype(scope, "ProcessingInstruction").unwrap_or(node_prototype);
    ensure_bridge_prototype(
        scope,
        bridge,
        "__detachedProcessingInstructionPrototype",
        processing_instruction_prototype,
    );
    if let Some(prototype) = global_constructor_prototype(scope, "CDATASection") {
        ensure_bridge_prototype(scope, bridge, "__detachedCDATASectionPrototype", prototype);
    }
    if let Some(prototype) = global_constructor_prototype(scope, "Element") {
        ensure_bridge_prototype(scope, bridge, "__detachedElementPrototype", prototype);
    }
    if let Some(prototype) = global_constructor_prototype(scope, "HTMLDocument") {
        ensure_bridge_prototype(scope, bridge, "__detachedHTMLDocumentPrototype", prototype);
    }
    if let Some(prototype) = global_constructor_prototype(scope, "XMLDocument") {
        ensure_bridge_prototype(scope, bridge, "__detachedXMLDocumentPrototype", prototype);
    }
    if let Some(prototype) = global_constructor_prototype(scope, "Document") {
        ensure_bridge_prototype(scope, bridge, "__detachedDocumentPrototype", prototype);
    }
}

pub(in crate::native_bridge::document) fn set_string_tag<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    tag: &str,
) {
    let _ = DetachedObjectTagDeclaration::new(tag.to_owned()).initialize(scope, object);
}

pub(in crate::native_bridge::document) fn new_detached_object_with_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    prototype_name: &str,
    to_string_tag: Option<&'static str>,
) -> Option<v8::Local<'s, v8::Object>> {
    let prototype = bridge_prototype_object(scope, prototype_name)?;
    let object = DetachedObjectWithPrototypeDeclaration::new(prototype, to_string_tag)
        .bind(scope)
        .ok()?;
    if let Some(interface) = to_string_tag {
        web_api_interfaces::initialize(scope, object, interface).ok()?;
    }
    Some(object)
}
