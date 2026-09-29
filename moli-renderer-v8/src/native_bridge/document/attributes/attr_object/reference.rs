use super::*;
use crate::dom::native::DomStringValue;
use crate::native_bridge::node::reference::TreeNodeReference;
use crate::native_bridge::wrapped_handle_value_for_receiver;
use crate::util::{v8_string_from_utf16_units, v8_string_to_u16_string};

/// Read an Attr's internal identity and value without invoking author methods
/// on its owner. Attached values live in the native Element; detached values
/// live in the existing private Attr state.
pub(in crate::native_bridge) struct AttrReference<'s> {
    pub object: v8::Local<'s, v8::Object>,
    pub(super) state: v8::Local<'s, v8::Object>,
    pub namespace: Option<String>,
    pub local_name: String,
}

impl<'s> AttrReference<'s> {
    pub fn metadata(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        name: &'static str,
    ) -> Option<v8::Local<'s, v8::Value>> {
        self.state.get(scope, v8str(scope, name).into())
    }

    pub fn name(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<String> {
        object_string_property(scope, self.state, "name")
    }

    pub fn prefix(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<String> {
        nullable_state_string(scope, self.state, "prefix")
    }

    pub fn child_nodes(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Option<v8::Local<'s, v8::Object>> {
        const SLOT: &str = "__moliAttrChildNodes";
        if let Some(list) = get_private_object(scope, self.object, SLOT) {
            return Some(list);
        }
        let context = self.object.get_creation_context(scope)?;
        let scope = &mut v8::ContextScope::new(scope, context);
        let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
        // Attr never acquires children. A cached empty native NodeList remains
        // live and satisfies SameObject across calls from different realms.
        let list = collections::build_collection_wrapper(
            scope,
            runtime_ptr,
            &[],
            CollectionKind::NodeList,
        );
        set_private_value(scope, self.object, SLOT, list.into());
        Some(list)
    }

    pub fn from_object(
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
    ) -> Option<Self> {
        let object = moli_webapi_declare::web_api_object_target(scope, object)?;
        let state = attr_state_object(scope, object)?;
        Some(Self {
            object,
            state,
            namespace: nullable_state_string(scope, state, "namespaceURI"),
            local_name: object_string_property(scope, state, "localName")?,
        })
    }

    pub fn owner_element(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Option<v8::Local<'s, v8::Object>> {
        object_property_as_object(scope, self.state, "ownerElement")
    }

    pub fn owner(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<TreeNodeReference> {
        let owner = self.owner_element(scope)?;
        TreeNodeReference::from_object(scope, owner)
    }

    pub fn owner_document(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Option<v8::Local<'s, v8::Object>> {
        if let Some(owner) = self.owner_element(scope)
            && let Some(document) = native_owner_document(scope, owner)
        {
            return Some(document);
        }
        object_property_as_object(scope, self.state, "ownerDocument")
    }

    pub fn value(&self, scope: &mut v8::PinScope<'s, '_>) -> DomStringValue {
        if let Some(owner) = self.owner(scope)
            && let Some(element) = unsafe { &*owner.runtime_ptr }
                .dom_host()
                .node(owner.handle)
                .and_then(|node| node.as_element())
        {
            let namespace = self.namespace.as_deref().unwrap_or_default();
            if let Some(units) = element.attribute_ns_utf16_units(namespace, &self.local_name) {
                return DomStringValue::from_utf16(units);
            }
            if let Some(value) = element.attribute_ns(namespace, &self.local_name) {
                return value.into();
            }
        }
        self.state
            .get(scope, v8str(scope, "value").into())
            .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())
            .map(|value| {
                DomStringValue::from_utf16(v8_string_to_u16_string(scope, value).as_slice())
            })
            .unwrap_or_default()
    }

    pub fn clone_node(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let name = object_string_property(scope, self.state, "name")?;
        let prefix = nullable_state_string(scope, self.state, "prefix");
        let value = self.value(scope);
        let document = self.owner_document(scope);
        let context = document
            .and_then(|document| document.get_creation_context(scope))
            .or_else(|| self.object.get_creation_context(scope))?;
        let scope = &mut v8::ContextScope::new(scope, context);
        new_attr_object(
            scope,
            &name,
            value,
            None,
            document,
            self.namespace.as_deref(),
            prefix.as_deref(),
            &self.local_name,
        )
    }

    pub fn detach(&self, scope: &mut v8::PinScope<'s, '_>) {
        let value = self.value(scope);
        let document = self.owner_document(scope);
        let value = v8_string_from_utf16_units(scope, &value.utf16_units())
            .unwrap_or_else(|| v8::String::empty(scope));
        let _ = self
            .state
            .set(scope, v8str(scope, "value").into(), value.into());
        if let Some(document) = document {
            let _ = self
                .state
                .set(scope, v8str(scope, "ownerDocument").into(), document.into());
        }
        let _ = self.state.set(
            scope,
            v8str(scope, "ownerElement").into(),
            v8::null(scope).into(),
        );
    }
}

pub(in crate::native_bridge::document) fn native_owner_document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let node = TreeNodeReference::from_object(scope, owner)?;
    let document = unsafe { &*node.runtime_ptr }
        .dom_host()
        .owner_document_handle(node.handle)?;
    let value = wrapped_handle_value_for_receiver(scope, node.runtime_ptr, owner, document)?;
    v8::Local::<v8::Object>::try_from(value).ok()
}

fn nullable_state_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
    property: &'static str,
) -> Option<String> {
    let value = state.get(scope, v8str(scope, property).into())?;
    v8::Local::<v8::String>::try_from(value)
        .ok()
        .map(|value| value.to_rust_string_lossy(scope))
        .filter(|value| !value.is_empty())
}
