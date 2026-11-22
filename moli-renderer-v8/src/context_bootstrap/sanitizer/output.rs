use crate::util::{v8_string_from_utf16_units, v8str};

use super::config::{Configuration, ElementAttributes, Filter, Name, Names, Targets};

fn property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
    value: impl Into<v8::Local<'s, v8::Value>>,
) -> Option<()> {
    let key = v8str(scope, name);
    object
        .create_data_property(scope, key.into(), value.into())?
        .then_some(())
}

fn name_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &Name,
) -> Option<v8::Local<'s, v8::Object>> {
    let object = v8::Object::new(scope);
    let local = v8_string_from_utf16_units(scope, &name.local)?;
    let namespace: v8::Local<'s, v8::Value> = match &name.namespace {
        Some(units) => v8_string_from_utf16_units(scope, units)?.into(),
        None => v8::null(scope).into(),
    };
    property(scope, object, "name", local)?;
    property(scope, object, "namespace", namespace)?;
    Some(object)
}

fn element_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &Name,
    attributes: &ElementAttributes,
) -> Option<v8::Local<'s, v8::Object>> {
    let object = name_object(scope, name)?;
    if let Some(names) = &attributes.allow {
        let array = name_array(scope, names)?;
        property(scope, object, "attributes", array)?;
    }
    if let Some(names) = &attributes.remove {
        let array = name_array(scope, names)?;
        property(scope, object, "removeAttributes", array)?;
    }
    Some(object)
}

fn name_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    names: &Names,
) -> Option<v8::Local<'s, v8::Array>> {
    let values = names
        .iter()
        .map(|name| name_object(scope, name).map(Into::into))
        .collect::<Option<Vec<_>>>()?;
    Some(v8::Array::new_with_elements(scope, &values))
}

fn target_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    targets: &Targets,
) -> Option<v8::Local<'s, v8::Array>> {
    let mut values = Vec::with_capacity(targets.len());
    for target in targets {
        let object = v8::Object::new(scope);
        let target = v8_string_from_utf16_units(scope, target)?;
        property(scope, object, "target", target)?;
        values.push(object.into());
    }
    Some(v8::Array::new_with_elements(scope, &values))
}

impl Configuration {
    pub(super) fn to_dictionary<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let object = v8::Object::new(scope);
        if let Filter::Allow(names) = &self.attributes {
            let array = name_array(scope, names)?;
            property(scope, object, "attributes", array)?;
        }
        let comments = v8::Boolean::new(scope, self.comments);
        property(scope, object, "comments", comments)?;
        if let Some(allow) = self.data_attributes {
            let data = v8::Boolean::new(scope, allow);
            property(scope, object, "dataAttributes", data)?;
        }
        if let Filter::Allow(elements) = &self.elements {
            let values = elements
                .iter()
                .map(|(name, attributes)| element_object(scope, name, attributes).map(Into::into))
                .collect::<Option<Vec<_>>>()?;
            let array = v8::Array::new_with_elements(scope, &values);
            property(scope, object, "elements", array)?;
        }
        let javascript_urls = v8::Boolean::new(scope, self.javascript_urls);
        property(scope, object, "javascriptURLs", javascript_urls)?;
        if let Filter::Allow(targets) = &self.processing_instructions {
            let array = target_array(scope, targets)?;
            property(scope, object, "processingInstructions", array)?;
        }
        if let Filter::Remove(names) = &self.attributes {
            let array = name_array(scope, names)?;
            property(scope, object, "removeAttributes", array)?;
        }
        if let Filter::Remove(names) = &self.elements {
            let array = name_array(scope, names)?;
            property(scope, object, "removeElements", array)?;
        }
        if let Filter::Remove(targets) = &self.processing_instructions {
            let array = target_array(scope, targets)?;
            property(scope, object, "removeProcessingInstructions", array)?;
        }
        if let Some(names) = &self.replace_with_children {
            let array = name_array(scope, names)?;
            property(scope, object, "replaceWithChildrenElements", array)?;
        }
        Some(object)
    }
}
