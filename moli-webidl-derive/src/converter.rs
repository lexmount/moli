use quote::quote;
use syn::spanned::Spanned;
use syn::{Error, Expr, Field, GenericArgument, LitStr, PathArguments, Type};

use crate::attrs::FieldAttrs;

#[derive(Clone, Copy, Default, Eq, PartialEq)]
pub(crate) enum ConverterKind {
    #[default]
    DomString,
    DomString16,
    UsvString,
    ByteString,
    BufferSource,
    CallbackFunction,
    CallbackInterface,
    Long,
    EnforceRangeLong,
    EnforceRangeLongLong,
    Short,
    UnsignedShort,
    ClampedUnsignedShort,
    UnsignedLong,
    EnforceRangeUnsignedLong,
    UnsignedLongLong,
    EnforceRangeUnsignedLongLong,
    Float,
    UnrestrictedFloat,
    Double,
    UnrestrictedDouble,
    Boolean,
    Enum,
    Interface,
    Dictionary,
    Raw,
}

impl ConverterKind {
    /// Parses the string form used by `#[webidl(converter = "...")]`.
    ///
    /// The names stay explicit instead of mirroring Rust type names so call
    /// sites read like WebIDL conversion choices: `dom_string`,
    /// `enforce_range_unsigned_long`, `buffer_source`, and so on.
    pub(crate) fn from_lit(value: &LitStr) -> Result<Self, Error> {
        match value.value().as_str() {
            "dom_string" => Ok(Self::DomString),
            "dom_string16" => Ok(Self::DomString16),
            "usv_string" => Ok(Self::UsvString),
            "byte_string" => Ok(Self::ByteString),
            "buffer_source" => Ok(Self::BufferSource),
            "callback_function" => Ok(Self::CallbackFunction),
            "callback_interface" => Ok(Self::CallbackInterface),
            "long" => Ok(Self::Long),
            "enforce_range_long" => Ok(Self::EnforceRangeLong),
            "enforce_range_long_long" => Ok(Self::EnforceRangeLongLong),
            "short" => Ok(Self::Short),
            "unsigned_short" => Ok(Self::UnsignedShort),
            "clamped_unsigned_short" => Ok(Self::ClampedUnsignedShort),
            "unsigned_long" => Ok(Self::UnsignedLong),
            "enforce_range_unsigned_long" => Ok(Self::EnforceRangeUnsignedLong),
            "unsigned_long_long" => Ok(Self::UnsignedLongLong),
            "enforce_range_unsigned_long_long" => Ok(Self::EnforceRangeUnsignedLongLong),
            "float" => Ok(Self::Float),
            "unrestricted_float" => Ok(Self::UnrestrictedFloat),
            "double" => Ok(Self::Double),
            "unrestricted_double" => Ok(Self::UnrestrictedDouble),
            "boolean" => Ok(Self::Boolean),
            "enum" => Ok(Self::Enum),
            "raw" => Ok(Self::Raw),
            _ => Err(Error::new(value.span(), "unsupported converter kind")),
        }
    }

    /// Returns the wrapper type whose `WebIdlConverter` impl performs the actual
    /// conversion. `Raw` is the only case that keeps the field type directly and
    /// should be reserved for V8 locals that are intentionally not converted.
    pub(crate) fn wrapper_type(self, ty: &Type) -> proc_macro2::TokenStream {
        match self {
            Self::DomString => quote!(::moli_webidl::DomString),
            Self::DomString16 => quote!(::moli_webidl::DomString16),
            Self::UsvString => quote!(::moli_webidl::UsvString),
            Self::ByteString => quote!(::moli_webidl::ByteString),
            Self::BufferSource => quote!(::moli_webidl::BufferSource),
            Self::CallbackFunction => quote!(::moli_webidl::WebIdlCallbackFunction),
            Self::CallbackInterface => quote!(::moli_webidl::WebIdlCallbackInterface),
            Self::Long => quote!(::moli_webidl::Long),
            Self::EnforceRangeLong => quote!(::moli_webidl::EnforceRangeLong),
            Self::EnforceRangeLongLong => quote!(::moli_webidl::EnforceRangeLongLong),
            Self::Short => quote!(::moli_webidl::Short),
            Self::UnsignedShort => quote!(::moli_webidl::UnsignedShort),
            Self::ClampedUnsignedShort => quote!(::moli_webidl::ClampedUnsignedShort),
            Self::UnsignedLong => quote!(::moli_webidl::UnsignedLong),
            Self::EnforceRangeUnsignedLong => {
                quote!(::moli_webidl::EnforceRangeUnsignedLong)
            }
            Self::UnsignedLongLong => quote!(::moli_webidl::UnsignedLongLong),
            Self::EnforceRangeUnsignedLongLong => {
                quote!(::moli_webidl::EnforceRangeUnsignedLongLong)
            }
            Self::Float => quote!(::moli_webidl::Float),
            Self::UnrestrictedFloat => quote!(::moli_webidl::UnrestrictedFloat),
            Self::Double => quote!(::moli_webidl::Double),
            Self::UnrestrictedDouble => quote!(::moli_webidl::UnrestrictedDouble),
            Self::Boolean => quote!(::moli_webidl::Boolean),
            Self::Enum => quote!(::moli_webidl::EnumValue<#ty>),
            Self::Interface => quote!(::moli_webidl::InterfaceObject<'_>),
            Self::Dictionary => quote!(::moli_webidl::Dictionary<#ty>),
            Self::Raw => quote!(#ty),
        }
    }

    /// Wraps a Rust default expression in the converter wrapper expected by the
    /// generated optional/default path.
    pub(crate) fn wrap_default(self, expr: Expr) -> proc_macro2::TokenStream {
        match self {
            Self::DomString => {
                quote!(::moli_webidl::DomString(::std::convert::Into::into(#expr)))
            }
            Self::DomString16 => {
                quote!(::moli_webidl::DomString16(::std::convert::Into::into(#expr)))
            }
            Self::UsvString => {
                quote!(::moli_webidl::UsvString(::std::convert::Into::into(#expr)))
            }
            Self::ByteString => {
                quote!(::moli_webidl::ByteString::from_scalar_string(#expr))
            }
            Self::BufferSource => quote!(::moli_webidl::BufferSource::from(#expr)),
            Self::CallbackFunction | Self::CallbackInterface => quote!(#expr),
            Self::Long => quote!(::moli_webidl::Long(#expr)),
            Self::EnforceRangeLong => quote!(::moli_webidl::EnforceRangeLong(#expr)),
            Self::EnforceRangeLongLong => quote!(::moli_webidl::EnforceRangeLongLong(#expr)),
            Self::Short => quote!(::moli_webidl::Short(#expr)),
            Self::UnsignedShort => quote!(::moli_webidl::UnsignedShort(#expr)),
            Self::ClampedUnsignedShort => {
                quote!(::moli_webidl::ClampedUnsignedShort(#expr))
            }
            Self::UnsignedLong => quote!(::moli_webidl::UnsignedLong(#expr)),
            Self::EnforceRangeUnsignedLong => {
                quote!(::moli_webidl::EnforceRangeUnsignedLong(#expr))
            }
            Self::UnsignedLongLong => quote!(::moli_webidl::UnsignedLongLong(#expr)),
            Self::EnforceRangeUnsignedLongLong => {
                quote!(::moli_webidl::EnforceRangeUnsignedLongLong(#expr))
            }
            Self::Float => quote!(::moli_webidl::Float(#expr)),
            Self::UnrestrictedFloat => quote!(::moli_webidl::UnrestrictedFloat(#expr)),
            Self::Double => quote!(::moli_webidl::Double(#expr)),
            Self::UnrestrictedDouble => quote!(::moli_webidl::UnrestrictedDouble(#expr)),
            Self::Boolean => quote!(::moli_webidl::Boolean(#expr)),
            Self::Enum => quote!(::moli_webidl::EnumValue(#expr)),
            Self::Interface => quote!(::moli_webidl::InterfaceObject(#expr)),
            Self::Dictionary => quote!(::moli_webidl::Dictionary(#expr)),
            Self::Raw => quote!(#expr),
        }
    }

    /// Projects a converted wrapper back to the Rust field type.
    pub(crate) fn unwrap_value(self, value: proc_macro2::TokenStream) -> proc_macro2::TokenStream {
        match self {
            Self::ByteString => quote!(::std::convert::Into::into(#value)),
            Self::BufferSource => quote!(::std::convert::Into::into(#value)),
            Self::CallbackFunction | Self::CallbackInterface => value,
            Self::DomString
            | Self::DomString16
            | Self::UsvString
            | Self::Long
            | Self::EnforceRangeLong
            | Self::EnforceRangeLongLong
            | Self::Short
            | Self::UnsignedShort
            | Self::ClampedUnsignedShort
            | Self::UnsignedLong
            | Self::EnforceRangeUnsignedLong
            | Self::UnsignedLongLong
            | Self::EnforceRangeUnsignedLongLong
            | Self::Float
            | Self::UnrestrictedFloat
            | Self::Double
            | Self::UnrestrictedDouble
            | Self::Boolean
            | Self::Enum
            | Self::Interface
            | Self::Dictionary => quote!(#value.0),
            Self::Raw => value,
        }
    }

    /// Builds the options value passed to `WebIdlConverter::convert`.
    ///
    /// String and interface converters accept options. Rejecting
    /// `treat_null_as_empty_string` on non-string converters keeps invalid
    /// derive usage as a compile-time error instead of silently ignoring it.
    pub(crate) fn options_expr(
        self,
        attrs: &FieldAttrs,
    ) -> Result<proc_macro2::TokenStream, Error> {
        if self == Self::Interface {
            let interface = attrs.interface.as_ref().expect("interface converter path");
            let brand_check = attrs
                .brand_check
                .as_ref()
                .map(|path| quote!(#path))
                .unwrap_or_else(|| quote!(<#interface>::is_instance));
            return Ok(quote!(::moli_webidl::InterfaceOptions {
                name: <#interface>::NAME,
                brand_check: #brand_check,
            }));
        }
        if attrs.treat_null_as_empty_string {
            match self {
                Self::DomString | Self::DomString16 | Self::UsvString | Self::ByteString => {
                    return Ok(quote!(::moli_webidl::StringOptions {
                        treat_null_as_empty_string: true,
                    }));
                }
                _ => {
                    return Err(Error::new(
                        proc_macro2::Span::call_site(),
                        "treat_null_as_empty_string only applies to string converters",
                    ));
                }
            }
        }
        Ok(quote!(::std::default::Default::default()))
    }
}

/// A field conversion, optionally collecting an iterable of converted items.
#[derive(Clone, Copy)]
pub(crate) struct FieldConverter {
    kind: ConverterKind,
    sequence: bool,
}

impl FieldConverter {
    pub(crate) fn wrapper_type(self, ty: &Type) -> proc_macro2::TokenStream {
        if self.sequence {
            let item_type = vec_inner_type(ty).expect("validated sequence Vec");
            let item_converter = self.kind.wrapper_type(item_type);
            quote!(::moli_webidl::Sequence<#item_converter>)
        } else {
            self.kind.wrapper_type(ty)
        }
    }

    pub(crate) fn wrap_default(self, expr: Expr, ty: &Type) -> proc_macro2::TokenStream {
        if self.sequence {
            let item = self.kind.wrap_default(syn::parse_quote!(item));
            quote!({
                let default_items: #ty = #expr;
                ::moli_webidl::Sequence(default_items.into_iter().map(|item| #item).collect())
            })
        } else {
            self.kind.wrap_default(expr)
        }
    }

    pub(crate) fn unwrap_value(self, value: proc_macro2::TokenStream) -> proc_macro2::TokenStream {
        if self.sequence {
            if self.kind == ConverterKind::Raw {
                return quote!(#value.0);
            }
            let item = self.kind.unwrap_value(quote!(item));
            quote!(#value.0.into_iter().map(|item| #item).collect::<::std::vec::Vec<_>>())
        } else {
            self.kind.unwrap_value(value)
        }
    }

    pub(crate) fn options_expr(
        self,
        attrs: &FieldAttrs,
    ) -> Result<proc_macro2::TokenStream, Error> {
        self.kind.options_expr(attrs)
    }
}

pub(crate) fn converter_kind(field: &Field, attrs: &FieldAttrs) -> Result<FieldConverter, Error> {
    converter_kind_for_field_type(field, attrs, inner_type_for_field(field))
}

pub(crate) fn converter_kind_for_field_type(
    field: &Field,
    attrs: &FieldAttrs,
    ty: &Type,
) -> Result<FieldConverter, Error> {
    let ty = if attrs.sequence {
        vec_inner_type(ty).ok_or_else(|| {
            Error::new(
                ty.span(),
                "sequence fields must use Vec<T> (or Option<Vec<T>>)",
            )
        })?
    } else {
        ty
    };
    let kind = element_converter_kind(field, attrs, ty)?;
    Ok(FieldConverter {
        kind,
        sequence: attrs.sequence,
    })
}

fn element_converter_kind(
    field: &Field,
    attrs: &FieldAttrs,
    ty: &Type,
) -> Result<ConverterKind, Error> {
    if attrs.dictionary {
        return Ok(ConverterKind::Dictionary);
    }
    if attrs.interface.is_some() {
        if !is_v8_object_type(ty) {
            return Err(Error::new(
                ty.span(),
                "interface fields must use v8::Local<'_, v8::Object> (or Option/sequence/variadic Vec of it)",
            ));
        }
        return Ok(ConverterKind::Interface);
    }
    if let Some(converter) = attrs.converter {
        return Ok(converter);
    }
    converter_kind_for_type(ty).ok_or_else(|| {
        Error::new(
            field.span(),
            "could not infer converter; add #[webidl(converter = \"...\")]",
        )
    })
}

pub(crate) fn converter_kind_for_type(ty: &Type) -> Option<ConverterKind> {
    infer_converter_kind(ty)
}

/// Returns the type that should be converted for a field.
///
/// `Option<T>` fields still convert `T`; the argument/dictionary expansion owns
/// the missing/nullability semantics around that conversion.
pub(crate) fn inner_type_for_field(field: &Field) -> &Type {
    option_inner_type(&field.ty).unwrap_or(&field.ty)
}

/// Extracts the item type for a variadic `Vec<T>` argument field.
pub(crate) fn vec_inner_type(ty: &Type) -> Option<&Type> {
    let Type::Path(type_path) = ty else {
        return None;
    };
    let segment = type_path.path.segments.last()?;
    if segment.ident != "Vec" {
        return None;
    }
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    let argument = arguments.args.first()?;
    let GenericArgument::Type(inner) = argument else {
        return None;
    };
    Some(inner)
}

fn infer_converter_kind(ty: &Type) -> Option<ConverterKind> {
    // Inference intentionally covers only unambiguous Rust boundary types.
    // Nullable `Option<T>` handling is performed by the caller, and unknown
    // domain enums must opt into `converter = "enum"` so invalid-token errors
    // get the correct WebIDL enum name.
    let ty = option_inner_type(ty).unwrap_or(ty);
    if is_type_ident(ty, "u32") {
        return Some(ConverterKind::UnsignedLong);
    }
    if is_type_ident(ty, "i32") {
        return Some(ConverterKind::Long);
    }
    if is_type_ident(ty, "u16") {
        return Some(ConverterKind::UnsignedShort);
    }
    if is_type_ident(ty, "i16") {
        return Some(ConverterKind::Short);
    }
    if is_type_ident(ty, "u64") {
        return Some(ConverterKind::UnsignedLongLong);
    }
    if is_type_ident(ty, "f32") {
        return Some(ConverterKind::Float);
    }
    if is_type_ident(ty, "f64") {
        return Some(ConverterKind::UnrestrictedDouble);
    }
    if is_type_ident(ty, "bool") {
        return Some(ConverterKind::Boolean);
    }
    if is_type_ident(ty, "String") {
        return Some(ConverterKind::DomString);
    }
    if is_type_ident(ty, "BufferSource") {
        return Some(ConverterKind::BufferSource);
    }
    if is_type_ident(ty, "WebIdlCallbackFunction") {
        return Some(ConverterKind::CallbackFunction);
    }
    if is_type_ident(ty, "WebIdlCallbackInterface") {
        return Some(ConverterKind::CallbackInterface);
    }
    if is_v8_local_type(ty) {
        return Some(ConverterKind::Raw);
    }
    if option_inner_type(ty).is_some() {
        return None;
    }
    None
}

pub(crate) fn is_option_type(ty: &Type) -> bool {
    option_inner_type(ty).is_some()
}

fn is_type_ident(ty: &Type, ident: &str) -> bool {
    match ty {
        Type::Path(path) => path.path.is_ident(ident),
        _ => false,
    }
}

fn is_v8_local_type(ty: &Type) -> bool {
    let Type::Path(type_path) = ty else {
        return false;
    };
    type_path
        .path
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "Local")
}

fn is_v8_object_type(ty: &Type) -> bool {
    let Type::Path(type_path) = ty else {
        return false;
    };
    let Some(segment) = type_path.path.segments.last() else {
        return false;
    };
    if segment.ident != "Local" {
        return false;
    }
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return false;
    };
    matches!(arguments.args.last(), Some(GenericArgument::Type(Type::Path(path)))
        if path.path.segments.last().is_some_and(|segment| segment.ident == "Object"))
}

fn option_inner_type(ty: &Type) -> Option<&Type> {
    let Type::Path(type_path) = ty else {
        return None;
    };
    let segment = type_path.path.segments.last()?;
    if segment.ident != "Option" {
        return None;
    }
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    let argument = arguments.args.first()?;
    let GenericArgument::Type(inner) = argument else {
        return None;
    };
    Some(inner)
}
