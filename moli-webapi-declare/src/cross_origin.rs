//! Native cross-origin member declarations. Templates contain callback metadata
//! only; V8 owns each context's unique function instance and its lifetime.

use std::collections::HashMap;

use moli_v8_util::v8str;

pub type CrossOriginGetter = for<'s, 'i> fn(
    &mut v8::PinScope<'s, 'i>,
    v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>>;
pub type CrossOriginSetter = for<'s, 'i> fn(
    &mut v8::PinScope<'s, 'i>,
    v8::Local<'s, v8::Object>,
    v8::Local<'s, v8::Value>,
) -> Option<()>;

pub struct CrossOriginFunction {
    pub build: for<'s, 'i> fn(&v8::PinScope<'s, 'i>) -> v8::Local<'s, v8::FunctionTemplate>,
}

#[derive(Default)]
struct FunctionTemplates(HashMap<usize, v8::Global<v8::FunctionTemplate>>);

impl CrossOriginFunction {
    pub fn get<'s>(
        &'static self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Option<v8::Local<'s, v8::Function>> {
        // The key identifies an immutable declaration member, never a Window
        // or a realm. No callback data can retain a target or an observer.
        let key = self as *const Self as usize;
        let cached = scope
            .get_slot::<FunctionTemplates>()
            .and_then(|templates| templates.0.get(&key))
            .map(|template| v8::Local::new(scope, template));
        let template = match cached {
            Some(template) => template,
            None => {
                let template = (self.build)(scope);
                let retained = v8::Global::new(scope, template);
                if scope.get_slot::<FunctionTemplates>().is_none() {
                    scope.set_slot(FunctionTemplates::default());
                }
                scope
                    .get_slot_mut::<FunctionTemplates>()
                    .expect("template store was initialized")
                    .0
                    .insert(key, retained);
                template
            }
        };
        template.get_function(scope)
    }
}

pub enum CrossOriginProperty {
    Method {
        name: &'static str,
        function: CrossOriginFunction,
    },
    Accessor {
        name: &'static str,
        getter: Option<(CrossOriginGetter, CrossOriginFunction)>,
        setter: Option<(CrossOriginSetter, CrossOriginFunction)>,
    },
}

impl CrossOriginProperty {
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Method { name, .. } | Self::Accessor { name, .. } => name,
        }
    }

    pub fn get<'s>(
        &'static self,
        scope: &mut v8::PinScope<'s, '_>,
        receiver: v8::Local<'s, v8::Object>,
    ) -> Option<v8::Local<'s, v8::Value>> {
        match self {
            Self::Method { function, .. } => function.get(scope).map(Into::into),
            Self::Accessor {
                getter: Some((getter, _)),
                ..
            } => getter(scope, receiver),
            _ => None,
        }
    }

    pub fn set<'s>(
        &'static self,
        scope: &mut v8::PinScope<'s, '_>,
        receiver: v8::Local<'s, v8::Object>,
        value: v8::Local<'s, v8::Value>,
    ) -> Option<()> {
        match self {
            Self::Accessor {
                setter: Some((setter, _)),
                ..
            } => setter(scope, receiver, value),
            _ => None,
        }
    }

    pub const fn has_getter(&self) -> bool {
        matches!(
            self,
            Self::Method { .. }
                | Self::Accessor {
                    getter: Some(_),
                    ..
                }
        )
    }

    pub const fn has_setter(&self) -> bool {
        matches!(
            self,
            Self::Accessor {
                setter: Some(_),
                ..
            }
        )
    }

    pub fn descriptor<'s>(
        &'static self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let descriptor = v8::Object::new(scope);
        let fields: [(&str, v8::Local<v8::Value>); 2] = match self {
            Self::Method { function, .. } => [
                ("value", function.get(scope)?.into()),
                ("writable", v8::Boolean::new(scope, false).into()),
            ],
            Self::Accessor { getter, setter, .. } => {
                let get = match getter {
                    Some((_, function)) => function.get(scope)?.into(),
                    None => v8::undefined(scope).into(),
                };
                let set = match setter {
                    Some((_, function)) => function.get(scope)?.into(),
                    None => v8::undefined(scope).into(),
                };
                [("get", get), ("set", set)]
            }
        };
        for (name, value) in fields.into_iter().chain([
            ("enumerable", v8::Boolean::new(scope, false).into()),
            ("configurable", v8::Boolean::new(scope, true).into()),
        ]) {
            descriptor
                .define_own_property(
                    scope,
                    v8str(scope, name).into(),
                    value,
                    v8::PropertyAttribute::NONE,
                )
                .filter(|defined| *defined)?;
        }
        Some(descriptor)
    }
}

pub trait WebApiCrossOriginDeclaration {
    fn cross_origin_properties() -> &'static [CrossOriginProperty];

    fn cross_origin_property(name: &str) -> Option<&'static CrossOriginProperty> {
        Self::cross_origin_properties()
            .iter()
            .find(|property| property.name() == name)
    }
}
