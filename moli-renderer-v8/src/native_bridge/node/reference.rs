use super::*;
use crate::native_bridge::document::AttrReference;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::native_bridge) struct TreeNodeReference {
    pub runtime_ptr: *mut JsContextHost,
    pub handle: DomHandle,
}

impl TreeNodeReference {
    pub fn from_object<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
    ) -> Option<Self> {
        let object = moli_webapi_declare::web_api_object_target(scope, object)?;
        let context = object.get_creation_context(scope)?;
        let scope = &mut v8::ContextScope::new(scope, context);
        let (runtime_ptr, handle) =
            node_runtime_and_handle_from_object_or_detached(scope, object).ok()?;
        Some(Self {
            runtime_ptr,
            handle,
        })
    }

    fn position(self, other: Self) -> Option<u16> {
        if self.runtime_ptr != other.runtime_ptr {
            return None;
        }
        let dom_host = unsafe { &*self.runtime_ptr }.dom_host();
        Some(
            dom_host
                .node(self.handle)?
                .compare_document_position(dom_host.dom(), other.handle),
        )
    }
}

/// Native Node identity includes both tree nodes and attributes. Branding
/// determines whether an object implements Node; this layer resolves its data.
pub(in crate::native_bridge) enum NativeNodeReference<'s> {
    Tree {
        object: v8::Local<'s, v8::Object>,
        node: TreeNodeReference,
    },
    Attr(AttrReference<'s>),
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
        if let Some(attr) = AttrReference::from_object(scope, object) {
            return Some(Self::Attr(attr));
        }
        Some(Self::Tree {
            object,
            node: TreeNodeReference::from_object(scope, object)?,
        })
    }

    pub fn receiver(
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
        method: &str,
    ) -> Option<Self> {
        let node = Self::from_object(scope, object);
        if node.is_none() {
            throw_incompatible_method_receiver(scope, "Node", method);
        }
        node
    }

    pub fn tree_node(&self) -> Option<TreeNodeReference> {
        match self {
            Self::Tree { node, .. } => Some(*node),
            Self::Attr(_) => None,
        }
    }

    pub fn object(&self) -> v8::Local<'s, v8::Object> {
        match self {
            Self::Tree { object, .. } => *object,
            Self::Attr(attr) => attr.object,
        }
    }

    pub fn is_same(&self, scope: &mut v8::PinScope<'s, '_>, other: &Self) -> bool {
        if self.object().strict_equals(other.object().into()) {
            return true;
        }
        match (self, other) {
            (Self::Tree { node: left, .. }, Self::Tree { node: right, .. }) => left == right,
            (Self::Attr(left), Self::Attr(right)) => {
                left.namespace == right.namespace
                    && left.local_name == right.local_name
                    && left
                        .owner(scope)
                        .is_some_and(|owner| Some(owner) == right.owner(scope))
            }
            _ => false,
        }
    }

    pub fn contains(&self, scope: &mut v8::PinScope<'s, '_>, other: &Self) -> bool {
        if self.is_same(scope, other) {
            return true;
        }
        let (Self::Tree { node: left, .. }, Self::Tree { node: right, .. }) = (self, other) else {
            return false;
        };
        if left.runtime_ptr != right.runtime_ptr {
            return false;
        }
        let dom_host = unsafe { &*left.runtime_ptr }.dom_host();
        dom_host
            .node(left.handle)
            .is_some_and(|node| node.contains(dom_host.dom(), right.handle))
    }

    pub fn is_equal(&self, scope: &mut v8::PinScope<'s, '_>, other: &Self) -> bool {
        match (self, other) {
            (Self::Attr(left), Self::Attr(right)) => {
                left.namespace == right.namespace
                    && left.local_name == right.local_name
                    && left.value(scope) == right.value(scope)
            }
            (Self::Tree { node: left, .. }, Self::Tree { node: right, .. }) => {
                let left_dom = unsafe { &*left.runtime_ptr }.dom_host().dom();
                let right_dom = unsafe { &*right.runtime_ptr }.dom_host().dom();
                match (left_dom.node(left.handle), right_dom.node(right.handle)) {
                    (Some(left), Some(right)) => left.is_equal_node_in(left_dom, right, right_dom),
                    _ => false,
                }
            }
            _ => false,
        }
    }

    pub fn tree_or_owner(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<TreeNodeReference> {
        match self {
            Self::Tree { node, .. } => Some(*node),
            Self::Attr(attr) => attr.owner(scope),
        }
    }

    pub fn compare_position(&self, scope: &mut v8::PinScope<'s, '_>, other: &Self) -> u16 {
        if self.is_same(scope, other) {
            return 0;
        }
        let Some(left) = self.tree_or_owner(scope) else {
            return self.disconnected_position(scope, other);
        };
        let Some(right) = other.tree_or_owner(scope) else {
            return self.disconnected_position(scope, other);
        };
        if left == right {
            return match (self, other) {
                (Self::Attr(a), Self::Attr(b)) => {
                    let element = unsafe { &*left.runtime_ptr }
                        .dom_host()
                        .node(left.handle)
                        .and_then(Node::as_element);
                    let position = |attr: &AttrReference<'_>| {
                        element.and_then(|element| {
                            element.attributes().iter().position(|candidate| {
                                candidate.namespace()
                                    == attr.namespace.as_deref().unwrap_or_default()
                                    && candidate.local_name() == attr.local_name
                            })
                        })
                    };
                    match (position(a), position(b)) {
                        (Some(a), Some(b)) => {
                            Node::DOCUMENT_POSITION_IMPLEMENTATION_SPECIFIC
                                | if a < b {
                                    Node::DOCUMENT_POSITION_FOLLOWING
                                } else {
                                    Node::DOCUMENT_POSITION_PRECEDING
                                }
                        }
                        _ => self.disconnected_position(scope, other),
                    }
                }
                (Self::Attr(_), _) => {
                    Node::DOCUMENT_POSITION_CONTAINS | Node::DOCUMENT_POSITION_PRECEDING
                }
                (_, Self::Attr(_)) => {
                    Node::DOCUMENT_POSITION_CONTAINED_BY | Node::DOCUMENT_POSITION_FOLLOWING
                }
                _ => 0,
            };
        }
        let Some(mut position) = left.position(right) else {
            return self.disconnected_position(scope, other);
        };
        // Attr borrows its owner's position, but is not the owner's ancestor
        // or descendant. Only the DOM algorithm's corresponding tree side may
        // contribute a containment bit.
        if matches!(self, Self::Attr(_)) {
            position &= !Node::DOCUMENT_POSITION_CONTAINED_BY;
        }
        if matches!(other, Self::Attr(_)) {
            position &= !Node::DOCUMENT_POSITION_CONTAINS;
        }
        position
    }

    fn disconnected_position(&self, scope: &mut v8::PinScope<'s, '_>, other: &Self) -> u16 {
        // Multiple realm wrappers for a tree node must agree on its order.
        // Ownerless attributes have no arena identity, so only those need an
        // ordinal on their canonical private object.
        let follows = match (self.tree_or_owner(scope), other.tree_or_owner(scope)) {
            (Some(left), Some(right)) if left != right => {
                (right.runtime_ptr as usize, right.handle.index())
                    > (left.runtime_ptr as usize, left.handle.index())
            }
            (Some(_), None) => true,
            (None, Some(_)) => false,
            _ => {
                disconnected_order_id(scope, other.object())
                    > disconnected_order_id(scope, self.object())
            }
        };
        Node::DOCUMENT_POSITION_DISCONNECTED
            | Node::DOCUMENT_POSITION_IMPLEMENTATION_SPECIFIC
            | if follows {
                Node::DOCUMENT_POSITION_FOLLOWING
            } else {
                Node::DOCUMENT_POSITION_PRECEDING
            }
    }
}

// A per-object ordinal gives disconnected nodes a stable total order without
// depending on V8 identity hashes, which can collide. The slot is private and
// carries no ownership, so it neither invokes author code nor retains nodes.
fn disconnected_order_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    const SLOT: &str = "__moliDisconnectedNodeOrder";
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    if let Some(value) = get_private_value(scope, object, SLOT)
        .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
    {
        return value.u64_value().0;
    }
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let value = v8::BigInt::new_from_u64(scope, id);
    crate::util::set_private_value(scope, object, SLOT, value.into());
    id
}
