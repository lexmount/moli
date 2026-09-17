use quote::quote;
use syn::parse_quote;
use syn::spanned::Spanned;
use syn::{Data, DeriveInput, Error, Field, Fields, GenericParam, LitStr};

use crate::attrs::{
    RenameRule, apply_rename_rule, default_required_arg_message, field_ident, field_member_name,
    parse_container_attrs, parse_enum_attrs, parse_field_attrs, parse_variant_attrs,
};
use crate::converter::{
    converter_kind, converter_kind_for_type, inner_type_for_field, is_option_type, vec_inner_type,
};

pub(crate) fn expand_webidl_args(input: DeriveInput) -> Result<proc_macro2::TokenStream, Error> {
    let struct_name = input.ident;
    let generics = input.generics.clone();
    let attrs = parse_container_attrs(&input.attrs)?;
    let prefix = attrs
        .prefix
        .unwrap_or_else(|| LitStr::new(&struct_name.to_string(), proc_macro2::Span::call_site()));
    let (impl_generics, ty_generics, where_clause, scope_lifetime) =
        impl_parts_for_scope(&generics, attrs.scope_lifetime.as_ref(), struct_name.span())?;
    let fields = named_fields(&input.data)?;

    // Arity is checked before any conversion, including custom parsers. A
    // missing later argument must not invoke an earlier argument's toString.
    let required_checks = fields
        .iter()
        .enumerate()
        .map(|(index, field)| {
            let attrs = parse_field_attrs(field)?;
            if !attrs.required {
                return Ok(quote! {});
            }
            let ident = field_ident(field)?;
            let index = attrs.index.unwrap_or(index) as i32;
            let message = attrs
                .missing_message
                .clone()
                .unwrap_or_else(|| default_required_arg_message(&prefix, &ident, &attrs));
            Ok(quote! {
                if args.length() <= #index {
                    return ::std::result::Result::Err(
                        ::moli_webidl::WebIdlError::custom_message(#message),
                    );
                }
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;
    let bindings = fields
        .iter()
        .enumerate()
        .map(|(index, field)| expand_args_field(field, index, fields.len(), &prefix))
        .collect::<Result<Vec<_>, _>>()?;
    let idents = fields
        .iter()
        .map(field_ident)
        .collect::<Result<Vec<_>, _>>()?;

    Ok(quote! {
        impl #impl_generics ::moli_webidl::WebIdlArguments<#scope_lifetime> for #struct_name #ty_generics #where_clause {
            fn parse_arguments(
                scope: &mut v8::PinScope<#scope_lifetime, '_>,
                args: &v8::FunctionCallbackArguments<#scope_lifetime>,
            ) -> ::std::result::Result<Self, ::moli_webidl::WebIdlError> {
                #(#required_checks)*
                #(#bindings)*
                ::std::result::Result::Ok(Self {
                    #(#idents),*
                })
            }
        }
    })
}

pub(crate) fn expand_webidl_dictionary(
    input: DeriveInput,
) -> Result<proc_macro2::TokenStream, Error> {
    let struct_name = input.ident;
    let generics = input.generics.clone();
    let attrs = parse_container_attrs(&input.attrs)?;
    let prefix = attrs
        .prefix
        .unwrap_or_else(|| LitStr::new(&struct_name.to_string(), proc_macro2::Span::call_site()));
    let (impl_generics, ty_generics, where_clause, scope_lifetime) =
        impl_parts_for_scope(&generics, attrs.scope_lifetime.as_ref(), struct_name.span())?;
    let rename_all = attrs.rename_all.unwrap_or(RenameRule::CamelCase);
    let fields = named_fields(&input.data)?;

    let bindings = fields
        .iter()
        .map(|field| expand_dictionary_field(field, &prefix, rename_all))
        .collect::<Result<Vec<_>, _>>()?;
    let idents = fields
        .iter()
        .map(field_ident)
        .collect::<Result<Vec<_>, _>>()?;

    Ok(quote! {
        impl #impl_generics ::moli_webidl::WebIdlDictionary<#scope_lifetime> for #struct_name #ty_generics #where_clause {
            fn parse_dictionary(
                scope: &mut v8::PinScope<#scope_lifetime, '_>,
                object: v8::Local<#scope_lifetime, v8::Object>,
            ) -> ::std::result::Result<Self, ::moli_webidl::WebIdlError> {
                #(#bindings)*
                ::std::result::Result::Ok(Self {
                    #(#idents),*
                })
            }
        }
    })
}

pub(crate) fn expand_webidl_enum(input: DeriveInput) -> Result<proc_macro2::TokenStream, Error> {
    let type_name = input.ident;
    let generics = input.generics.clone();
    let attrs = parse_enum_attrs(&input.attrs)?;
    let enum_name = attrs
        .name
        .unwrap_or_else(|| LitStr::new(&type_name.to_string(), proc_macro2::Span::call_site()));
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    let parse_body = if let Some(parse_with) = attrs.parse_with {
        quote!(#parse_with(value))
    } else {
        let variants = match &input.data {
            Data::Enum(data) => data.variants.iter().collect::<Vec<_>>(),
            _ => {
                return Err(Error::new(
                    type_name.span(),
                    "WebIdlEnum derive without parse_with requires an enum",
                ));
            }
        };
        let mut arms = Vec::with_capacity(variants.len());
        let mut seen = std::collections::BTreeMap::<String, proc_macro2::Span>::new();
        for variant in variants {
            if !matches!(variant.fields, Fields::Unit) {
                return Err(Error::new(
                    variant.span(),
                    "WebIdlEnum derive only supports unit variants unless parse_with is used",
                ));
            }
            let variant_attrs = parse_variant_attrs(variant)?;
            let mut tokens = variant_attrs.tokens;
            if tokens.is_empty() {
                let token = apply_rename_rule(&variant.ident.to_string(), attrs.rename_all);
                tokens.push(LitStr::new(&token, variant.ident.span()));
            }
            let ident = &variant.ident;
            for token in tokens {
                let value = token.value();
                if let Some(previous) = seen.insert(value.clone(), token.span()) {
                    let mut error = Error::new(
                        token.span(),
                        format!("duplicate WebIDL enum token `{value}`"),
                    );
                    error.combine(Error::new(previous, "previous token defined here"));
                    return Err(error);
                }
                arms.push(quote!(#token => ::std::option::Option::Some(Self::#ident),));
            }
        }
        quote! {
            match value {
                #(#arms)*
                _ => ::std::option::Option::None,
            }
        }
    };
    Ok(quote! {
        impl #impl_generics ::moli_webidl::WebIdlEnum for #type_name #ty_generics #where_clause {
            const NAME: &'static str = #enum_name;

            fn parse_token(value: &str) -> ::std::option::Option<Self> {
                #parse_body
            }
        }
    })
}

fn impl_parts_for_scope(
    generics: &syn::Generics,
    explicit_scope_lifetime: Option<&syn::Lifetime>,
    span: proc_macro2::Span,
) -> Result<
    (
        proc_macro2::TokenStream,
        proc_macro2::TokenStream,
        proc_macro2::TokenStream,
        syn::Lifetime,
    ),
    Error,
> {
    if let Some(lifetime) = explicit_scope_lifetime {
        if !generics.params.iter().any(
            |param| matches!(param, GenericParam::Lifetime(param) if param.lifetime == *lifetime),
        ) {
            return Err(Error::new(
                lifetime.span(),
                "#[webidl(scope_lifetime = ...)] must name a lifetime parameter on the struct",
            ));
        }
        let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
        return Ok((
            quote!(#impl_generics),
            quote!(#ty_generics),
            quote!(#where_clause),
            lifetime.clone(),
        ));
    }

    let mut lifetimes = generics.params.iter().filter_map(|param| match param {
        GenericParam::Lifetime(param) => Some(param.lifetime.clone()),
        _ => None,
    });
    match (lifetimes.next(), lifetimes.next()) {
        (Some(lifetime), None) => {
            let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
            Ok((
                quote!(#impl_generics),
                quote!(#ty_generics),
                quote!(#where_clause),
                lifetime,
            ))
        }
        (Some(_), Some(_)) => Err(Error::new(
            span,
            "WebIDL derives with multiple lifetimes require #[webidl(scope_lifetime = '...)]",
        )),
        (None, None) => {
            let scope_lifetime: syn::Lifetime = parse_quote!('s);
            let mut impl_generics = generics.clone();
            impl_generics.params.insert(0, parse_quote!('s));
            let (impl_generics, _, _) = impl_generics.split_for_impl();
            let (_, ty_generics, where_clause) = generics.split_for_impl();
            Ok((
                quote!(#impl_generics),
                quote!(#ty_generics),
                quote!(#where_clause),
                scope_lifetime,
            ))
        }
        (None, Some(_)) => unreachable!("a second lifetime cannot exist without a first lifetime"),
    }
}

fn named_fields(data: &Data) -> Result<Vec<Field>, Error> {
    match data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => Ok(fields.named.iter().cloned().collect()),
            _ => Err(Error::new(
                data.struct_token.span(),
                "expected a struct with named fields",
            )),
        },
        _ => Err(Error::new(
            proc_macro2::Span::call_site(),
            "expected a struct",
        )),
    }
}

fn expand_args_field(
    field: &Field,
    index: usize,
    field_count: usize,
    prefix: &LitStr,
) -> Result<proc_macro2::TokenStream, Error> {
    let ident = field_ident(field)?;
    let attrs = parse_field_attrs(field)?;
    if attrs.variadic {
        // Variadic arguments represent the rest parameter tail. They cannot be
        // required, nullable, defaulted, or custom-parsed because the generated
        // code owns the loop over remaining native-binding arguments.
        if attrs.with.is_some()
            || attrs.required
            || attrs.default.is_some()
            || attrs.nullable
            || attrs.legacy_nullish
        {
            return Err(Error::new(
                field.span(),
                "variadic WebIdlArgs fields cannot be combined with required/default/nullable/legacy_nullish/with",
            ));
        }
        if index + 1 != field_count {
            return Err(Error::new(
                field.span(),
                "variadic WebIdlArgs fields must be the last field",
            ));
        }
        let Some(item_type) = vec_inner_type(&field.ty) else {
            return Err(Error::new(
                field.span(),
                "variadic WebIdlArgs fields must use Vec<T>",
            ));
        };
        let converter = if let Some(converter) = attrs.converter {
            converter
        } else {
            converter_kind_for_type(item_type).ok_or_else(|| {
                Error::new(
                    field.span(),
                    "could not infer variadic item converter; add #[webidl(converter = \"...\")]",
                )
            })?
        };
        let arg_index = attrs.index.unwrap_or(index) as i32;
        let converter_ty = converter.wrapper_type(item_type);
        let unwrap_value = converter.unwrap_value(quote!(value));
        let options = converter.options_expr(&attrs)?;
        return Ok(quote! {
            let mut #ident = ::std::vec::Vec::new();
            for variadic_index in #arg_index..args.length() {
                let variadic_ordinal = variadic_index as usize + 1;
                let value = ::moli_webidl::argument_with_options::<#converter_ty>(
                    scope,
                    args,
                    variadic_index,
                    ::moli_webidl::Context::argument(#prefix, variadic_ordinal),
                    &#options,
                )?;
                #ident.push(#unwrap_value);
            }
        });
    }
    if let Some(with) = attrs.with.as_ref() {
        // Custom parsers own conversion after the required-arity preflight.
        // They receive the raw V8 argument list and resolved index so APIs can preserve
        // browser-specific ordering or validation without fighting generated
        // scalar conversion.
        let arg_index = attrs.index.unwrap_or(index) as i32;
        return Ok(quote! {
            let #ident = #with(scope, args, #arg_index)?;
        });
    }
    let converter = converter_kind(field, &attrs)?;
    let arg_index = attrs.index.unwrap_or(index) as i32;
    let ordinal = usize::try_from(arg_index).unwrap_or(0) + 1;
    let field_type = inner_type_for_field(field);
    let converter_ty = converter.wrapper_type(field_type);
    let unwrap_value = converter.unwrap_value(quote!(value));
    let options = converter.options_expr(&attrs)?;
    let context = quote!(::moli_webidl::Context::argument(#prefix, #ordinal));
    if attrs.nullable && !is_option_type(&field.ty) {
        return Err(Error::new(
            field.span(),
            "nullable WebIdlArgs fields must use Option<T>",
        ));
    }

    let binding = if attrs.nullable {
        // `nullable` models `T?`, so the Rust field must be `Option<T>`. Required
        // nullable arguments still require the argument position to exist, but
        // `null`/`undefined` become `None` once the argument is present.
        if attrs.required {
            quote! {
                let #ident = {
                    let raw = args.get(#arg_index);
                    if raw.is_null() || raw.is_undefined() {
                        ::std::option::Option::None
                    } else {
                        let value = ::moli_webidl::convert_with_options::<#converter_ty>(
                            scope,
                            raw,
                            #context,
                            &#options,
                        )?;
                        ::std::option::Option::Some(#unwrap_value)
                    }
                };
            }
        } else {
            quote! {
                let #ident = if args.length() <= #arg_index || args.get(#arg_index).is_undefined() {
                    ::std::option::Option::None
                } else {
                    let raw = args.get(#arg_index);
                    if raw.is_null() {
                        ::std::option::Option::None
                    } else {
                        let value = ::moli_webidl::convert_with_options::<#converter_ty>(
                            scope,
                            raw,
                            #context,
                            &#options,
                        )?;
                        ::std::option::Option::Some(#unwrap_value)
                    }
                };
            }
        }
    } else if is_option_type(&field.ty) {
        // `Option<T>` without `nullable` is an optional argument: missing and
        // `undefined` skip conversion, while `null` is converted as a real value.
        quote! {
            let #ident = if args.length() <= #arg_index || args.get(#arg_index).is_undefined() {
                ::std::option::Option::None
            } else {
                let value = ::moli_webidl::argument_with_options::<#converter_ty>(
                    scope,
                    args,
                    #arg_index,
                    #context,
                    &#options,
                )?;
                ::std::option::Option::Some(#unwrap_value)
            };
        }
    } else if attrs.required {
        quote! {
            let #ident = {
                let value = ::moli_webidl::argument_with_options::<#converter_ty>(
                    scope,
                    args,
                    #arg_index,
                    #context,
                    &#options,
                )?;
                #unwrap_value
            };
        }
    } else {
        let default = attrs.default.ok_or_else(|| {
            Error::new(
                field.span(),
                "optional WebIdlArgs fields require #[webidl(default = ...)]",
            )
        })?;
        let wrapped_default = converter.wrap_default(default);
        let unwrap_default = converter.unwrap_value(wrapped_default);
        quote! {
            let #ident = {
                if args.length() <= #arg_index || args.get(#arg_index).is_undefined() {
                    #unwrap_default
                } else {
                    let value = ::moli_webidl::argument_with_options::<#converter_ty>(
                        scope,
                        args,
                        #arg_index,
                        #context,
                        &#options,
                    )?;
                    #unwrap_value
                }
            };
        }
    };

    Ok(binding)
}

fn expand_dictionary_field(
    field: &Field,
    prefix: &LitStr,
    rename_all: RenameRule,
) -> Result<proc_macro2::TokenStream, Error> {
    let ident = field_ident(field)?;
    let attrs = parse_field_attrs(field)?;
    if let Some(with) = attrs.with.as_ref() {
        // Custom dictionary parsers receive the resolved member name and own all
        // reads/conversion for the field. Use this for APIs whose WebIDL order
        // or validation depends on multiple members.
        let name = field_member_name(field, &attrs, rename_all)?;
        return Ok(quote! {
            let #ident = #with(scope, object, #name)?;
        });
    }
    let converter = converter_kind(field, &attrs)?;
    let name = field_member_name(field, &attrs, rename_all)?;
    let converter_ty = converter.wrapper_type(inner_type_for_field(field));
    let unwrap_value = converter.unwrap_value(quote!(value));
    let options = converter.options_expr(&attrs)?;
    let context = quote!(::moli_webidl::Context::member(#prefix, #name));
    let optional_member_fn = if attrs.legacy_nullish {
        // Legacy dictionary members intentionally treat both `undefined` and
        // `null` as absent. This is not the default WebIDL optional-member path.
        quote!(::moli_webidl::legacy_optional_member_with_options)
    } else {
        quote!(::moli_webidl::optional_member_with_options)
    };
    let optional_member_or_fn = if attrs.legacy_nullish {
        quote!(::moli_webidl::legacy_optional_member_or_with_options)
    } else {
        quote!(::moli_webidl::optional_member_or_with_options)
    };

    if attrs.nullable && !is_option_type(&field.ty) {
        return Err(Error::new(
            field.span(),
            "nullable WebIdlDictionary fields must use Option<T>",
        ));
    }

    if attrs.nullable {
        if attrs.required {
            return Ok(quote! {
                let #ident = match ::moli_webidl::property_result(
                    scope,
                    object,
                    #name,
                    #context,
                )? {
                    ::std::option::Option::Some(raw) if raw.is_null() => {
                        ::std::option::Option::None
                    }
                    ::std::option::Option::Some(raw) if raw.is_undefined() => {
                        return ::std::result::Result::Err(
                            ::moli_webidl::WebIdlError::missing_required(
                                #context,
                            ),
                        );
                    }
                    ::std::option::Option::Some(raw) => {
                        let value = ::moli_webidl::convert_with_options::<#converter_ty>(
                            scope,
                            raw,
                            #context,
                            &#options,
                        )?;
                        ::std::option::Option::Some(#unwrap_value)
                    }
                    ::std::option::Option::None => {
                        return ::std::result::Result::Err(
                            ::moli_webidl::WebIdlError::missing_required(
                                #context,
                            ),
                        );
                    }
                };
            });
        }
        return Ok(quote! {
            let #ident = match ::moli_webidl::property_result(
                scope,
                object,
                #name,
                #context,
            )? {
                ::std::option::Option::Some(raw) if raw.is_null() || raw.is_undefined() => {
                    ::std::option::Option::None
                }
                ::std::option::Option::Some(raw) => {
                    let value = ::moli_webidl::convert_with_options::<#converter_ty>(
                        scope,
                        raw,
                        #context,
                        &#options,
                    )?;
                    ::std::option::Option::Some(#unwrap_value)
                }
                ::std::option::Option::None => ::std::option::Option::None,
            };
        });
    }

    if is_option_type(&field.ty) {
        return Ok(quote! {
            let #ident = #optional_member_fn::<#converter_ty>(
                scope,
                object,
                #name,
                #context,
                &#options,
            )?
            .map(|value| #unwrap_value);
        });
    }

    if attrs.required {
        return Ok(quote! {
            let #ident = match #optional_member_fn::<#converter_ty>(
                scope,
                object,
                #name,
                #context,
                &#options,
            )? {
                ::std::option::Option::Some(value) => #unwrap_value,
                ::std::option::Option::None => {
                    return ::std::result::Result::Err(
                        ::moli_webidl::WebIdlError::missing_required(
                            #context,
                        ),
                    );
                }
            };
        });
    }

    let default = attrs.default.ok_or_else(|| {
        Error::new(
            field.span(),
            "WebIdlDictionary fields currently require #[webidl(required)] or #[webidl(default = ...)]",
        )
    })?;
    let wrapped_default = converter.wrap_default(default);
    Ok(quote! {
        let #ident = {
            let value = #optional_member_or_fn::<#converter_ty>(
                scope,
                object,
                #name,
                #context,
                #wrapped_default,
                &#options,
            )?;
            #unwrap_value
        };
    })
}

#[cfg(test)]
mod tests {
    use super::impl_parts_for_scope;
    use syn::parse_quote;

    #[test]
    fn scope_lifetime_is_inferred_for_single_lifetime_structs() {
        let generics: syn::Generics = parse_quote!(<'scope>);

        let (_, _, _, lifetime) =
            impl_parts_for_scope(&generics, None, proc_macro2::Span::call_site()).unwrap();

        assert_eq!(lifetime.ident, "scope");
    }

    #[test]
    fn scope_lifetime_is_synthesized_for_owned_structs() {
        let generics: syn::Generics = parse_quote!(<T>);

        let (impl_generics, ty_generics, _, lifetime) =
            impl_parts_for_scope(&generics, None, proc_macro2::Span::call_site()).unwrap();

        assert_eq!(lifetime.ident, "s");
        assert!(impl_generics.to_string().contains("'s"));
        assert!(!ty_generics.to_string().contains("'s"));
    }

    #[test]
    fn multiple_lifetimes_require_explicit_scope_lifetime() {
        let generics: syn::Generics = parse_quote!(<'scope, 'data>);

        let error = match impl_parts_for_scope(&generics, None, proc_macro2::Span::call_site()) {
            Ok(_) => panic!("multiple lifetimes should require scope_lifetime"),
            Err(error) => error,
        };

        assert!(
            error
                .to_string()
                .contains("multiple lifetimes require #[webidl(scope_lifetime")
        );
    }

    #[test]
    fn explicit_scope_lifetime_is_accepted_for_multi_lifetime_structs() {
        let generics: syn::Generics = parse_quote!(<'scope, 'data>);
        let explicit: syn::Lifetime = parse_quote!('scope);

        let (_, _, _, lifetime) =
            impl_parts_for_scope(&generics, Some(&explicit), proc_macro2::Span::call_site())
                .unwrap();

        assert_eq!(lifetime.ident, "scope");
    }
}
