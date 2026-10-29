use super::*;

pub(crate) fn expand_webapi_cross_origin(
    input: DeriveInput,
) -> Result<proc_macro2::TokenStream, Error> {
    let name = &input.ident;
    if !input.generics.params.is_empty() {
        return Err(Error::new_spanned(
            &input.generics,
            "cross-origin declarations cannot carry runtime state",
        ));
    }
    let attrs = parse_object_attrs(&input.attrs)?;
    if !matches!(attrs.role, None | Some(ObjectRole::Plain))
        || attrs.prototype.is_some()
        || attrs.own_to_string_tag.is_some()
        || attrs.fallback_to_string_tag.is_some()
        || attrs.scope_lifetime.is_some()
        || attrs.default_data_properties
        || attrs.default_enumerable
        || attrs.receiver_access_check
    {
        return Err(Error::new_spanned(
            input,
            "cross-origin declarations describe stateless property tables, not object initialization",
        ));
    }
    let fields = declared_fields(
        &input.data,
        attrs.rename_all,
        FieldDefaults {
            receiver: attrs.receiver.as_ref(),
            ..FieldDefaults::default()
        },
    )?;
    let mut helpers = Vec::new();
    let mut entries = Vec::new();
    for DeclaredField { field, attrs } in fields {
        if !type_is_unit(&field.ty)
            || attrs.data.is_some()
            || attrs.setter_data.is_some()
            || attrs.returns_promise
            || attrs.symbol.is_some()
            || attrs.getter_value.is_some()
            || attrs.receiver_access_check.is_some()
            || attrs.value.is_some()
            || attrs.init.is_some()
            || attrs.enumerable
            || attrs.dont_delete
        {
            return Err(Error::new_spanned(
                field,
                "cross-origin members must be stateless string-keyed methods or accessors",
            ));
        }
        let property_name = match attrs.name.as_ref() {
            Some(syn::Expr::Lit(syn::ExprLit {
                lit: Lit::Str(name),
                ..
            })) => name.clone(),
            _ => {
                return Err(Error::new_spanned(
                    &field,
                    "cross-origin member names must be string literals",
                ));
            }
        };
        let ident = field.ident.as_ref().expect("named field");
        let receiver_check = attrs.receiver.as_ref().map(|receiver| {
            quote! {
                if !#receiver(__moli_scope, __moli_receiver) {
                    ::moli_webapi_declare::__private::throw_illegal_invocation(__moli_scope);
                    return None;
                }
            }
        });
        let function = |suffix: &str,
                        callback: proc_macro2::TokenStream,
                        length: i32,
                        function_name: LitStr| {
            let factory = format_ident!("__cross_origin_{}_{}_template", ident, suffix);
            let helper = quote! {
                fn #factory<'s>(
                    scope: &::moli_webapi_declare::v8::PinScope<'s, '_>,
                ) -> ::moli_webapi_declare::v8::Local<'s, ::moli_webapi_declare::v8::FunctionTemplate> {
                    let template = ::moli_webapi_declare::v8::FunctionTemplate::builder(#callback)
                        .length(#length)
                        .constructor_behavior(::moli_webapi_declare::v8::ConstructorBehavior::Throw)
                        .build(scope);
                    template.remove_prototype();
                    template.set_class_name(::moli_webapi_declare::__private::v8str(scope, #function_name));
                    template
                }
            };
            (
                helper,
                quote!(::moli_webapi_declare::CrossOriginFunction { build: Self::#factory }),
            )
        };
        match attrs.kind {
            FieldKind::Method => {
                let callback = attrs
                    .callback
                    .as_ref()
                    .ok_or_else(|| Error::new_spanned(&field, "method requires callback"))?;
                let callback = expand_callback(callback, &attrs, false);
                let (helper, function) = function(
                    "method",
                    callback,
                    attrs.length.unwrap_or(0),
                    property_name.clone(),
                );
                helpers.push(helper);
                entries.push(quote! {
                    ::moli_webapi_declare::CrossOriginProperty::Method { name: #property_name, function: #function }
                });
            }
            FieldKind::AccessorProperty => {
                if attrs.getter.is_none() && attrs.setter.is_none() {
                    return Err(Error::new_spanned(
                        field,
                        "cross-origin accessor requires a getter or setter",
                    ));
                }
                let mut accessors = Vec::new();
                for (suffix, callback, length) in [
                    ("get", attrs.getter.as_ref(), 0),
                    ("set", attrs.setter.as_ref(), 1),
                ] {
                    let Some(callback) = callback else {
                        accessors.push(quote!(None));
                        continue;
                    };
                    let value_helper = format_ident!("__cross_origin_{}_{}", ident, suffix);
                    let callback_helper =
                        format_ident!("__cross_origin_{}_{}_callback", ident, suffix);
                    let function_name = LitStr::new(
                        &format!("{suffix} {}", property_name.value()),
                        property_name.span(),
                    );
                    let (helper, function) = function(
                        suffix,
                        quote!(Self::#callback_helper),
                        length,
                        function_name,
                    );
                    helpers.push(helper);
                    if length == 0 {
                        helpers.push(quote! {
                            fn #value_helper<'s>(
                                __moli_scope: &mut ::moli_webapi_declare::v8::PinScope<'s, '_>,
                                __moli_receiver: ::moli_webapi_declare::v8::Local<'s, ::moli_webapi_declare::v8::Object>,
                            ) -> Option<::moli_webapi_declare::v8::Local<'s, ::moli_webapi_declare::v8::Value>> {
                                #receiver_check
                                #callback(__moli_scope, __moli_receiver)
                            }
                            fn #callback_helper<'s>(
                                __moli_scope: &mut ::moli_webapi_declare::v8::PinScope<'s, '_>,
                                args: ::moli_webapi_declare::v8::FunctionCallbackArguments<'s>,
                                mut rv: ::moli_webapi_declare::v8::ReturnValue<'s>,
                            ) {
                                if let Some(value) = Self::#value_helper(__moli_scope, args.this()) { rv.set(value); }
                            }
                        });
                    } else {
                        helpers.push(quote! {
                            fn #value_helper<'s>(
                                __moli_scope: &mut ::moli_webapi_declare::v8::PinScope<'s, '_>,
                                __moli_receiver: ::moli_webapi_declare::v8::Local<'s, ::moli_webapi_declare::v8::Object>,
                                value: ::moli_webapi_declare::v8::Local<'s, ::moli_webapi_declare::v8::Value>,
                            ) -> Option<()> {
                                #receiver_check
                                #callback(__moli_scope, __moli_receiver, value)
                            }
                            fn #callback_helper<'s>(
                                __moli_scope: &mut ::moli_webapi_declare::v8::PinScope<'s, '_>,
                                args: ::moli_webapi_declare::v8::FunctionCallbackArguments<'s>,
                                _rv: ::moli_webapi_declare::v8::ReturnValue<'s>,
                            ) {
                                let _ = Self::#value_helper(__moli_scope, args.this(), args.get(0));
                            }
                        });
                    }
                    accessors.push(quote!(Some((Self::#value_helper, #function))));
                }
                let getter = &accessors[0];
                let setter = &accessors[1];
                entries.push(quote! {
                    ::moli_webapi_declare::CrossOriginProperty::Accessor {
                        name: #property_name, getter: #getter, setter: #setter,
                    }
                });
            }
            _ => {
                return Err(Error::new_spanned(
                    &field,
                    "cross-origin declarations only support method and accessor_property",
                ));
            }
        }
    }
    Ok(quote! {
        impl #name { #(#helpers)* }
        impl ::moli_webapi_declare::WebApiCrossOriginDeclaration for #name {
            fn cross_origin_properties() -> &'static [::moli_webapi_declare::CrossOriginProperty] {
                &[#(#entries),*]
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_declarations_that_would_capture_runtime_state() {
        let invalid: [DeriveInput; 3] = [
            syn::parse_quote! {
                #[webapi(plain)]
                struct Bad<T> { #[webapi(method, callback = callback)] operation: T }
            },
            syn::parse_quote! {
                #[webapi(plain)]
                struct Bad { #[webapi(method, callback = callback, data = self.state)] operation: (), state: usize }
            },
            syn::parse_quote! {
                #[webapi(plain)]
                struct Bad { #[webapi(method, name = runtime_name(), callback = callback)] operation: () }
            },
        ];
        for input in invalid {
            assert!(expand_webapi_cross_origin(input).is_err());
        }
    }
}
