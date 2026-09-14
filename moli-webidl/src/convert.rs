use crate::{
    Boolean, BufferSource, ByteString, ClampedUnsignedShort, Context, DomString, Double,
    EnforceRangeLong, EnforceRangeUnsignedLong, EnforceRangeUnsignedLongLong, EnumValue, Long,
    Record, Sequence, StringOptions, UnrestrictedDouble, UnsignedLong, UnsignedLongLong,
    UnsignedShort, UsvString, WebIdlArguments, WebIdlConverter, WebIdlDictionary, WebIdlEnum,
    WebIdlError, WebIdlErrorKind, dictionary_value, is_nullish, property_result,
    symbol_property_result, throw_error, throw_type_error,
};

impl<'s> WebIdlConverter<'s> for DomString {
    type Options = StringOptions;

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        options: &Self::Options,
    ) -> Result<Self, WebIdlError> {
        if value.is_null() && options.treat_null_as_empty_string {
            return Ok(Self(String::new()));
        }
        if value.is_symbol() {
            return Err(WebIdlError::new(
                context,
                WebIdlErrorKind::CannotConvert("DOMString"),
            ));
        }
        string_value_utf16(scope, value, context, "DOMString")
            .map(|value| Self(String::from_utf16_lossy(&value)))
    }
}

// String conversion first captures V8's UTF-16 result so `DOMString`,
// `USVString`, and `ByteString` can apply their different WebIDL post-processing
// rules. This keeps getter/toString side effects in one V8-facing helper while
// making the scalar-value and byte restrictions explicit in Rust.
impl<'s> WebIdlConverter<'s> for UsvString {
    type Options = StringOptions;

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        options: &Self::Options,
    ) -> Result<Self, WebIdlError> {
        if value.is_null() && options.treat_null_as_empty_string {
            return Ok(Self(String::new()));
        }
        if value.is_symbol() {
            return Err(WebIdlError::new(
                context,
                WebIdlErrorKind::CannotConvert("USVString"),
            ));
        }
        string_value_utf16(scope, value, context, "USVString")
            .map(|value| Self(scalar_value_string_from_utf16(&value)))
    }
}

impl<'s> WebIdlConverter<'s> for ByteString {
    type Options = StringOptions;

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        options: &Self::Options,
    ) -> Result<Self, WebIdlError> {
        let value = DomString::convert(scope, value, context, options)?.0;
        let mut bytes = Vec::with_capacity(value.len());
        for ch in value.chars() {
            let code_point = ch as u32;
            if code_point > 0xff {
                return Err(WebIdlError::new(
                    context,
                    WebIdlErrorKind::InvalidByteString,
                ));
            }
            bytes.push(code_point as u8);
        }
        Ok(Self(bytes))
    }
}

impl<'s> WebIdlConverter<'s> for BufferSource {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _options: &Self::Options,
    ) -> Result<Self, WebIdlError> {
        if let Ok(view) = v8::Local::<v8::ArrayBufferView>::try_from(value) {
            let mut bytes = vec![0; view.byte_length()];
            let written = view.copy_contents(&mut bytes);
            bytes.truncate(written);
            return Ok(Self(bytes));
        }

        if let Ok(buffer) = v8::Local::<v8::ArrayBuffer>::try_from(value) {
            let view =
                v8::Uint8Array::new(scope, buffer, 0, buffer.byte_length()).ok_or_else(|| {
                    WebIdlError::new(context, WebIdlErrorKind::CannotConvert("BufferSource"))
                })?;
            let mut bytes = vec![0; view.byte_length()];
            let written = view.copy_contents(&mut bytes);
            bytes.truncate(written);
            return Ok(Self(bytes));
        }

        if let Ok(buffer) = v8::Local::<v8::SharedArrayBuffer>::try_from(value) {
            let backing_store = buffer.get_backing_store();
            let mut bytes = Vec::with_capacity(backing_store.byte_length());
            for index in 0..backing_store.byte_length() {
                bytes.push(backing_store[index].get());
            }
            return Ok(Self(bytes));
        }

        Err(WebIdlError::new(
            context,
            WebIdlErrorKind::CannotConvert("BufferSource"),
        ))
    }
}

// Numeric converters follow WebIDL's default wrapping behavior unless the type
// name carries an explicit extended attribute such as `[EnforceRange]` or
// `[Clamp]`. Call sites that need browser rejection for out-of-range values
// should choose an EnforceRange wrapper instead of checking after conversion.
impl<'s> WebIdlConverter<'s> for Boolean {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        _context: Context,
        _options: &Self::Options,
    ) -> Result<Self, WebIdlError> {
        Ok(Self(value.boolean_value(scope)))
    }
}

impl<'s> WebIdlConverter<'s> for UnrestrictedDouble {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _options: &Self::Options,
    ) -> Result<Self, WebIdlError> {
        number_value(scope, value, context, "unrestricted double").map(Self)
    }
}

impl<'s> WebIdlConverter<'s> for Double {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _options: &Self::Options,
    ) -> Result<Self, WebIdlError> {
        let value = number_value(scope, value, context, "double")?;
        if value.is_finite() {
            Ok(Self(value))
        } else {
            Err(WebIdlError::new(
                context,
                WebIdlErrorKind::CannotConvert("double"),
            ))
        }
    }
}

impl<'s> WebIdlConverter<'s> for Long {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _options: &Self::Options,
    ) -> Result<Self, WebIdlError> {
        number_value(scope, value, context, "long").map(|value| Self(long(value)))
    }
}

impl<'s> WebIdlConverter<'s> for EnforceRangeLong {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _options: &Self::Options,
    ) -> Result<Self, WebIdlError> {
        let value = number_value(scope, value, context, "[EnforceRange] long")?;
        enforce_range_long(value, context).map(Self)
    }
}

impl<'s> WebIdlConverter<'s> for UnsignedLong {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _options: &Self::Options,
    ) -> Result<Self, WebIdlError> {
        number_value(scope, value, context, "unsigned long").map(|value| Self(unsigned_long(value)))
    }
}

impl<'s> WebIdlConverter<'s> for EnforceRangeUnsignedLong {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _options: &Self::Options,
    ) -> Result<Self, WebIdlError> {
        let value = number_value(scope, value, context, "[EnforceRange] unsigned long")?;
        enforce_range_unsigned_long(value, context).map(Self)
    }
}

impl<'s> WebIdlConverter<'s> for UnsignedShort {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _options: &Self::Options,
    ) -> Result<Self, WebIdlError> {
        number_value(scope, value, context, "unsigned short")
            .map(|value| Self(unsigned_short(value)))
    }
}

impl<'s> WebIdlConverter<'s> for ClampedUnsignedShort {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _options: &Self::Options,
    ) -> Result<Self, WebIdlError> {
        number_value(scope, value, context, "[Clamp] unsigned short")
            .map(|value| Self(clamped_unsigned_short(value)))
    }
}

impl<'s> WebIdlConverter<'s> for UnsignedLongLong {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _options: &Self::Options,
    ) -> Result<Self, WebIdlError> {
        number_value(scope, value, context, "unsigned long long")
            .map(|value| Self(unsigned_long_long(value)))
    }
}

impl<'s> WebIdlConverter<'s> for EnforceRangeUnsignedLongLong {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _options: &Self::Options,
    ) -> Result<Self, WebIdlError> {
        let value = number_value(scope, value, context, "[EnforceRange] unsigned long long")?;
        enforce_range_unsigned_long_long(value, context).map(Self)
    }
}

impl<'s, T> WebIdlConverter<'s> for EnumValue<T>
where
    T: WebIdlEnum,
{
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _options: &Self::Options,
    ) -> Result<Self, WebIdlError> {
        let value = DomString::convert(scope, value, context, &StringOptions::default())?.0;
        T::parse_token(&value)
            .map(Self)
            .ok_or_else(|| WebIdlError::new(context, WebIdlErrorKind::InvalidEnum(T::NAME)))
    }
}

impl<'s, T> WebIdlConverter<'s> for Sequence<T>
where
    T: WebIdlConverter<'s>,
{
    type Options = T::Options;

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        options: &Self::Options,
    ) -> Result<Self, WebIdlError> {
        convert_optional_sequence(scope, value, context, options)?
            .ok_or_else(|| WebIdlError::new(context, WebIdlErrorKind::CannotConvert("sequence")))
    }
}

/// Converts an iterable to a WebIDL sequence, or returns `None` when the value
/// has no `@@iterator` method.
///
/// This is the union-discrimination form of sequence conversion. It reads
/// `@@iterator` exactly once, so callers can fall back to another union member
/// without probing the property separately and then converting it again.
pub fn convert_optional_sequence<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    context: Context,
    options: &T::Options,
) -> Result<Option<Sequence<T>>, WebIdlError>
where
    T: WebIdlConverter<'s>,
{
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        return Ok(None);
    };
    let iterator_key = v8::Symbol::get_iterator(scope);
    let Some(iterator_method) = symbol_property_result(scope, object, iterator_key, context)?
        .filter(|value| !value.is_null_or_undefined())
    else {
        return Ok(None);
    };
    let iterator_method = v8::Local::<v8::Function>::try_from(iterator_method)
        .map_err(|_| WebIdlError::new(context, WebIdlErrorKind::CannotConvert("sequence")))?;
    let (iterator, next_method) =
        sequence_iterator_from_method(scope, value, iterator_method, context)?;
    let mut values = Vec::new();
    while let Some(item) = sequence_iterator_next(scope, iterator, next_method, context)? {
        let (converted, caught_exception) = {
            let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
            let mut conversion_scope = try_catch.init();
            let converted = T::convert(&mut conversion_scope, item, context, options);
            let caught_exception = conversion_scope
                .has_caught()
                .then(|| conversion_scope.exception())
                .flatten()
                .map(|exception| v8::Global::new(&conversion_scope, exception));
            (converted, caught_exception)
        };
        match converted {
            Ok(value) => values.push(value),
            Err(error) => {
                sequence_iterator_close_ignoring_errors(scope, iterator);
                if let Some(exception) = caught_exception {
                    let exception = v8::Local::new(scope, &exception);
                    scope.throw_exception(exception);
                }
                return Err(error);
            }
        }
    }
    Ok(Some(Sequence(values)))
}

// Record conversion collects own keys, then observes each descriptor immediately
// before converting that entry. Getters and Proxy traps can affect later entries.
// If two JavaScript keys become the same WebIDL key after key conversion, the
// later property wins, matching the WebIDL record replacement behavior.
impl<'s, K, V> WebIdlConverter<'s> for Record<K, V>
where
    K: WebIdlConverter<'s> + PartialEq,
    K::Options: Default,
    V: WebIdlConverter<'s>,
    V::Options: Default,
{
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _options: &Self::Options,
    ) -> Result<Self, WebIdlError> {
        let object = v8::Local::<v8::Object>::try_from(value)
            .map_err(|_| WebIdlError::new(context, WebIdlErrorKind::CannotConvert("record")))?;
        let properties = own_property_keys(scope, object, context)?;
        let mut entries: Vec<(K, V)> = Vec::with_capacity(properties.length() as usize);
        for index in 0..properties.length() {
            let key_value = properties.get_index(scope, index).ok_or_else(|| {
                WebIdlError::new(context, WebIdlErrorKind::CannotConvert("record"))
            })?;
            let property = v8::Local::<v8::Name>::try_from(key_value)
                .map_err(|_| WebIdlError::new(context, WebIdlErrorKind::CannotConvert("record")))?;
            if !record_property_is_enumerable(scope, object, property, context)? {
                continue;
            }
            let key = K::convert(scope, key_value, context, &K::Options::default())?;
            let value = record_property_value(scope, object, key_value, context)?;
            let value = V::convert(scope, value, context, &V::Options::default())?;
            if let Some((_, existing_value)) = entries
                .iter_mut()
                .find(|(existing_key, _)| *existing_key == key)
            {
                *existing_value = value;
            } else {
                entries.push((key, value));
            }
        }
        Ok(Self(entries))
    }
}

impl<'s> WebIdlConverter<'s> for v8::Local<'s, v8::Value> {
    type Options = ();

    fn convert(
        _scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        _context: Context,
        _options: &Self::Options,
    ) -> Result<Self, WebIdlError> {
        Ok(value)
    }
}

impl<'s> WebIdlConverter<'s> for v8::Local<'s, v8::Promise> {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _options: &Self::Options,
    ) -> Result<Self, WebIdlError> {
        if let Ok(promise) = v8::Local::<v8::Promise>::try_from(value) {
            return Ok(promise);
        }

        let resolver = v8::PromiseResolver::new(scope)
            .ok_or_else(|| WebIdlError::new(context, WebIdlErrorKind::CannotConvert("Promise")))?;
        let promise = resolver.get_promise(scope);
        if resolver.resolve(scope, value) == Some(true) {
            Ok(promise)
        } else {
            Err(WebIdlError::new(
                context,
                WebIdlErrorKind::CannotConvert("Promise"),
            ))
        }
    }
}

impl<'s> WebIdlConverter<'s> for v8::Local<'s, v8::Object> {
    type Options = ();

    fn convert(
        _scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _options: &Self::Options,
    ) -> Result<Self, WebIdlError> {
        v8::Local::<v8::Object>::try_from(value)
            .map_err(|_| WebIdlError::new(context, WebIdlErrorKind::CannotConvert("object")))
    }
}

impl<'s> WebIdlConverter<'s> for v8::Local<'s, v8::Function> {
    type Options = ();

    fn convert(
        _scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _options: &Self::Options,
    ) -> Result<Self, WebIdlError> {
        v8::Local::<v8::Function>::try_from(value)
            .map_err(|_| WebIdlError::new(context, WebIdlErrorKind::CannotConvert("function")))
    }
}

impl<'s> WebIdlConverter<'s> for moli_webidl_callback::WebIdlCallbackFunction {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _options: &Self::Options,
    ) -> Result<Self, WebIdlError> {
        let callback = v8::Local::<v8::Object>::try_from(value)
            .ok()
            .filter(|callback| callback.is_callable())
            .ok_or_else(|| {
                WebIdlError::new(context, WebIdlErrorKind::CannotConvert("callback function"))
            })?;
        let current_context = scope.get_current_context();
        let relevant_context = callback
            .get_creation_context(scope)
            .unwrap_or(current_context);
        let incumbent_context = scope.get_incumbent_context().unwrap_or(current_context);

        moli_webidl_callback::WebIdlCallbackFunction::try_new(
            scope,
            callback,
            relevant_context,
            incumbent_context,
        )
        .ok_or_else(|| {
            WebIdlError::new(context, WebIdlErrorKind::CannotConvert("callback function"))
        })
    }
}

impl<'s> WebIdlConverter<'s> for moli_webidl_callback::WebIdlCallbackInterface {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _options: &Self::Options,
    ) -> Result<Self, WebIdlError> {
        let callback = v8::Local::<v8::Object>::try_from(value).map_err(|_| {
            WebIdlError::new(
                context,
                WebIdlErrorKind::CannotConvert("callback interface"),
            )
        })?;
        let current_context = scope.get_current_context();
        let relevant_context = callback
            .get_creation_context(scope)
            .unwrap_or(current_context);
        let incumbent_context = scope.get_incumbent_context().unwrap_or(current_context);

        Ok(moli_webidl_callback::WebIdlCallbackInterface::new(
            scope,
            callback,
            relevant_context,
            incumbent_context,
        ))
    }
}

impl<'s> WebIdlConverter<'s> for v8::Local<'s, v8::Array> {
    type Options = ();

    fn convert(
        _scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _options: &Self::Options,
    ) -> Result<Self, WebIdlError> {
        v8::Local::<v8::Array>::try_from(value)
            .map_err(|_| WebIdlError::new(context, WebIdlErrorKind::CannotConvert("Array")))
    }
}

impl<'s> WebIdlConverter<'s> for v8::Local<'s, v8::Uint8Array> {
    type Options = ();

    fn convert(
        _scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _options: &Self::Options,
    ) -> Result<Self, WebIdlError> {
        v8::Local::<v8::Uint8Array>::try_from(value)
            .map_err(|_| WebIdlError::new(context, WebIdlErrorKind::CannotConvert("Uint8Array")))
    }
}

/// Converts one JavaScript value with the default options for `T`.
pub fn convert<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    context: Context,
) -> Result<T, WebIdlError>
where
    T: WebIdlConverter<'s>,
{
    T::convert(scope, value, context, &T::Options::default())
}

/// Converts one JavaScript value with explicit converter options.
///
/// This is the entry point used by generated parsers for field attributes such
/// as `#[webidl(treat_null_as_empty_string)]`.
pub fn convert_with_options<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    context: Context,
    options: &T::Options,
) -> Result<T, WebIdlError>
where
    T: WebIdlConverter<'s>,
{
    T::convert(scope, value, context, options)
}

/// Converts one positional native-binding argument with default converter
/// options.
pub fn argument<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
    context: Context,
) -> Result<T, WebIdlError>
where
    T: WebIdlConverter<'s>,
{
    convert(scope, args.get(index), context)
}

/// Converts one positional native-binding argument with explicit converter
/// options.
pub fn argument_with_options<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
    context: Context,
    options: &T::Options,
) -> Result<T, WebIdlError>
where
    T: WebIdlConverter<'s>,
{
    convert_with_options(scope, args.get(index), context, options)
}

/// Converts one required positional argument and throws on failure.
///
/// New code should generally prefer `parse_args` with a derived
/// `WebIdlArguments` struct. This helper remains for older hand-written native
/// binding paths that already own their TypeError text.
pub fn required_argument<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
    context: Context,
    missing_message: &'static str,
) -> Option<T>
where
    T: WebIdlConverter<'s>,
{
    if args.length() <= index {
        throw_type_error(scope, missing_message);
        return None;
    }
    argument(scope, args, index, context).map_or_else(
        |error| {
            throw_error(scope, &error);
            None
        },
        Some,
    )
}

/// Converts one optional positional argument, using `default` when missing or
/// `undefined`.
pub fn optional_argument_or<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
    context: Context,
    default: T,
) -> Option<T>
where
    T: WebIdlConverter<'s>,
{
    if args.length() <= index || args.get(index).is_undefined() {
        return Some(default);
    }
    argument(scope, args, index, context).map_or_else(
        |error| {
            throw_error(scope, &error);
            None
        },
        Some,
    )
}

/// Parses a derived `WebIdlArguments` struct and throws conversion errors.
///
/// Renderer callbacks normally use this helper and return immediately when it
/// returns `None`. Use `try_parse_args` when the caller needs to inspect or map
/// the `WebIdlError` before throwing.
pub fn parse_args<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<T>
where
    T: WebIdlArguments<'s>,
{
    try_parse_args(scope, args).map_or_else(
        |error| {
            throw_error(scope, &error);
            None
        },
        Some,
    )
}

/// Parses a derived `WebIdlArguments` struct without throwing.
pub fn try_parse_args<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<T, WebIdlError>
where
    T: WebIdlArguments<'s>,
{
    T::parse_arguments(scope, args)
}

/// Reads and converts an optional dictionary member.
///
/// Missing properties and `undefined` both produce `Ok(None)`. `null` is still
/// converted normally unless the target converter or a caller-specific helper
/// treats it specially.
pub fn optional_member<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    key: &'static str,
    context: Context,
) -> Result<Option<T>, WebIdlError>
where
    T: WebIdlConverter<'s>,
{
    let Some(value) = property_result(scope, object, key, context)? else {
        return Ok(None);
    };
    if value.is_undefined() {
        return Ok(None);
    }
    convert(scope, value, context).map(Some)
}

/// Reads and converts an optional dictionary member with explicit options.
pub fn optional_member_with_options<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    key: &'static str,
    context: Context,
    options: &T::Options,
) -> Result<Option<T>, WebIdlError>
where
    T: WebIdlConverter<'s>,
{
    let Some(value) = property_result(scope, object, key, context)? else {
        return Ok(None);
    };
    if value.is_undefined() {
        return Ok(None);
    }
    convert_with_options(scope, value, context, options).map(Some)
}

/// Reads and converts an optional dictionary member, returning `default` when
/// the member is missing or `undefined`.
pub fn optional_member_or<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    key: &'static str,
    context: Context,
    default: T,
) -> Result<T, WebIdlError>
where
    T: WebIdlConverter<'s>,
{
    optional_member(scope, object, key, context).map(|value| value.unwrap_or(default))
}

/// Reads and converts an optional dictionary member with explicit options,
/// returning `default` when the member is missing or `undefined`.
pub fn optional_member_or_with_options<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    key: &'static str,
    context: Context,
    default: T,
    options: &T::Options,
) -> Result<T, WebIdlError>
where
    T: WebIdlConverter<'s>,
{
    optional_member_with_options(scope, object, key, context, options)
        .map(|value| value.unwrap_or(default))
}

/// Reads and converts an optional legacy dictionary member.
///
/// This differs from `optional_member`: both `undefined` and `null` count as
/// missing. Use it only for APIs whose browser behavior historically treats
/// nullish dictionary members as absent.
pub fn legacy_optional_member<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    key: &'static str,
    context: Context,
) -> Result<Option<T>, WebIdlError>
where
    T: WebIdlConverter<'s>,
{
    let Some(value) = property_result(scope, object, key, context)? else {
        return Ok(None);
    };
    if is_nullish(value) {
        return Ok(None);
    }
    convert(scope, value, context).map(Some)
}

/// Reads and converts an optional legacy dictionary member with explicit
/// converter options.
pub fn legacy_optional_member_with_options<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    key: &'static str,
    context: Context,
    options: &T::Options,
) -> Result<Option<T>, WebIdlError>
where
    T: WebIdlConverter<'s>,
{
    let Some(value) = property_result(scope, object, key, context)? else {
        return Ok(None);
    };
    if is_nullish(value) {
        return Ok(None);
    }
    convert_with_options(scope, value, context, options).map(Some)
}

/// Reads and converts an optional legacy dictionary member, returning `default`
/// when the member is missing, `undefined`, or `null`.
pub fn legacy_optional_member_or<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    key: &'static str,
    context: Context,
    default: T,
) -> Result<T, WebIdlError>
where
    T: WebIdlConverter<'s>,
{
    legacy_optional_member(scope, object, key, context).map(|value| value.unwrap_or(default))
}

/// Reads and converts an optional legacy dictionary member with explicit
/// options, returning `default` when the member is missing, `undefined`, or
/// `null`.
pub fn legacy_optional_member_or_with_options<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    key: &'static str,
    context: Context,
    default: T,
    options: &T::Options,
) -> Result<T, WebIdlError>
where
    T: WebIdlConverter<'s>,
{
    legacy_optional_member_with_options(scope, object, key, context, options)
        .map(|value| value.unwrap_or(default))
}

/// Parses a value as an optional WebIDL dictionary.
///
/// `null` and `undefined` produce `Ok(None)`. Non-object values fail with a
/// dictionary conversion error. Object values are delegated to the derived
/// `WebIdlDictionary` implementation.
pub fn parse_dictionary<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    context: Context,
) -> Result<Option<T>, WebIdlError>
where
    T: WebIdlDictionary<'s>,
{
    dictionary_value(value, context)?
        .map(|object| T::parse_dictionary(scope, object))
        .transpose()
}

/// Parses a value that is already known to be a dictionary object.
pub fn parse_dictionary_object<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Result<T, WebIdlError>
where
    T: WebIdlDictionary<'s>,
{
    T::parse_dictionary(scope, object)
}

/// Legacy convenience helper for boolean dictionary members.
///
/// New dictionary code should usually use `#[derive(WebIdlDictionary)]`; this
/// helper remains for older call sites that intentionally swallow conversion
/// errors and fall back to a product-compatible default.
pub fn legacy_bool_member_or<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    prefix: &'static str,
    key: &'static str,
    default: bool,
) -> bool {
    legacy_optional_member_or::<Boolean>(
        scope,
        object,
        key,
        Context::member(prefix, key),
        Boolean(default),
    )
    .map(|value| value.0)
    .unwrap_or(default)
}

/// Legacy convenience helper for numeric dictionary members.
pub fn legacy_number_member_or<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    prefix: &'static str,
    key: &'static str,
    default: f64,
) -> f64 {
    legacy_optional_member_or::<UnrestrictedDouble>(
        scope,
        object,
        key,
        Context::member(prefix, key),
        UnrestrictedDouble(default),
    )
    .map(|value| value.0)
    .ok()
    .filter(|value| !value.is_nan())
    .unwrap_or(default)
}

/// Legacy convenience helper for DOMString dictionary members.
pub fn legacy_string_member_or<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    prefix: &'static str,
    key: &'static str,
    default: &str,
) -> String {
    legacy_optional_member_or::<DomString>(
        scope,
        object,
        key,
        Context::member(prefix, key),
        DomString(default.to_owned()),
    )
    .map(Into::into)
    .unwrap_or_else(|_| default.to_owned())
}

/// Legacy convenience helper for positional DOMString arguments.
pub fn string_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Option<String> {
    argument::<DomString>(
        scope,
        args,
        index,
        Context::argument("", (index + 1) as usize),
    )
    .ok()
    .map(Into::into)
}

/// Converts a JavaScript value to an unrestricted number, falling back on
/// conversion failure.
pub fn number_or<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    default: f64,
) -> f64 {
    convert::<UnrestrictedDouble>(scope, value, Context::argument("", 1))
        .map(Into::into)
        .unwrap_or(default)
}

/// Legacy convenience helper for positional unrestricted-number arguments.
pub fn number_arg_or<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
    prefix: &'static str,
    default: f64,
) -> f64 {
    argument::<UnrestrictedDouble>(
        scope,
        args,
        index,
        Context::argument(prefix, (index + 1) as usize),
    )
    .map(Into::into)
    .unwrap_or(default)
}

/// Parses timer-like millisecond input where missing, non-finite, and
/// non-positive values collapse to zero.
pub fn non_negative_milliseconds_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
    prefix: &'static str,
) -> u32 {
    if args.length() <= index {
        return 0;
    }
    let value = number_arg_or(scope, args, index, prefix, 0.0);
    if !value.is_finite() || value <= 0.0 {
        return 0;
    }
    value.min(u32::MAX as f64) as u32
}

/// Parses `setTimeout`/`setInterval` style delay input using WebIDL `long`
/// wrapping before clamping negative values to zero.
pub fn timer_milliseconds_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
    prefix: &'static str,
) -> u32 {
    if args.length() <= index {
        return 0;
    }
    let value = number_arg_or(scope, args, index, prefix, 0.0);
    let value = long(value);
    value.max(0) as u32
}

fn string_value_utf16<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    context: Context,
    kind: &'static str,
) -> Result<Vec<u16>, WebIdlError> {
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    match value.to_string(&scope) {
        Some(value) => {
            let mut buffer = vec![0; value.length()];
            value.write_v2(&scope, 0, &mut buffer, v8::WriteFlags::empty());
            Ok(buffer)
        }
        None if scope.has_caught() => {
            let _ = scope.rethrow();
            Err(WebIdlError::pending_exception(context))
        }
        None => Err(WebIdlError::new(
            context,
            WebIdlErrorKind::CannotConvert(kind),
        )),
    }
}

fn scalar_value_string_from_utf16(units: &[u16]) -> String {
    let mut output = String::with_capacity(units.len());
    let mut index = 0;
    while index < units.len() {
        let unit = units[index];
        if (0xD800..=0xDBFF).contains(&unit) {
            if let Some(&trail) = units.get(index + 1)
                && (0xDC00..=0xDFFF).contains(&trail)
            {
                let high = u32::from(unit) - 0xD800;
                let low = u32::from(trail) - 0xDC00;
                let scalar = 0x10000 + ((high << 10) | low);
                if let Some(ch) = char::from_u32(scalar) {
                    output.push(ch);
                }
                index += 2;
                continue;
            }
            output.push('\u{FFFD}');
        } else if (0xDC00..=0xDFFF).contains(&unit) {
            output.push('\u{FFFD}');
        } else if let Some(ch) = char::from_u32(u32::from(unit)) {
            output.push(ch);
        }
        index += 1;
    }
    output
}

fn number_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    context: Context,
    kind: &'static str,
) -> Result<f64, WebIdlError> {
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    match value.number_value(&scope) {
        Some(value) => Ok(value),
        None if scope.has_caught() => {
            let _ = scope.rethrow();
            Err(WebIdlError::pending_exception(context))
        }
        None => Err(WebIdlError::new(
            context,
            WebIdlErrorKind::CannotConvert(kind),
        )),
    }
}

fn sequence_iterator_from_method<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    iterator_method: v8::Local<'s, v8::Function>,
    context: Context,
) -> Result<(v8::Local<'s, v8::Object>, v8::Local<'s, v8::Function>), WebIdlError> {
    let iterator_value = call_sequence_function(scope, iterator_method, value, &[], context)?
        .ok_or_else(|| WebIdlError::new(context, WebIdlErrorKind::CannotConvert("sequence")))?;
    let iterator = v8::Local::<v8::Object>::try_from(iterator_value)
        .map_err(|_| WebIdlError::new(context, WebIdlErrorKind::CannotConvert("sequence")))?;
    let next_method = property_result(scope, iterator, "next", context)?
        .filter(|value| !value.is_null_or_undefined())
        .ok_or_else(|| WebIdlError::new(context, WebIdlErrorKind::CannotConvert("sequence")))?;
    let next_method = v8::Local::<v8::Function>::try_from(next_method)
        .map_err(|_| WebIdlError::new(context, WebIdlErrorKind::CannotConvert("sequence")))?;
    Ok((iterator, next_method))
}

fn sequence_iterator_next<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    iterator: v8::Local<'s, v8::Object>,
    next_method: v8::Local<'s, v8::Function>,
    context: Context,
) -> Result<Option<v8::Local<'s, v8::Value>>, WebIdlError> {
    let step = call_sequence_function(scope, next_method, iterator.into(), &[], context)?
        .ok_or_else(|| WebIdlError::new(context, WebIdlErrorKind::CannotConvert("sequence")))?;
    let step = v8::Local::<v8::Object>::try_from(step)
        .map_err(|_| WebIdlError::new(context, WebIdlErrorKind::CannotConvert("sequence")))?;
    let done = property_result(scope, step, "done", context)?
        .is_some_and(|value| value.boolean_value(scope));
    if done {
        return Ok(None);
    }
    let value = property_result(scope, step, "value", context)?
        .ok_or_else(|| WebIdlError::new(context, WebIdlErrorKind::CannotConvert("sequence")))?;
    Ok(Some(value))
}

fn sequence_iterator_close_ignoring_errors<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    iterator: v8::Local<'s, v8::Object>,
) {
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let scope = try_catch.init();
    let Some(return_key) = v8::String::new(&scope, "return") else {
        return;
    };
    let Some(return_method) = iterator.get(&scope, return_key.into()) else {
        return;
    };
    if return_method.is_null_or_undefined() {
        return;
    }
    let Ok(return_method) = v8::Local::<v8::Function>::try_from(return_method) else {
        return;
    };
    let _ = return_method.call(&scope, iterator.into(), &[]);
}

fn call_sequence_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    function: v8::Local<'s, v8::Function>,
    receiver: v8::Local<'s, v8::Value>,
    args: &[v8::Local<'s, v8::Value>],
    context: Context,
) -> Result<Option<v8::Local<'s, v8::Value>>, WebIdlError> {
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    match function.call(&scope, receiver, args) {
        Some(value) => Ok(Some(value)),
        None if scope.has_caught() => {
            let _ = scope.rethrow();
            Err(WebIdlError::pending_exception(context))
        }
        None => Err(WebIdlError::new(
            context,
            WebIdlErrorKind::CannotConvert("sequence"),
        )),
    }
}

fn own_property_keys<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    context: Context,
) -> Result<v8::Local<'s, v8::Array>, WebIdlError> {
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    let args = v8::GetPropertyNamesArgs {
        property_filter: v8::PropertyFilter::ALL_PROPERTIES,
        key_conversion: v8::KeyConversionMode::ConvertToString,
        ..Default::default()
    };
    match object.get_own_property_names(&scope, args) {
        Some(properties) => Ok(properties),
        None if scope.has_caught() => {
            let _ = scope.rethrow();
            Err(WebIdlError::pending_exception(context))
        }
        None => Err(WebIdlError::new(
            context,
            WebIdlErrorKind::CannotConvert("record"),
        )),
    }
}

fn record_property_is_enumerable<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    key: v8::Local<'s, v8::Name>,
    context: Context,
) -> Result<bool, WebIdlError> {
    let descriptor = {
        let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
        let mut scope = try_catch.init();
        match object.get_own_property_descriptor(&scope, key) {
            Some(descriptor) => descriptor,
            None if scope.has_caught() => {
                let _ = scope.rethrow();
                return Err(WebIdlError::pending_exception(context));
            }
            None => {
                return Err(WebIdlError::new(
                    context,
                    WebIdlErrorKind::CannotConvert("record"),
                ));
            }
        }
    };
    if descriptor.is_undefined() {
        return Ok(false);
    }
    let descriptor = v8::Local::<v8::Object>::try_from(descriptor)
        .map_err(|_| WebIdlError::new(context, WebIdlErrorKind::CannotConvert("record")))?;
    property_result(scope, descriptor, "enumerable", context)
        .map(|value| value.is_some_and(|value| value.is_true()))
}

fn record_property_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    key: v8::Local<'s, v8::Value>,
    context: Context,
) -> Result<v8::Local<'s, v8::Value>, WebIdlError> {
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    match object.get(&scope, key) {
        Some(value) => Ok(value),
        None if scope.has_caught() => {
            let _ = scope.rethrow();
            Err(WebIdlError::pending_exception(context))
        }
        None => Err(WebIdlError::new(
            context,
            WebIdlErrorKind::CannotConvert("record"),
        )),
    }
}

fn unsigned_long(value: f64) -> u32 {
    if !value.is_finite() || value == 0.0 {
        return 0;
    }
    let integer = value.trunc();
    let wrapped = integer.rem_euclid(2f64.powi(32));
    wrapped as u32
}

fn unsigned_short(value: f64) -> u16 {
    if !value.is_finite() || value == 0.0 {
        return 0;
    }
    let integer = value.trunc();
    let wrapped = integer.rem_euclid(2f64.powi(16));
    wrapped as u16
}

fn clamped_unsigned_short(value: f64) -> u16 {
    if value.is_nan() {
        return 0;
    }
    let clamped = value.clamp(0.0, f64::from(u16::MAX));
    round_half_even(clamped) as u16
}

fn round_half_even(value: f64) -> f64 {
    let floor = value.floor();
    let fraction = value - floor;
    if fraction < 0.5 {
        floor
    } else if fraction > 0.5 {
        floor + 1.0
    } else if (floor as u64).is_multiple_of(2) {
        floor
    } else {
        floor + 1.0
    }
}

fn unsigned_long_long(value: f64) -> u64 {
    if !value.is_finite() || value == 0.0 {
        return 0;
    }
    let integer = value.trunc();
    let wrapped = integer.rem_euclid(2f64.powi(64));
    wrapped as u64
}

fn enforce_range_unsigned_long_long(value: f64, context: Context) -> Result<u64, WebIdlError> {
    if !value.is_finite() || value < 0.0 || value >= 2f64.powi(64) {
        return Err(WebIdlError::new(
            context,
            WebIdlErrorKind::CannotConvert("[EnforceRange] unsigned long long"),
        ));
    }
    Ok(value.trunc() as u64)
}

fn enforce_range_unsigned_long(value: f64, context: Context) -> Result<u32, WebIdlError> {
    if !value.is_finite() || value < 0.0 || value >= 2f64.powi(32) {
        return Err(WebIdlError::new(
            context,
            WebIdlErrorKind::CannotConvert("[EnforceRange] unsigned long"),
        ));
    }
    Ok(value.trunc() as u32)
}

fn enforce_range_long(value: f64, context: Context) -> Result<i32, WebIdlError> {
    if !value.is_finite() || value < -(2f64.powi(31)) || value >= 2f64.powi(31) {
        return Err(WebIdlError::new(
            context,
            WebIdlErrorKind::CannotConvert("[EnforceRange] long"),
        ));
    }
    Ok(value.trunc() as i32)
}

fn long(value: f64) -> i32 {
    let value = unsigned_long(value);
    if value >= 2u32.pow(31) {
        (i64::from(value) - 2i64.pow(32)) as i32
    } else {
        value as i32
    }
}

#[cfg(test)]
mod tests {
    use super::{
        clamped_unsigned_short, enforce_range_long, enforce_range_unsigned_long,
        enforce_range_unsigned_long_long, long, scalar_value_string_from_utf16, unsigned_long,
        unsigned_long_long, unsigned_short,
    };
    use crate::{Context, WebIdlCallbackFunction, WebIdlCallbackInterface, WebIdlConverter};

    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        let source = v8::String::new(scope, source).expect("test source");
        let script = v8::Script::compile(scope, source, None).expect("compile test source");
        script.run(scope).expect("run test source")
    }

    #[test]
    fn unsigned_long_wraps_per_webidl_rules() {
        assert_eq!(unsigned_long(f64::NAN), 0);
        assert_eq!(unsigned_long(f64::INFINITY), 0);
        assert_eq!(unsigned_long(-1.9), u32::MAX);
        assert_eq!(unsigned_long(1.9), 1);
        assert_eq!(unsigned_long(4_294_967_297.0), 1);
    }

    #[test]
    fn unsigned_short_wraps_per_webidl_rules() {
        assert_eq!(unsigned_short(f64::NAN), 0);
        assert_eq!(unsigned_short(f64::INFINITY), 0);
        assert_eq!(unsigned_short(-1.9), u16::MAX);
        assert_eq!(unsigned_short(1.9), 1);
        assert_eq!(unsigned_short(65_537.0), 1);
    }

    #[test]
    fn clamped_unsigned_short_clamps_and_rounds_half_even() {
        assert_eq!(clamped_unsigned_short(f64::NAN), 0);
        assert_eq!(clamped_unsigned_short(f64::INFINITY), u16::MAX);
        assert_eq!(clamped_unsigned_short(f64::NEG_INFINITY), 0);
        assert_eq!(clamped_unsigned_short(-1.0), 0);
        assert_eq!(clamped_unsigned_short(65_536.0), u16::MAX);
        assert_eq!(clamped_unsigned_short(1.5), 2);
        assert_eq!(clamped_unsigned_short(2.5), 2);
        assert_eq!(clamped_unsigned_short(3.5), 4);
    }

    #[test]
    fn long_wraps_per_webidl_rules() {
        assert_eq!(long(f64::NAN), 0);
        assert_eq!(long(f64::INFINITY), 0);
        assert_eq!(long(-1.9), -1);
        assert_eq!(long(1.9), 1);
        assert_eq!(long(2_147_483_648.0), -2_147_483_648);
        assert_eq!(long(4_294_967_296.0), 0);
        assert_eq!(long(4_294_967_297.0), 1);
    }

    #[test]
    fn unsigned_long_long_wraps_per_webidl_rules() {
        assert_eq!(unsigned_long_long(f64::NAN), 0);
        assert_eq!(unsigned_long_long(f64::INFINITY), 0);
        assert_eq!(unsigned_long_long(-1.0), u64::MAX);
        assert_eq!(unsigned_long_long(1.9), 1);
    }

    #[test]
    fn enforce_range_unsigned_long_long_rejects_out_of_range_values() {
        let context = Context::argument("IDBFactory.open", 2);
        assert!(enforce_range_unsigned_long_long(f64::NAN, context).is_err());
        assert!(enforce_range_unsigned_long_long(f64::INFINITY, context).is_err());
        assert!(enforce_range_unsigned_long_long(-1.0, context).is_err());
        assert!(enforce_range_unsigned_long_long(2f64.powi(64), context).is_err());
        assert_eq!(enforce_range_unsigned_long_long(1.9, context).unwrap(), 1);
    }

    #[test]
    fn enforce_range_unsigned_long_rejects_out_of_range_values() {
        let context = Context::argument("IDBCursor.advance", 1);
        assert!(enforce_range_unsigned_long(f64::NAN, context).is_err());
        assert!(enforce_range_unsigned_long(f64::INFINITY, context).is_err());
        assert!(enforce_range_unsigned_long(-1.0, context).is_err());
        assert!(enforce_range_unsigned_long(2f64.powi(32), context).is_err());
        assert_eq!(enforce_range_unsigned_long(1.9, context).unwrap(), 1);
    }

    #[test]
    fn enforce_range_long_rejects_out_of_range_values() {
        let context = Context::argument("CanvasRenderingContext2D.getImageData", 1);
        assert!(enforce_range_long(f64::NAN, context).is_err());
        assert!(enforce_range_long(f64::INFINITY, context).is_err());
        assert!(enforce_range_long(-(2f64.powi(31)) - 1.0, context).is_err());
        assert!(enforce_range_long(2f64.powi(31), context).is_err());
        assert_eq!(enforce_range_long(-1.9, context).unwrap(), -1);
        assert_eq!(enforce_range_long(1.9, context).unwrap(), 1);
    }

    #[test]
    fn scalar_value_string_replaces_lone_surrogates() {
        assert_eq!(
            scalar_value_string_from_utf16(&[0xD83E, 0xDD95]),
            "\u{1F995}"
        );
        assert_eq!(scalar_value_string_from_utf16(&[0xD800]), "\u{FFFD}");
        assert_eq!(scalar_value_string_from_utf16(&[0xDC00]), "\u{FFFD}");
        assert_eq!(
            scalar_value_string_from_utf16(&[b'a' as u16, 0xD800, b'b' as u16, 0xDC00]),
            "a\u{FFFD}b\u{FFFD}"
        );
    }

    #[test]
    fn callback_function_conversion_captures_relevant_and_incumbent_contexts() {
        moli_v8_test_util::ensure_v8();
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let conversion_context = v8::Context::new(scope, Default::default());
        let relevant_context = v8::Context::new(scope, Default::default());
        let incumbent_context = v8::Context::new(scope, Default::default());
        let callback = {
            let scope = &mut v8::ContextScope::new(scope, relevant_context);
            let callback =
                v8::Local::<v8::Object>::try_from(eval(scope, "(function () { return 42; })"))
                    .expect("callback function");
            v8::Global::new(scope, callback)
        };
        let scope = &mut v8::ContextScope::new(scope, conversion_context);
        let callback = v8::Local::new(scope, &callback);
        let incumbent_scope = std::pin::pin!(v8::BackupIncumbentScope::new(incumbent_context));
        let _incumbent_scope = incumbent_scope.init();

        let converted = WebIdlCallbackFunction::convert(
            scope,
            callback.into(),
            Context::argument("Test.callback", 1),
            &(),
        )
        .expect("callback function conversion");
        let prepared = converted.prepare(scope);

        assert_eq!(prepared.relevant_context(scope), relevant_context);
        assert_eq!(prepared.incumbent_context(scope), incumbent_context);
        assert!(converted.matches(scope, callback));
    }

    #[test]
    fn callback_function_conversion_accepts_callable_proxy_and_rejects_object() {
        moli_v8_test_util::ensure_v8();
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let callable_proxy = eval(scope, "new Proxy(function () {}, {})");
        let non_callable = v8::Object::new(scope);

        assert!(
            WebIdlCallbackFunction::convert(
                scope,
                callable_proxy,
                Context::argument("Test.callback", 1),
                &(),
            )
            .is_ok()
        );
        assert!(
            WebIdlCallbackFunction::convert(
                scope,
                non_callable.into(),
                Context::argument("Test.callback", 1),
                &(),
            )
            .is_err()
        );
    }

    #[test]
    fn callback_interface_conversion_captures_contexts_and_callability() {
        moli_v8_test_util::ensure_v8();
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let conversion_context = v8::Context::new(scope, Default::default());
        let relevant_context = v8::Context::new(scope, Default::default());
        let incumbent_context = v8::Context::new(scope, Default::default());
        let callback = {
            let scope = &mut v8::ContextScope::new(scope, relevant_context);
            let callback = v8::Local::<v8::Object>::try_from(eval(scope, "({ acceptNode() {} })"))
                .expect("callback-interface object");
            v8::Global::new(scope, callback)
        };
        let scope = &mut v8::ContextScope::new(scope, conversion_context);
        let callback = v8::Local::new(scope, &callback);
        let incumbent_scope = std::pin::pin!(v8::BackupIncumbentScope::new(incumbent_context));
        let _incumbent_scope = incumbent_scope.init();

        let converted = WebIdlCallbackInterface::convert(
            scope,
            callback.into(),
            Context::argument("Test.callback", 1),
            &(),
        )
        .expect("callback-interface conversion");
        let prepared = converted.prepare(scope);

        assert_eq!(prepared.relevant_context(scope), relevant_context);
        assert_eq!(prepared.incumbent_context(scope), incumbent_context);
        assert!(!prepared.callable_at_conversion());
        assert!(converted.matches(scope, callback));
    }

    #[test]
    fn callback_interface_conversion_accepts_callable_proxy_and_rejects_primitive() {
        moli_v8_test_util::ensure_v8();
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let callable_proxy = eval(scope, "new Proxy(function () {}, {})");
        let primitive = v8::Integer::new(scope, 7);

        let converted = WebIdlCallbackInterface::convert(
            scope,
            callable_proxy,
            Context::argument("Test.callback", 1),
            &(),
        )
        .expect("callable proxy is a callback-interface object");
        assert!(converted.prepare(scope).callable_at_conversion());
        assert!(
            WebIdlCallbackInterface::convert(
                scope,
                primitive.into(),
                Context::argument("Test.callback", 1),
                &(),
            )
            .is_err()
        );
    }
}
