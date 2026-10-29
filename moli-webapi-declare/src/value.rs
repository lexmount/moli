use crate::{__private, v8};

/// Converts Rust declaration field values into V8 values during binding.
///
/// The derive uses this trait for declared properties, hidden properties,
/// private slots, runtime prototypes, and runtime toStringTag values. Returning
/// `None` makes the generated binding fail with a `BindError` instead of
/// silently installing an incomplete object.
pub trait WebApiValue<'s> {
    fn to_v8_value(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>>;
}

/// Converts Rust declaration field values into V8 values during template binding.
///
/// Function-template initialization runs without an entered context, so template
/// constants cannot use the runtime-object `WebApiValue` scope shape.
pub trait WebApiTemplateValue<'s> {
    fn to_v8_template_value(
        &self,
        scope: &mut v8::PinScope<'s, '_, ()>,
    ) -> Option<v8::Local<'s, v8::Value>>;
}

macro_rules! impl_number_value {
    ($($ty:ty),* $(,)?) => {
        $(
            impl<'s> WebApiValue<'s> for $ty {
                fn to_v8_value(
                    &self,
                    scope: &mut v8::PinScope<'s, '_>,
                ) -> Option<v8::Local<'s, v8::Value>> {
                    Some(v8::Number::new(scope, *self as f64).into())
                }
            }

            impl<'s> WebApiTemplateValue<'s> for $ty {
                fn to_v8_template_value(
                    &self,
                    scope: &mut v8::PinScope<'s, '_, ()>,
                ) -> Option<v8::Local<'s, v8::Value>> {
                    Some(v8::Number::new(scope, *self as f64).into())
                }
            }
        )*
    };
}

impl_number_value!(f64, f32, usize, u64, u32, u16, u8, isize, i64, i32, i16, i8);

/// None inside a value is null; dictionary derives separately omit absent members.
impl<'s, T: WebApiValue<'s>> WebApiValue<'s> for Option<T> {
    fn to_v8_value(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        match self {
            Some(value) => value.to_v8_value(scope),
            None => Some(v8::null(scope).into()),
        }
    }
}

impl<'s> WebApiValue<'s> for bool {
    fn to_v8_value(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        Some(v8::Boolean::new(scope, *self).into())
    }
}

impl<'s> WebApiTemplateValue<'s> for bool {
    fn to_v8_template_value(
        &self,
        scope: &mut v8::PinScope<'s, '_, ()>,
    ) -> Option<v8::Local<'s, v8::Value>> {
        Some(v8::Boolean::new(scope, *self).into())
    }
}

impl<'s> WebApiValue<'s> for str {
    fn to_v8_value(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        __private::v8_string(scope, self).map(Into::into)
    }
}

impl<'s> WebApiTemplateValue<'s> for str {
    fn to_v8_template_value(
        &self,
        scope: &mut v8::PinScope<'s, '_, ()>,
    ) -> Option<v8::Local<'s, v8::Value>> {
        __private::v8_string(scope, self).map(Into::into)
    }
}

impl<'s> WebApiValue<'s> for String {
    fn to_v8_value(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        self.as_str().to_v8_value(scope)
    }
}

impl<'s> WebApiTemplateValue<'s> for String {
    fn to_v8_template_value(
        &self,
        scope: &mut v8::PinScope<'s, '_, ()>,
    ) -> Option<v8::Local<'s, v8::Value>> {
        self.as_str().to_v8_template_value(scope)
    }
}

impl<'s> WebApiValue<'s> for &str {
    fn to_v8_value(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        (**self).to_v8_value(scope)
    }
}

impl<'s> WebApiTemplateValue<'s> for &str {
    fn to_v8_template_value(
        &self,
        scope: &mut v8::PinScope<'s, '_, ()>,
    ) -> Option<v8::Local<'s, v8::Value>> {
        (**self).to_v8_template_value(scope)
    }
}

impl<'s, T> WebApiValue<'s> for v8::Local<'s, T>
where
    v8::Local<'s, v8::Value>: From<v8::Local<'s, T>>,
{
    fn to_v8_value(&self, _scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        Some((*self).into())
    }
}

impl<'s, T> WebApiTemplateValue<'s> for v8::Local<'s, T>
where
    v8::Local<'s, v8::Value>: From<v8::Local<'s, T>>,
{
    fn to_v8_template_value(
        &self,
        _scope: &mut v8::PinScope<'s, '_, ()>,
    ) -> Option<v8::Local<'s, v8::Value>> {
        Some((*self).into())
    }
}

impl<'s, T> WebApiValue<'s> for [T]
where
    T: WebApiValue<'s>,
{
    fn to_v8_value(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        let length = i32::try_from(self.len()).ok()?;
        let array = v8::Array::new(scope, length);
        for (index, value) in self.iter().enumerate() {
            let value = value.to_v8_value(scope)?;
            define_array_data_property(scope, array, index as u32, value)?;
        }
        Some(array.into())
    }
}

impl<'s, T, const N: usize> WebApiValue<'s> for [T; N]
where
    T: WebApiValue<'s>,
{
    fn to_v8_value(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        self.as_slice().to_v8_value(scope)
    }
}

impl<'s, T> WebApiValue<'s> for Vec<T>
where
    T: WebApiValue<'s>,
{
    fn to_v8_value(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        self.as_slice().to_v8_value(scope)
    }
}

impl<'s, T> WebApiValue<'s> for &[T]
where
    T: WebApiValue<'s>,
{
    fn to_v8_value(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        (*self).to_v8_value(scope)
    }
}

/// Defines an indexed own data property on a V8 array.
///
/// This uses ECMA-262 `CreateDataProperty` semantics instead of ordinary
/// assignment, so declaration arrays do not trigger inherited indexed setters
/// installed on `Array.prototype`.
pub fn define_array_data_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    array: v8::Local<'s, v8::Array>,
    index: u32,
    value: v8::Local<'s, v8::Value>,
) -> Option<()> {
    let key = __private::v8_string(scope, &index.to_string())?;
    array
        .create_data_property(scope, key.into(), value)?
        .then_some(())
}

macro_rules! impl_tuple_value {
    ($length:expr; $($index:tt => $name:ident),+ $(,)?) => {
        impl<'s, $($name),+> WebApiValue<'s> for ($($name,)+)
        where
            $($name: WebApiValue<'s>,)+
        {
            fn to_v8_value(
                &self,
                scope: &mut v8::PinScope<'s, '_>,
            ) -> Option<v8::Local<'s, v8::Value>> {
                let array = v8::Array::new(scope, $length);
                $(
                    let value = self.$index.to_v8_value(scope)?;
                    define_array_data_property(scope, array, $index, value)?;
                )+
                Some(array.into())
            }
        }
    };
}

impl_tuple_value!(2; 0 => A, 1 => B);
impl_tuple_value!(3; 0 => A, 1 => B, 2 => C);
impl_tuple_value!(4; 0 => A, 1 => B, 2 => C, 3 => D);
