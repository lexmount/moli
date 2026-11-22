use std::collections::{BTreeMap, BTreeSet};

use crate::{web_api_interfaces, webidl};
use webidl::{Context, WebIdlConverter, WebIdlError};

use super::config::{
    Configuration, ElementAttributes, Filter, HTML, Name, Names, can_replace_with_children,
};

fn is_dictionary(value: v8::Local<'_, v8::Value>) -> bool {
    value.is_object() || value.is_null_or_undefined()
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "SanitizerName")]
struct NameDictionary {
    #[webidl(required, converter = "dom_string16")]
    name: Vec<u16>,
    #[webidl(with = namespace_member)]
    namespace: Option<Option<Vec<u16>>>,
}

fn namespace_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    member: &'static str,
) -> Result<Option<Option<Vec<u16>>>, WebIdlError> {
    let context = Context::member("SanitizerName", member);
    let Some(value) = webidl::property_result(scope, object, member, context)? else {
        return Ok(None);
    };
    if value.is_undefined() {
        return Ok(None);
    }
    if value.is_null() {
        return Ok(Some(None));
    }
    let namespace = webidl::convert::<webidl::DomString16>(scope, value, context)?.0;
    Ok(Some((!namespace.is_empty()).then_some(namespace)))
}

impl NameDictionary {
    fn canonicalize(self, default_namespace: Option<&str>) -> Name {
        Name {
            local: self.name,
            namespace: self.namespace.unwrap_or_else(|| {
                default_namespace.map(|namespace| namespace.encode_utf16().collect())
            }),
        }
    }
}

fn convert_name<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    context: Context,
    namespace: Option<&str>,
) -> Result<Name, WebIdlError> {
    if is_dictionary(value) {
        let name = webidl::convert::<webidl::Dictionary<NameDictionary>>(scope, value, context)?.0;
        return Ok(name.canonicalize(namespace));
    }
    let local = webidl::convert::<webidl::DomString16>(scope, value, context)?.0;
    Ok(Name {
        local,
        namespace: namespace.map(|value| value.encode_utf16().collect()),
    })
}

impl<'s> WebIdlConverter<'s> for Name {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _: &(),
    ) -> Result<Self, WebIdlError> {
        convert_name(scope, value, context, None)
    }
}

#[derive(Clone)]
pub(super) struct ElementName(pub(super) Name);

impl<'s> WebIdlConverter<'s> for ElementName {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _: &(),
    ) -> Result<Self, WebIdlError> {
        convert_name(scope, value, context, Some(HTML)).map(Self)
    }
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "SanitizerElementNamespaceWithAttributes")]
struct ElementDictionary {
    #[webidl(inherit)]
    name: NameDictionary,
    #[webidl(sequence, converter = "raw")]
    attributes: Option<Vec<Name>>,
    #[webidl(sequence, converter = "raw")]
    remove_attributes: Option<Vec<Name>>,
}

#[derive(Clone)]
pub(super) struct ElementInput {
    name: Name,
    attributes: Option<Vec<Name>>,
    remove_attributes: Option<Vec<Name>>,
}

impl<'s> WebIdlConverter<'s> for ElementInput {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _: &(),
    ) -> Result<Self, WebIdlError> {
        if is_dictionary(value) {
            let dictionary =
                webidl::convert::<webidl::Dictionary<ElementDictionary>>(scope, value, context)?.0;
            return Ok(Self {
                name: dictionary.name.canonicalize(Some(HTML)),
                attributes: dictionary.attributes,
                remove_attributes: dictionary.remove_attributes,
            });
        }
        Ok(Self {
            name: convert_name(scope, value, context, Some(HTML))?,
            attributes: None,
            remove_attributes: None,
        })
    }
}

impl ElementInput {
    fn parts(self, unique: bool) -> Result<(Name, ElementAttributes), &'static str> {
        let collect = |values: Option<Vec<Name>>| -> Result<Option<Names>, _> {
            if unique {
                unique_list(values)
            } else {
                Ok(values.map(|values| values.into_iter().collect()))
            }
        };
        let mut attributes = ElementAttributes {
            allow: collect(self.attributes)?,
            remove: collect(self.remove_attributes)?,
        };
        if attributes.allow.is_none() && attributes.remove.is_none() {
            attributes.remove = Some(Names::new());
        }
        Ok((self.name, attributes))
    }

    pub(super) fn into_element(self) -> (Name, ElementAttributes) {
        self.parts(false)
            .expect("modifier accepts duplicate attributes")
    }
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "SanitizerProcessingInstruction")]
struct ProcessingInstructionDictionary {
    #[webidl(required, converter = "dom_string16")]
    target: Vec<u16>,
}

#[derive(Clone)]
pub(super) struct ProcessingInstruction(pub(super) Vec<u16>);

impl<'s> WebIdlConverter<'s> for ProcessingInstruction {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _: &(),
    ) -> Result<Self, WebIdlError> {
        if is_dictionary(value) {
            return webidl::convert::<webidl::Dictionary<ProcessingInstructionDictionary>>(
                scope, value, context,
            )
            .map(|dictionary| Self(dictionary.0.target));
        }
        webidl::convert::<webidl::DomString16>(scope, value, context).map(|value| Self(value.0))
    }
}

#[derive(Clone, Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "SanitizerConfig")]
pub(super) struct ConfigurationInput {
    #[webidl(sequence, converter = "raw")]
    attributes: Option<Vec<Name>>,
    comments: Option<bool>,
    data_attributes: Option<bool>,
    #[webidl(sequence, converter = "raw")]
    elements: Option<Vec<ElementInput>>,
    #[webidl(name = "javascriptURLs")]
    javascript_urls: Option<bool>,
    #[webidl(sequence, converter = "raw")]
    processing_instructions: Option<Vec<ProcessingInstruction>>,
    #[webidl(sequence, converter = "raw")]
    remove_attributes: Option<Vec<Name>>,
    #[webidl(sequence, converter = "raw")]
    remove_elements: Option<Vec<ElementName>>,
    #[webidl(sequence, converter = "raw")]
    remove_processing_instructions: Option<Vec<ProcessingInstruction>>,
    #[webidl(sequence, converter = "raw")]
    replace_with_children_elements: Option<Vec<ElementName>>,
}

#[derive(Clone)]
pub(super) enum ConstructorInput {
    Default,
    Dictionary(Box<ConfigurationInput>),
}

impl<'s> WebIdlConverter<'s> for ConstructorInput {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _: &(),
    ) -> Result<Self, WebIdlError> {
        if is_dictionary(value) {
            return webidl::convert::<webidl::Dictionary<ConfigurationInput>>(
                scope, value, context,
            )
            .map(|dictionary| Self::Dictionary(Box::new(dictionary.0)));
        }
        let preset = webidl::convert::<webidl::DomString>(scope, value, context)?;
        if preset.0 == "default" {
            Ok(Self::Default)
        } else {
            Err(WebIdlError::custom_message(
                "The SanitizerPresets value must be 'default'.",
            ))
        }
    }
}

#[derive(Clone)]
pub(super) enum SanitizerInput<'s> {
    New(ConstructorInput),
    Existing(v8::Local<'s, v8::Object>),
}

impl<'s> WebIdlConverter<'s> for SanitizerInput<'s> {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _: &(),
    ) -> Result<Self, WebIdlError> {
        if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
            && web_api_interfaces::Sanitizer::is_instance(scope, object)
        {
            return Ok(Self::Existing(object));
        }
        webidl::convert::<ConstructorInput>(scope, value, context).map(Self::New)
    }
}

fn unique_list<T: Ord>(values: Option<Vec<T>>) -> Result<Option<BTreeSet<T>>, &'static str> {
    values
        .map(|values| {
            let length = values.len();
            let items = values.into_iter().collect::<BTreeSet<_>>();
            if items.len() == length {
                Ok(items)
            } else {
                Err("A sanitizer configuration list contains a duplicate name.")
            }
        })
        .transpose()
}

fn attribute_and_pi_filter<T: Ord>(
    allow: Option<Vec<T>>,
    remove: Option<Vec<T>>,
    default_allowlist: bool,
) -> Result<Filter<BTreeSet<T>>, &'static str> {
    match (unique_list(allow)?, unique_list(remove)?) {
        (Some(_), Some(_)) => Err("An allow list and a remove list cannot both be present."),
        (Some(list), None) => Ok(Filter::Allow(list)),
        (None, Some(list)) => Ok(Filter::Remove(list)),
        (None, None) if default_allowlist => Ok(Filter::Allow(BTreeSet::new())),
        (None, None) => Ok(Filter::Remove(BTreeSet::new())),
    }
}

impl ConfigurationInput {
    pub(super) fn is_empty(&self) -> bool {
        self.attributes.is_none()
            && self.comments.is_none()
            && self.data_attributes.is_none()
            && self.elements.is_none()
            && self.javascript_urls.is_none()
            && self.processing_instructions.is_none()
            && self.remove_attributes.is_none()
            && self.remove_elements.is_none()
            && self.remove_processing_instructions.is_none()
            && self.replace_with_children_elements.is_none()
    }

    pub(super) fn canonicalize(self, permissive: bool) -> Result<Configuration, &'static str> {
        let replacements = unique_list(
            self.replace_with_children_elements
                .map(|list| list.into_iter().map(|name| name.0).collect()),
        )?;
        if replacements
            .as_ref()
            .is_some_and(|names| names.iter().any(|name| !can_replace_with_children(name)))
        {
            return Err("An HTML, SVG, or MathML root cannot be replaced with its children.");
        }
        let attributes = attribute_and_pi_filter(self.attributes, self.remove_attributes, false)?;
        let data_attributes = match &attributes {
            Filter::Allow(global) => {
                let allow = self.data_attributes.unwrap_or(permissive);
                if allow && global.iter().any(Name::is_data_attribute) {
                    return Err("The global attribute list repeats a custom data attribute.");
                }
                Some(allow)
            }
            Filter::Remove(_) if self.data_attributes.is_some() => {
                return Err("dataAttributes requires an attribute allow list.");
            }
            Filter::Remove(_) => None,
        };
        let elements = match (self.elements, self.remove_elements) {
            (Some(_), Some(_)) => return Err("Both elements and removeElements are present."),
            (Some(elements), None) => {
                let mut unique = BTreeMap::new();
                for element in elements {
                    let (name, local) = element.parts(true)?;
                    if replacements
                        .as_ref()
                        .is_some_and(|names| names.contains(&name))
                    {
                        return Err(
                            "An allowed element also appears in replaceWithChildrenElements.",
                        );
                    }
                    validate_local_attributes(&attributes, &local, data_attributes == Some(true))?;
                    if unique.insert(name, local).is_some() {
                        return Err("The elements list contains a duplicate name.");
                    }
                }
                Filter::Allow(unique)
            }
            (None, remove) => {
                let removed =
                    unique_list(remove.map(|list| list.into_iter().map(|name| name.0).collect()))?
                        .unwrap_or_default();
                if replacements
                    .as_ref()
                    .is_some_and(|names| !names.is_disjoint(&removed))
                {
                    return Err("A removed element also appears in replaceWithChildrenElements.");
                }
                Filter::Remove(removed)
            }
        };
        let processing_instructions = attribute_and_pi_filter(
            self.processing_instructions
                .map(|list| list.into_iter().map(|pi| pi.0).collect()),
            self.remove_processing_instructions
                .map(|list| list.into_iter().map(|pi| pi.0).collect()),
            !permissive,
        )?;
        Ok(Configuration {
            elements,
            attributes,
            processing_instructions,
            replace_with_children: replacements,
            comments: self.comments.unwrap_or(permissive),
            data_attributes,
            javascript_urls: self.javascript_urls.unwrap_or(permissive),
        })
    }
}

fn validate_local_attributes(
    global: &Filter<Names>,
    local: &ElementAttributes,
    data: bool,
) -> Result<(), &'static str> {
    let invalid = match global {
        Filter::Allow(global) => {
            local.allow.as_ref().is_some_and(|list| {
                !list.is_disjoint(global) || data && list.iter().any(Name::is_data_attribute)
            }) || local
                .remove
                .as_ref()
                .is_some_and(|list| !list.is_subset(global))
        }
        Filter::Remove(global) => {
            local.allow.is_some() && local.remove.is_some()
                || local
                    .allow
                    .as_ref()
                    .is_some_and(|list| !list.is_disjoint(global))
                || local
                    .remove
                    .as_ref()
                    .is_some_and(|list| !list.is_disjoint(global))
        }
    };
    if invalid {
        Err("An element's attribute lists conflict with the global attribute configuration.")
    } else {
        Ok(())
    }
}
