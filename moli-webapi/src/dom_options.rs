//! DOM arguments whose conversion uses WebIDL values and shared brands.
use crate::interfaces as web_api_interfaces;
use moli_webidl as webidl;

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "AssignedNodesOptions")]
pub struct AssignedNodesOptions {
    #[webidl(default = false)]
    pub flatten: bool,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "GetRootNodeOptions")]
pub struct GetRootNodeOptions {
    #[webidl(default = false)]
    pub composed: bool,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "GetHTMLOptions")]
pub struct GetHtmlOptions<'scope> {
    #[webidl(name = "serializableShadowRoots", default = false)]
    pub serializable_shadow_roots: bool,
    #[webidl(name = "shadowRoots", sequence, interface = web_api_interfaces::ShadowRoot, default = Vec::new())]
    pub shadow_roots: Vec<v8::Local<'scope, v8::Object>>,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "ElementDefinitionOptions")]
pub struct ElementDefinitionOptions {
    pub extends: Option<String>,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "PointerLockOptions")]
pub struct PointerLockOptions {
    #[webidl(name = "unadjustedMovement", default = false)]
    pub unadjusted_movement: bool,
}

#[derive(webidl::WebIdlEnum)]
pub enum ShadowRootMode {
    #[webidl(token = "open")]
    Open,
    #[webidl(token = "closed")]
    Closed,
}

#[derive(webidl::WebIdlEnum)]
pub enum SlotAssignmentMode {
    #[webidl(token = "named")]
    Named,
    #[webidl(token = "manual")]
    Manual,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "ElementCreationOptions")]
pub struct CreateElementOptions<Registry>
where
    Registry: for<'scope> webidl::WebIdlConverter<'scope, Options = ()>,
{
    // An omitted nullable member and an explicitly null registry have different
    // meanings. Keep its presence while reusing native interface conversion.
    #[webidl(name = "customElementRegistry", converter = "raw")]
    pub registry_association: Option<Registry>,
    #[webidl(name = "is", converter = "dom_string16")]
    pub is_name: Option<Vec<u16>>,
}

impl<Registry> Default for CreateElementOptions<Registry>
where
    Registry: for<'scope> webidl::WebIdlConverter<'scope, Options = ()>,
{
    fn default() -> Self {
        Self {
            registry_association: None,
            is_name: None,
        }
    }
}

impl<'s, Registry> webidl::WebIdlConverter<'s> for CreateElementOptions<Registry>
where
    Registry: for<'scope> webidl::WebIdlConverter<'scope, Options = ()>,
{
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _options: &(),
    ) -> Result<Self, webidl::WebIdlError> {
        // Objects select the dictionary even if they are boxed strings or have
        // a toString method. Nullish values select the empty dictionary.
        if value.is_object() || value.is_null_or_undefined() {
            return webidl::parse_dictionary::<Self>(scope, value, context)
                .map(Option::unwrap_or_default);
        }
        // Other primitives select the legacy DOMString member, which the DOM
        // algorithm ignores. Conversion still rejects Symbols.
        webidl::convert::<webidl::DomString16>(scope, value, context)?;
        Ok(Self::default())
    }
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "ShadowRootInit")]
pub struct ShadowRootInitMembers<Registry>
where
    Registry: for<'scope> webidl::WebIdlConverter<'scope, Options = ()>,
{
    #[webidl(default = false)]
    pub clonable: bool,
    #[webidl(converter = "raw")]
    pub custom_element_registry: Option<Registry>,
    #[webidl(default = false)]
    pub delegates_focus: bool,
    #[webidl(required, converter = "enum")]
    pub mode: ShadowRootMode,
    #[webidl(nullable)]
    pub reference_target: Option<String>,
    #[webidl(default = false)]
    pub serializable: bool,
    #[webidl(default = SlotAssignmentMode::Named, converter = "enum")]
    pub slot_assignment: SlotAssignmentMode,
}
