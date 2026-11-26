//! Parser construction keeps its creation realm witness in the V8 heap.
//! Parsing and native Document creation remain embedding operations.
use crate::interfaces;
use moli_v8_util::throw_type_error;
use moli_webapi_declare::WebApiObject;
use moli_webidl as webidl;

pub const DOM_PARSER_CONSTRUCTION_REALM_SLOT: &str = "__moliDomParserConstructionRealm";

#[derive(WebApiObject)]
#[webapi(interface = interfaces::DOMParser)]
struct DomParserObjectDeclaration<'s> {
    #[webapi(slot = DOM_PARSER_CONSTRUCTION_REALM_SLOT)]
    creation_realm: v8::Local<'s, v8::Object>,
}

pub fn dom_parser_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "DOMParser constructor must be called with new");
        return;
    }
    initialize_dom_parser_object(scope, args.this());
    rv.set(args.this().into());
}

pub fn initialize_dom_parser_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parser: v8::Local<'s, v8::Object>,
) {
    // The constructor's Window is independent of newTarget.prototype. Keep
    // a V8-traced creation-Realm witness, not a native persistent Context root
    // or a WindowProxy that can be rebound to another Window on navigation.
    let realm = moli_v8_util::new_null_prototype_object(scope);
    DomParserObjectDeclaration::new(realm)
        .initialize(scope, parser)
        .expect("DOMParser state should initialize");
}

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "SupportedType")]
pub enum DomParserSupportedType {
    #[webidl(token = "text/html")]
    Html,
    #[webidl(token = "text/xml")]
    TextXml,
    #[webidl(token = "application/xml")]
    ApplicationXml,
    #[webidl(token = "application/xhtml+xml")]
    ApplicationXhtmlXml,
    #[webidl(token = "image/svg+xml")]
    ImageSvgXml,
}

impl DomParserSupportedType {
    pub fn as_mime(self) -> &'static str {
        match self {
            Self::Html => "text/html",
            Self::TextXml => "text/xml",
            Self::ApplicationXml => "application/xml",
            Self::ApplicationXhtmlXml => "application/xhtml+xml",
            Self::ImageSvgXml => "image/svg+xml",
        }
    }
}
