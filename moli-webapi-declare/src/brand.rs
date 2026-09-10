use std::{cell::RefCell, collections::HashMap, rc::Rc};

use moli_v8_util::v8str;

use crate::{BindError, v8};

/// The primary Web IDL interface of a native platform object.
///
/// Names come from Rust declarations, never from JavaScript constructors or
/// prototypes. The numeric representation in V8 is isolate-local and must not
/// be persisted or used as a structured-clone wire tag.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct WebApiType {
    name: &'static str,
}

impl WebApiType {
    pub const fn name(self) -> &'static str {
        self.name
    }
}

#[derive(Clone)]
struct TypeEntry {
    name: &'static str,
    parent: Option<usize>,
    declared: bool,
}

struct TypeRegistry {
    key: v8::Global<v8::Private>,
    proxy_key: v8::Global<v8::Private>,
    entries: Vec<TypeEntry>,
    names: HashMap<&'static str, usize>,
}

impl TypeRegistry {
    fn intern(&mut self, name: &'static str) -> usize {
        if let Some(id) = self.names.get(name) {
            return *id;
        }
        let id = self.entries.len();
        self.entries.push(TypeEntry {
            name,
            parent: None,
            declared: false,
        });
        self.names.insert(name, id);
        id
    }

    fn implements(&self, mut id: usize, expected: usize) -> bool {
        // Registration rejects cycles. The bound also makes incomplete native
        // metadata harmless while a batch of interfaces is being registered.
        for _ in 0..self.entries.len() {
            if id == expected {
                return true;
            }
            let Some(parent) = self.entries.get(id).and_then(|entry| entry.parent) else {
                return false;
            };
            id = parent;
        }
        false
    }
}

fn registry(scope: &mut v8::PinScope<'_, '_, ()>) -> Rc<RefCell<TypeRegistry>> {
    if let Some(registry) = scope.get_slot::<Rc<RefCell<TypeRegistry>>>() {
        return registry.clone();
    }
    let name = v8str(scope, "__moliWebApiType");
    let key = v8::Private::new(scope, Some(name));
    let proxy_key = v8::Private::new(scope, None);
    let registry = Rc::new(RefCell::new(TypeRegistry {
        proxy_key: v8::Global::new(scope, proxy_key),
        key: v8::Global::new(scope, key),
        entries: Vec::new(),
        names: HashMap::new(),
    }));
    scope.as_mut().set_slot(registry.clone());
    registry
}

/// Registers interface inheritance from the same metadata used to declare
/// interfaces. This does not materialize constructors or depend on exposure in
/// the current realm. Repeated declarations must agree.
pub fn register_web_api_interfaces<C>(
    scope: &mut v8::PinScope<'_, '_, C>,
    interfaces: impl IntoIterator<Item = (&'static str, Option<&'static str>)>,
) -> Result<(), BindError> {
    v8::scope!(let scope, scope.as_mut());
    let registry = registry(scope);
    let mut registry = registry.borrow_mut();
    // Object declarations can register their parent on every construction.
    // Repeated metadata needs no copies or cycle validation of the whole graph.
    let mut changes = interfaces.into_iter().filter(|(name, parent)| {
        !registry.names.get(name).is_some_and(|id| {
            let entry = &registry.entries[*id];
            entry.declared && entry.parent.map(|id| registry.entries[id].name) == *parent
        })
    });
    let Some(first) = changes.next() else {
        return Ok(());
    };
    // Validate the whole batch before publishing any inheritance changes.
    let mut entries = registry.entries.clone();
    let mut names = registry.names.clone();
    for (name, parent) in std::iter::once(first).chain(changes) {
        if name == "Object" || parent == Some("Object") {
            return Err(BindError::new("Object is not a Web IDL interface"));
        }
        let mut intern = |name| {
            *names.entry(name).or_insert_with(|| {
                let id = entries.len();
                entries.push(TypeEntry {
                    name,
                    parent: None,
                    declared: false,
                });
                id
            })
        };
        let id = intern(name);
        let parent = parent.map(intern);
        let entry = &mut entries[id];
        if entry.declared && entry.parent != parent {
            return Err(BindError::new(format!("conflicting parents for `{name}`")));
        }
        entry.parent = parent;
        entry.declared = true;
    }
    for entry in &entries {
        let mut parent = entry.parent;
        for depth in 0..=entries.len() {
            let Some(id) = parent else { break };
            if depth == entries.len() {
                return Err(BindError::new(format!(
                    "interface inheritance cycle at `{}`",
                    entry.name
                )));
            }
            parent = entries[id].parent;
        }
    }
    registry.entries = entries;
    registry.names = names;
    Ok(())
}

fn object_type_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    registry: &Rc<RefCell<TypeRegistry>>,
) -> Option<usize> {
    let object = native_identity_target(scope, object, registry)?;
    let key = v8::Local::new(scope, &registry.borrow().key);
    let value = object.get_private(scope, key)?;
    let id = v8::Local::<v8::Uint32>::try_from(value).ok()?.value() as usize;
    (id < registry.borrow().entries.len()).then_some(id)
}

/// Reads the object's own native identity, or the target identity of an
/// explicitly registered native Proxy. Prototype inheritance and JavaScript
/// properties (including symbols) cannot supply identity or invoke author code.
pub fn web_api_object_type<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<WebApiType> {
    let registry = scope.get_slot::<Rc<RefCell<TypeRegistry>>>()?.clone();
    let id = object_type_id(scope, object, &registry)?;
    Some(WebApiType {
        name: registry.borrow().entries[id].name,
    })
}

pub fn implements_interface<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    expected: &str,
) -> bool {
    let Some(registry) = scope.get_slot::<Rc<RefCell<TypeRegistry>>>().cloned() else {
        return false;
    };
    let Some(id) = object_type_id(scope, object, &registry) else {
        return false;
    };
    let registry = registry.borrow();
    registry
        .names
        .get(expected)
        .is_some_and(|expected| registry.implements(id, *expected))
}

/// Attaches identity to an initialized native instance. Generated declarations
/// call this automatically; native wrapper factories use the same entry point.
/// Base initialization preserves a derived type, and derived initialization can
/// refine a base type. Unrelated declarations cannot rebrand an existing object.
pub fn initialize_web_api_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    interface: &'static str,
) -> Result<(), BindError> {
    if interface == "Object" {
        return Err(BindError::new(
            "platform identity requires a native interface instance",
        ));
    }
    let registry = registry(scope);
    let object = native_identity_target(scope, object, &registry)
        .ok_or_else(|| BindError::new("author proxies cannot carry platform identity"))?;
    let id = registry.borrow_mut().intern(interface);
    if let Some(existing) = object_type_id(scope, object, &registry) {
        let registry = registry.borrow();
        if registry.implements(existing, id) {
            return Ok(());
        }
        if !registry.implements(id, existing) {
            return Err(BindError::new(format!(
                "cannot change `{}` identity to `{interface}`",
                registry.entries[existing].name
            )));
        }
    }
    let id = u32::try_from(id).map_err(|_| BindError::new("too many Web API types"))?;
    let key = v8::Local::new(scope, &registry.borrow().key);
    let value = v8::Integer::new_from_unsigned(scope, id);
    if object.set_private(scope, key, value.into()) != Some(true) {
        return Err(BindError::new(format!(
            "failed to initialize `{interface}` identity"
        )));
    }
    Ok(())
}

// A small number of native DOM wrappers use a JS Proxy to implement named
// properties. Only the factory can register one: the private handler value
// must be the very same proxy, so author proxies never inherit this permission.
fn native_identity_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    registry: &Rc<RefCell<TypeRegistry>>,
) -> Option<v8::Local<'s, v8::Object>> {
    let Ok(proxy) = v8::Local::<v8::Proxy>::try_from(object) else {
        return Some(object);
    };
    if proxy.is_revoked() {
        return None;
    }
    let handler = v8::Local::<v8::Object>::try_from(proxy.get_handler(scope)).ok()?;
    if handler.is_proxy() {
        return None;
    }
    let key = v8::Local::new(scope, &registry.borrow().proxy_key);
    if !handler.get_private(scope, key)?.strict_equals(proxy.into()) {
        return None;
    }
    let target = v8::Local::<v8::Object>::try_from(proxy.get_target(scope)).ok()?;
    (!target.is_proxy()).then_some(target)
}

/// Registers a Proxy created by a native wrapper factory. The target must
/// already have native identity and the handler must be a private ordinary
/// object. This never invokes traps or accepts a Proxy wrapping another Proxy.
pub fn register_web_api_proxy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    proxy: v8::Local<'s, v8::Proxy>,
) -> Result<(), BindError> {
    let registry = registry(scope);
    let target = v8::Local::<v8::Object>::try_from(proxy.get_target(scope))
        .map_err(|_| BindError::new("native proxy requires an object target"))?;
    if target.is_proxy() || object_type_id(scope, target, &registry).is_none() {
        return Err(BindError::new("native proxy requires a branded target"));
    }
    let handler = v8::Local::<v8::Object>::try_from(proxy.get_handler(scope))
        .map_err(|_| BindError::new("native proxy requires an object handler"))?;
    if handler.is_proxy() {
        return Err(BindError::new("native proxy requires an ordinary handler"));
    }
    let key = v8::Local::new(scope, &registry.borrow().proxy_key);
    if handler.set_private(scope, key, proxy.into()) != Some(true) {
        return Err(BindError::new("failed to register native proxy"));
    }
    Ok(())
}
