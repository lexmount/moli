use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct TreeNodeReference {
    pub runtime_ptr: *mut JsContextHost,
    pub handle: DomHandle,
}

impl TreeNodeReference {
    fn from_object<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
    ) -> Option<Self> {
        let context = object.get_creation_context(scope)?;
        let scope = &mut v8::ContextScope::new(scope, context);
        let (runtime_ptr, handle) =
            node_runtime_and_handle_from_object_or_detached(scope, object).ok()?;
        Some(Self {
            runtime_ptr,
            handle,
        })
    }
}

/// Resolves branded Node identity without evaluating author-visible properties.
/// Attr uses its canonical backing object; tree nodes also carry arena identity.
pub(crate) enum NativeNodeReference<'s> {
    Tree {
        object: v8::Local<'s, v8::Object>,
        node: TreeNodeReference,
    },
    Attr(v8::Local<'s, v8::Object>),
}

impl<'s> NativeNodeReference<'s> {
    pub fn from_object(
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
    ) -> Option<Self> {
        if !web_api_interfaces::Node::is_instance(scope, object) {
            return None;
        }
        let object = moli_webapi_declare::web_api_object_target(scope, object)?;
        if web_api_interfaces::Attr::is_instance(scope, object) {
            return Some(Self::Attr(object));
        }
        Some(Self::Tree {
            object,
            node: TreeNodeReference::from_object(scope, object)?,
        })
    }

    pub fn tree_node(&self) -> Option<TreeNodeReference> {
        match self {
            Self::Tree { node, .. } => Some(*node),
            Self::Attr(_) => None,
        }
    }

    fn object(&self) -> v8::Local<'s, v8::Object> {
        match self {
            Self::Tree { object, .. } | Self::Attr(object) => *object,
        }
    }

    pub fn is_same(&self, _scope: &mut v8::PinScope<'s, '_>, other: &Self) -> bool {
        if self.object().strict_equals(other.object().into()) {
            return true;
        }
        matches!((self, other), (Self::Tree { node: left, .. }, Self::Tree { node: right, .. }) if left == right)
    }
}
