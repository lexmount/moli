use crate::attrs::{RenameRule, field_ident, field_member_name, parse_field_attrs};
use crate::converter::{
    ConverterKind, converter_kind, inner_type_for_field, is_option_type, vec_inner_type,
};
use quote::quote;
use syn::spanned::Spanned;
use syn::{Error, Field, GenericParam, Generics, Ident, Lifetime};

pub(crate) fn expand_snapshot(
    name: &Ident,
    generics: &Generics,
    fields: &[&Field],
    rename_all: RenameRule,
    lifetime: &Lifetime,
) -> Result<proc_macro2::TokenStream, Error> {
    let mut members = Vec::new();
    let mut writes = Vec::new();
    let mut inherited_write = None;
    let mut inherited = quote!(::std::option::Option::None);
    let mut fields = fields
        .iter()
        .map(|field| {
            let attrs = parse_field_attrs(field)?;
            Ok((
                !attrs.inherit,
                attrs.snapshot_order.unwrap_or(u32::MAX),
                *field,
            ))
        })
        .collect::<Result<Vec<_>, Error>>()?;
    fields.sort_by_key(|(own, order, _)| (*own, *order));
    for (_, _, field) in fields {
        let attrs = parse_field_attrs(field)?;
        let ident = field_ident(field)?;
        if attrs.inherit {
            let ty = &field.ty;
            inherited = quote!(::std::option::Option::Some(&<#ty as ::moli_webidl::WebIdlDictionarySnapshot>::SNAPSHOT_SCHEMA));
            inherited_write = Some(
                quote!(::moli_webidl::WebIdlDictionarySerialize::write_snapshot_fields(&self.#ident, scope, writer)?;),
            );
            continue;
        }
        if attrs.snapshot_frozen && !attrs.sequence {
            return Err(Error::new(
                field.span(),
                "snapshot_frozen requires a sequence field",
            ));
        }
        if attrs.snapshot_default.is_some() && !is_option_type(&field.ty) {
            return Err(Error::new(
                field.span(),
                "snapshot_default requires an Option field",
            ));
        }
        if attrs.snapshot_slot.is_some() && attrs.snapshot_attributes.is_some() {
            return Err(Error::new(
                field.span(),
                "private snapshot slots cannot have property attributes",
            ));
        }
        let converter = if attrs.with.is_some() {
            ConverterKind::Raw
        } else {
            converter_kind(field, &attrs)?.kind
        };
        let field_ty = inner_type_for_field(field);
        let ty = if attrs.sequence {
            vec_inner_type(field_ty).expect("validated sequence")
        } else {
            field_ty
        };
        let item_kind = if let Some(kind) = &attrs.snapshot_kind {
            quote!(#kind)
        } else {
            match converter {
                ConverterKind::Dictionary => {
                    quote!(::moli_webidl::SnapshotKind::Dictionary(&<#ty as ::moli_webidl::WebIdlDictionarySnapshot>::SNAPSHOT_SCHEMA))
                }
                ConverterKind::Raw | ConverterKind::Uint8Array => {
                    quote!(<#ty as ::moli_webidl::WebIdlSnapshotValue>::SNAPSHOT_KIND)
                }
                ConverterKind::Interface
                | ConverterKind::CallbackFunction
                | ConverterKind::CallbackInterface => quote!(::moli_webidl::SnapshotKind::Identity),
                ConverterKind::BufferSource => quote!(::moli_webidl::SnapshotKind::BufferSource),
                _ => quote!(::moli_webidl::SnapshotKind::Scalar),
            }
        };
        let kind = if attrs.sequence {
            if attrs.snapshot_frozen {
                quote!(::moli_webidl::SnapshotKind::FrozenSequence(&#item_kind))
            } else {
                quote!(::moli_webidl::SnapshotKind::Sequence(&#item_kind))
            }
        } else {
            item_kind
        };
        let kind = if attrs.nullable {
            quote!(::moli_webidl::SnapshotKind::Nullable(&#kind))
        } else {
            kind
        };
        let member_name = field_member_name(field, &attrs, rename_all)?;
        let optional =
            is_option_type(&field.ty) && !attrs.nullable && attrs.snapshot_default.is_none();
        members.push(quote!(::moli_webidl::SnapshotMember { name: #member_name, kind: #kind, optional: #optional }));
        let item_value = if let Some(with) = &attrs.snapshot_with {
            quote!(#with(scope, value, writer.prototype))
        } else {
            match converter {
                ConverterKind::Dictionary => quote!(
                    ::moli_webidl::WebIdlDictionarySerialize::to_js_snapshot(
                        value,
                        scope,
                        writer.prototype
                    )
                    .map(::std::convert::Into::into)
                ),
                ConverterKind::Enum => {
                    quote!(::moli_webidl::__private::enum_snapshot_value(scope, value))
                }
                ConverterKind::DomString16 => quote!(
                    v8::String::new_from_two_byte(scope, value, v8::NewStringType::Normal)
                        .map(::std::convert::Into::into)
                ),
                ConverterKind::ByteString if vec_inner_type(ty).is_some() => quote!(
                    v8::String::new_from_one_byte(scope, value, v8::NewStringType::Normal)
                        .map(::std::convert::Into::into)
                ),
                ConverterKind::BufferSource => quote!(::std::option::Option::Some(
                    ::moli_webidl::__private::bytes_snapshot(
                        scope,
                        ::std::convert::AsRef::<[u8]>::as_ref(value)
                    )
                )),
                ConverterKind::CallbackFunction | ConverterKind::CallbackInterface => quote!({
                    let relevant = value
                        .relevant_context(scope)
                        .get_extras_binding_object(scope);
                    let incumbent = value
                        .incumbent_context(scope)
                        .get_extras_binding_object(scope);
                    writer.anchor_contexts(scope, &[relevant, incumbent])?;
                    ::std::option::Option::Some(value.value(scope))
                }),
                _ => quote!(::moli_webidl::__private::WebApiValue::to_v8_value(
                    value, scope
                )),
            }
        };
        let item_value = if let Some(with) = &attrs.snapshot_state_with {
            quote!(if writer.is_state() { #with(scope, value) } else { #item_value })
        } else {
            item_value
        };
        let value = if attrs.sequence {
            quote!({
                let mut items = ::std::vec::Vec::with_capacity(value.len());
                for value in value.iter() { items.push(#item_value?); }
                ::std::option::Option::Some(v8::Array::new_with_elements(scope, &items).into())
            })
        } else {
            item_value
        };
        let slot = match &attrs.snapshot_slot {
            Some(slot) => quote!(::std::option::Option::Some(#slot)),
            None => quote!(::std::option::Option::None),
        };
        let attributes = attrs
            .snapshot_attributes
            .as_ref()
            .map(|attributes| quote!(#attributes))
            .unwrap_or_else(|| quote!(v8::PropertyAttribute::NONE));
        let raw_copy = (matches!(converter, ConverterKind::Raw | ConverterKind::Uint8Array) && attrs.snapshot_with.is_none()).then(|| {
            if attrs.snapshot_state_with.is_some() {
                quote!(let value = if writer.is_state() { value } else { (#kind).copy(scope, value, writer.prototype)? };)
            } else {
                quote!(let value = (#kind).copy(scope, value, writer.prototype)?;)
            }
        });
        let freeze = attrs.snapshot_frozen.then(|| {
            quote! {
                let array = v8::Local::<v8::Array>::try_from(value).ok()?;
                array.set_integrity_level(scope, v8::IntegrityLevel::Frozen)?.then_some(())?;
            }
        });
        let write = quote! {
            let value: v8::Local<#lifetime, v8::Value> = #value?;
            #raw_copy
            #freeze
            writer.set(scope, #member_name, #slot, value, #attributes)?;
        };
        let write = if is_option_type(&field.ty) {
            let default = attrs
                .snapshot_default
                .as_ref()
                .map(|value| quote!(#value))
                .or_else(|| attrs.nullable.then(|| quote!(v8::null(scope).into())));
            let absent = default.map(|value| quote!(else { let value: v8::Local<#lifetime, v8::Value> = #value; writer.set(scope, #member_name, #slot, value, #attributes)?; }));
            quote!(if let ::std::option::Option::Some(value) = &self.#ident { #write } #absent)
        } else {
            quote!({ let value = &self.#ident; #write })
        };
        writes.push(write);
    }
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    let mut serialize_generics = generics.clone();
    if !serialize_generics
        .params
        .iter()
        .any(|param| matches!(param, GenericParam::Lifetime(param) if param.lifetime == *lifetime))
    {
        serialize_generics
            .params
            .insert(0, syn::parse_quote!(#lifetime));
    }
    let (serialize_impl, _, serialize_where) = serialize_generics.split_for_impl();
    Ok(quote! {
        impl #impl_generics ::moli_webidl::WebIdlDictionarySnapshot for #name #ty_generics #where_clause {
            const SNAPSHOT_SCHEMA: ::moli_webidl::SnapshotSchema = ::moli_webidl::SnapshotSchema { inherited: #inherited, members: &[#(#members),*] };
        }
        impl #serialize_impl ::moli_webidl::WebIdlDictionarySerialize<#lifetime> for #name #ty_generics #serialize_where {
            fn write_snapshot_own_fields(&self, scope: &mut v8::PinScope<#lifetime, '_>, writer: &::moli_webidl::SnapshotWriter<#lifetime>) -> ::std::option::Option<()> {
                #(#writes)*
                ::std::option::Option::Some(())
            }
            fn write_snapshot_fields(&self, scope: &mut v8::PinScope<#lifetime, '_>, writer: &::moli_webidl::SnapshotWriter<#lifetime>) -> ::std::option::Option<()> {
                #inherited_write
                ::moli_webidl::WebIdlDictionarySerialize::write_snapshot_own_fields(self, scope, writer)
            }
        }
    })
}
