use std::collections::HashMap;

use quote::{format_ident, quote};
use syn::spanned::Spanned;
use syn::{Data, DeriveInput, Error, Field, Fields, GenericParam, Ident, Lit, LitStr, Type};

use crate::attrs::{
    ConstructorAttr, ConstructorDefaultAttr, FunctionTemplateAttrs, ObjectAttrs, RenameRule,
    ValueInitAttr, parse_field_attrs, parse_function_template_attrs, parse_interface_attrs,
    parse_object_attrs,
};

struct WebApiFieldKey {
    display_name: proc_macro2::TokenStream,
    display_name_literal: Option<LitStr>,
    property_key: proc_macro2::TokenStream,
    function_name: Option<proc_macro2::TokenStream>,
}

pub(crate) fn expand_webapi_interface(
    input: DeriveInput,
) -> Result<proc_macro2::TokenStream, Error> {
    let struct_name = input.ident;
    let generics = input.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    let attrs = parse_interface_attrs(&input.attrs)?;
    let interface_name = attrs
        .name
        .unwrap_or_else(|| LitStr::new(&struct_name.to_string(), struct_name.span()));
    let parent = match attrs.parent {
        Some(parent) => quote!(::std::option::Option::Some(#parent)),
        None => quote!(::std::option::Option::None),
    };
    let constructor_callback = match attrs
        .constructor
        .clone()
        .unwrap_or(ConstructorAttr::Illegal)
    {
        ConstructorAttr::Illegal => {
            quote!(::moli_webapi_declare::illegal_constructor_callback)
        }
        ConstructorAttr::Callback(callback) => quote!(#callback),
    };
    let constructor_length = attrs.constructor_length.unwrap_or(0);
    let fields = named_fields(&input.data)?;
    let methods = fields
        .iter()
        .filter_map(|field| expand_interface_field(field, attrs.rename_all))
        .collect::<Result<Vec<_>, _>>()?;

    let body = quote! {
        const INTERFACE_NAME: &'static str = #interface_name;
        const PARENT_INTERFACE: ::std::option::Option<&'static str> = #parent;

        let prototype = ::moli_webapi_declare::v8::Object::new(scope);
        if let ::std::option::Option::Some(parent) = PARENT_INTERFACE {
            if let ::std::option::Option::Some(parent_prototype) =
                ::moli_webapi_declare::__private::global_constructor_prototype(scope, parent)
            {
                let _ = prototype.set_prototype(scope, parent_prototype.into());
            }
        }
        #(#methods)*
        let constructor = ::moli_webapi_declare::v8::Function::builder(#constructor_callback)
            .length(#constructor_length)
            .build(scope)
            .ok_or_else(|| {
                ::moli_webapi_declare::BindError::new(
                    ::std::format!("failed to build `{}` constructor", INTERFACE_NAME)
                )
            })?;
        constructor.set_name(::moli_webapi_declare::__private::v8str(scope, INTERFACE_NAME));
        ::moli_webapi_declare::define_interface_prototype_property(
            scope,
            constructor,
            prototype,
        )?;
        ::moli_webapi_declare::define_interface_constructor_property(
            scope,
            prototype,
            constructor,
        )?;
        ::moli_webapi_declare::define_to_string_tag(scope, prototype, INTERFACE_NAME);
        global
            .define_own_property(
                scope,
                ::moli_webapi_declare::__private::v8str(scope, INTERFACE_NAME).into(),
                constructor.into(),
                ::moli_webapi_declare::v8::PropertyAttribute::DONT_ENUM,
            )
            .unwrap_or(false)
            .then_some(())
            .ok_or_else(|| {
                ::moli_webapi_declare::BindError::new(
                    ::std::format!("failed to define `{}` constructor", INTERFACE_NAME)
                )
            })?;
        ::std::result::Result::Ok(constructor)
    };

    Ok(quote! {
        impl #impl_generics #struct_name #ty_generics #where_clause {
            pub fn bind<'s>(
                &self,
                scope: &mut ::moli_webapi_declare::v8::PinScope<'s, '_>,
                global: ::moli_webapi_declare::v8::Local<'s, ::moli_webapi_declare::v8::Object>,
            ) -> ::std::result::Result<
                ::moli_webapi_declare::v8::Local<'s, ::moli_webapi_declare::v8::Function>,
                ::moli_webapi_declare::BindError,
            > {
                #body
            }
        }

        impl #impl_generics ::moli_webapi_declare::WebApiInterfaceDeclaration for #struct_name #ty_generics #where_clause {
            const NAME: &'static str = #interface_name;
            const PARENT: ::std::option::Option<&'static str> = #parent;

            fn bind<'s>(
                &self,
                scope: &mut ::moli_webapi_declare::v8::PinScope<'s, '_>,
                global: ::moli_webapi_declare::v8::Local<'s, ::moli_webapi_declare::v8::Object>,
            ) -> ::std::result::Result<
                ::moli_webapi_declare::v8::Local<'s, ::moli_webapi_declare::v8::Function>,
                ::moli_webapi_declare::BindError,
            > {
                self.bind(scope, global)
            }
        }
    })
}

pub(crate) fn expand_webapi_function_template(
    input: DeriveInput,
) -> Result<proc_macro2::TokenStream, Error> {
    let struct_name = input.ident;
    let generics = input.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    let attrs = parse_function_template_attrs(&input.attrs)?;
    let template_name = attrs
        .name
        .clone()
        .unwrap_or_else(|| LitStr::new(&struct_name.to_string(), struct_name.span()));
    let constructor_callback = match attrs
        .constructor
        .clone()
        .unwrap_or(ConstructorAttr::Illegal)
    {
        ConstructorAttr::Illegal => {
            quote!(::moli_webapi_declare::illegal_constructor_callback)
        }
        ConstructorAttr::Callback(callback) => quote!(#callback),
    };
    let constructor_length = attrs.constructor_length.unwrap_or(0);
    let initialize_intrinsic_prototype_parent =
        attrs.intrinsic_prototype_parent.as_ref().map(|intrinsic| {
            quote! {
                let intrinsic_parent =
                        ::moli_webapi_declare::v8::FunctionTemplate::builder(
                            ::moli_webapi_declare::illegal_constructor_callback,
                        )
                        .constructor_behavior(
                            ::moli_webapi_declare::v8::ConstructorBehavior::Throw,
                        )
                        .build(scope);
                intrinsic_parent.remove_prototype();
                intrinsic_parent.set_intrinsic_data_property(
                    ::moli_webapi_declare::__private::v8str(scope, "prototype").into(),
                    #intrinsic,
                    ::moli_webapi_declare::v8::PropertyAttribute::NONE,
                );
                template.inherit(intrinsic_parent);
            }
        });
    let initialize_readonly_prototype = attrs.readonly_prototype.then(|| {
        quote! {
            template.read_only_prototype();
        }
    });
    let initialize_prototype_to_string_tag = attrs.prototype_to_string_tag.as_ref().map(|tag| {
        quote! {
            prototype.set_with_attr(
                ::moli_webapi_declare::v8::Symbol::get_to_string_tag(scope).into(),
                ::moli_webapi_declare::__private::v8str(scope, #tag).into(),
                ::moli_webapi_declare::v8::PropertyAttribute::READ_ONLY
                    | ::moli_webapi_declare::v8::PropertyAttribute::DONT_ENUM,
            );
        }
    });
    let fields = named_fields(&input.data)?;
    let declaration_field_reads = fields.iter().filter_map(|field| {
        field.ident.as_ref().map(|ident| {
            quote! {
                let _ = &self.#ident;
            }
        })
    });
    let template_fields = expand_function_template_fields(&fields, &attrs, &template_name)?;
    let FunctionTemplateFieldExpansions {
        template_methods,
        prototype_methods,
    } = template_fields;

    Ok(quote! {
        impl #impl_generics #struct_name #ty_generics #where_clause {
            fn __webapi_mark_declaration_fields_used(&self) {
                #(#declaration_field_reads)*
            }

            pub fn build<'s>(
                scope: &mut ::moli_webapi_declare::v8::PinScope<'s, '_, ()>,
            ) -> ::moli_webapi_declare::v8::Local<'s, ::moli_webapi_declare::v8::FunctionTemplate> {
                let _ = Self::__webapi_mark_declaration_fields_used;
                let template = ::moli_webapi_declare::v8::FunctionTemplate::builder(#constructor_callback)
                    .length(#constructor_length)
                    .build(scope);
                template.set_class_name(::moli_webapi_declare::__private::v8str(scope, #template_name));
                Self::initialize_template(scope, template);
                let prototype = template.prototype_template(scope);
                Self::initialize_prototype_template(scope, prototype);
                template
            }

            pub fn initialize_template<'s>(
                scope: &mut ::moli_webapi_declare::v8::PinScope<'s, '_, ()>,
                template: ::moli_webapi_declare::v8::Local<'s, ::moli_webapi_declare::v8::FunctionTemplate>,
            ) {
                #initialize_intrinsic_prototype_parent
                #initialize_readonly_prototype
                #(#template_methods)*
            }

            pub fn initialize_prototype_template<'s>(
                scope: &mut ::moli_webapi_declare::v8::PinScope<'s, '_, ()>,
                prototype: ::moli_webapi_declare::v8::Local<'s, ::moli_webapi_declare::v8::ObjectTemplate>,
            ) {
                #initialize_prototype_to_string_tag
                #(#prototype_methods)*
            }
        }

        impl #impl_generics ::moli_webapi_declare::WebApiFunctionTemplateDeclaration for #struct_name #ty_generics #where_clause {
            const NAME: &'static str = #template_name;

            fn build<'s>(
                scope: &mut ::moli_webapi_declare::v8::PinScope<'s, '_, ()>,
            ) -> ::moli_webapi_declare::v8::Local<'s, ::moli_webapi_declare::v8::FunctionTemplate> {
                <#struct_name #ty_generics>::build(scope)
            }

            fn initialize_template<'s>(
                scope: &mut ::moli_webapi_declare::v8::PinScope<'s, '_, ()>,
                template: ::moli_webapi_declare::v8::Local<'s, ::moli_webapi_declare::v8::FunctionTemplate>,
            ) {
                <#struct_name #ty_generics>::initialize_template(scope, template)
            }

            fn initialize_prototype_template<'s>(
                scope: &mut ::moli_webapi_declare::v8::PinScope<'s, '_, ()>,
                prototype: ::moli_webapi_declare::v8::Local<'s, ::moli_webapi_declare::v8::ObjectTemplate>,
            ) {
                <#struct_name #ty_generics>::initialize_prototype_template(scope, prototype)
            }
        }
    })
}

pub(crate) fn expand_webapi_object(input: DeriveInput) -> Result<proc_macro2::TokenStream, Error> {
    let struct_name = input.ident;
    let generics = input.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    let attrs = parse_object_attrs(&input.attrs)?;
    let interface = attrs
        .interface
        .clone()
        .ok_or_else(|| Error::new(struct_name.span(), "missing #[webapi(interface = \"...\")]"))?;
    let has_explicit_prototype = attrs.prototype.is_some();
    let prototype = attrs.prototype.clone().unwrap_or_else(|| interface.clone());
    let own_to_string_tag = match attrs.own_to_string_tag.as_ref() {
        Some(tag) => quote!(::std::option::Option::Some(#tag)),
        None => quote!(::std::option::Option::None),
    };
    let fallback_to_string_tag = match attrs.fallback_to_string_tag.as_ref() {
        Some(tag) => quote!(::std::option::Option::Some(#tag)),
        None => quote!(::std::option::Option::None),
    };
    let define_own_to_string_tag = if attrs.readonly_to_string_tag {
        quote! {
            if let ::std::option::Option::Some(tag) = #own_to_string_tag {
                ::moli_webapi_declare::define_to_string_tag_with_attributes(
                    scope,
                    object,
                    tag,
                    ::moli_webapi_declare::v8::PropertyAttribute::DONT_ENUM
                        | ::moli_webapi_declare::v8::PropertyAttribute::READ_ONLY,
                );
            }
        }
    } else {
        quote! {
            if let ::std::option::Option::Some(tag) = #own_to_string_tag {
                ::moli_webapi_declare::define_to_string_tag(scope, object, tag);
            }
        }
    };
    let set_prototype = if attrs.require_prototype {
        quote! {
            ::moli_webapi_declare::set_required_interface_prototype(scope, object, #prototype)?;
            true
        }
    } else {
        quote! {
            ::moli_webapi_declare::set_interface_prototype(scope, object, #prototype)
        }
    };
    let define_fallback_to_string_tag = quote! {
        if !prototype_bound {
            if let ::std::option::Option::Some(tag) = #fallback_to_string_tag {
                ::moli_webapi_declare::define_to_string_tag(scope, object, tag);
            }
        }
    };
    let fields = named_fields(&input.data)?;
    let generated_constructor = expand_object_generated_constructor(&fields, &attrs, &struct_name)?;
    let inferred_scope_lifetime = attrs
        .scope_lifetime
        .clone()
        .or_else(|| single_lifetime_param(&generics));
    let scope_lifetime = inferred_scope_lifetime.as_ref();
    if let Some(lifetime) = scope_lifetime
        && !generics.params.iter().any(
            |param| matches!(param, GenericParam::Lifetime(param) if param.lifetime == *lifetime),
        )
    {
        return Err(Error::new(
            lifetime.span(),
            "#[webapi(scope_lifetime = ...)] must name a lifetime parameter on the struct",
        ));
    }
    let method_scope_lifetime = scope_lifetime
        .map(|lifetime| quote!(#lifetime))
        .unwrap_or_else(|| quote!('__webapi_scope));
    let method_scope_generic = scope_lifetime.is_none().then(|| quote!(<'__webapi_scope>));
    let fields = fields
        .iter()
        .filter_map(|field| expand_object_field(field, &attrs))
        .collect::<Result<Vec<_>, _>>()?;
    if fields.is_empty() && !attrs.allow_empty && !has_explicit_prototype {
        return Err(Error::new(
            struct_name.span(),
            "empty WebApiObject declaration requires #[webapi(allow_empty)]",
        ));
    }

    let initialize_body = quote! {
        #(#fields)*
        ::std::result::Result::Ok(())
    };
    let trait_impl = match scope_lifetime {
        Some(lifetime) => {
            quote! {
                impl #impl_generics ::moli_webapi_declare::WebApiObjectDeclaration<#lifetime> for #struct_name #ty_generics #where_clause {
                    const INTERFACE: &'static str = #interface;
                    const OWN_TO_STRING_TAG: ::std::option::Option<&'static str> = #own_to_string_tag;

                    fn initialize(
                        &self,
                        scope: &mut ::moli_webapi_declare::v8::PinScope<#lifetime, '_>,
                        object: ::moli_webapi_declare::v8::Local<#lifetime, ::moli_webapi_declare::v8::Object>,
                    ) -> ::std::result::Result<(), ::moli_webapi_declare::BindError> {
                        self.initialize(scope, object)
                    }

                    fn bind_into(
                        &self,
                        scope: &mut ::moli_webapi_declare::v8::PinScope<#lifetime, '_>,
                        object: ::moli_webapi_declare::v8::Local<#lifetime, ::moli_webapi_declare::v8::Object>,
                    ) -> ::std::result::Result<(), ::moli_webapi_declare::BindError> {
                        self.bind_into(scope, object)
                    }

                    fn bind(
                        &self,
                        scope: &mut ::moli_webapi_declare::v8::PinScope<#lifetime, '_>,
                    ) -> ::std::result::Result<
                        ::moli_webapi_declare::v8::Local<#lifetime, ::moli_webapi_declare::v8::Object>,
                        ::moli_webapi_declare::BindError,
                    > {
                        self.bind(scope)
                    }
                }
            }
        }
        None => {
            let mut trait_generics = generics.clone();
            trait_generics
                .params
                .insert(0, syn::parse_quote!('__webapi_scope));
            let (trait_impl_generics, _, trait_where_clause) = trait_generics.split_for_impl();
            quote! {
                impl #trait_impl_generics ::moli_webapi_declare::WebApiObjectDeclaration<'__webapi_scope> for #struct_name #ty_generics #trait_where_clause {
                    const INTERFACE: &'static str = #interface;
                    const OWN_TO_STRING_TAG: ::std::option::Option<&'static str> = #own_to_string_tag;

                    fn initialize(
                        &self,
                        scope: &mut ::moli_webapi_declare::v8::PinScope<'__webapi_scope, '_>,
                        object: ::moli_webapi_declare::v8::Local<'__webapi_scope, ::moli_webapi_declare::v8::Object>,
                    ) -> ::std::result::Result<(), ::moli_webapi_declare::BindError> {
                        self.initialize(scope, object)
                    }

                    fn bind_into(
                        &self,
                        scope: &mut ::moli_webapi_declare::v8::PinScope<'__webapi_scope, '_>,
                        object: ::moli_webapi_declare::v8::Local<'__webapi_scope, ::moli_webapi_declare::v8::Object>,
                    ) -> ::std::result::Result<(), ::moli_webapi_declare::BindError> {
                        self.bind_into(scope, object)
                    }

                    fn bind(
                        &self,
                        scope: &mut ::moli_webapi_declare::v8::PinScope<'__webapi_scope, '_>,
                    ) -> ::std::result::Result<
                        ::moli_webapi_declare::v8::Local<'__webapi_scope, ::moli_webapi_declare::v8::Object>,
                        ::moli_webapi_declare::BindError,
                    > {
                        self.bind(scope)
                    }
                }
            }
        }
    };
    let value_impl = match scope_lifetime {
        Some(lifetime) => {
            quote! {
                impl #impl_generics ::moli_webapi_declare::WebApiValue<#lifetime> for #struct_name #ty_generics #where_clause {
                    fn to_v8_value(
                        &self,
                        scope: &mut ::moli_webapi_declare::v8::PinScope<#lifetime, '_>,
                    ) -> ::std::option::Option<::moli_webapi_declare::v8::Local<#lifetime, ::moli_webapi_declare::v8::Value>> {
                        self.bind(scope).ok().map(::std::convert::Into::into)
                    }
                }
            }
        }
        None => {
            let mut trait_generics = generics.clone();
            trait_generics
                .params
                .insert(0, syn::parse_quote!('__webapi_scope));
            let (trait_impl_generics, _, trait_where_clause) = trait_generics.split_for_impl();
            quote! {
                impl #trait_impl_generics ::moli_webapi_declare::WebApiValue<'__webapi_scope> for #struct_name #ty_generics #trait_where_clause {
                    fn to_v8_value(
                        &self,
                        scope: &mut ::moli_webapi_declare::v8::PinScope<'__webapi_scope, '_>,
                    ) -> ::std::option::Option<::moli_webapi_declare::v8::Local<'__webapi_scope, ::moli_webapi_declare::v8::Value>> {
                        self.bind(scope).ok().map(::std::convert::Into::into)
                    }
                }
            }
        }
    };

    Ok(quote! {
        impl #impl_generics #struct_name #ty_generics #where_clause {
            #generated_constructor

            pub fn initialize #method_scope_generic(
                &self,
                scope: &mut ::moli_webapi_declare::v8::PinScope<#method_scope_lifetime, '_>,
                object: ::moli_webapi_declare::v8::Local<#method_scope_lifetime, ::moli_webapi_declare::v8::Object>,
            ) -> ::std::result::Result<(), ::moli_webapi_declare::BindError> {
                #initialize_body
            }

            pub fn bind_into #method_scope_generic(
                &self,
                scope: &mut ::moli_webapi_declare::v8::PinScope<#method_scope_lifetime, '_>,
                object: ::moli_webapi_declare::v8::Local<#method_scope_lifetime, ::moli_webapi_declare::v8::Object>,
            ) -> ::std::result::Result<(), ::moli_webapi_declare::BindError> {
                let prototype_bound = { #set_prototype };
                #define_own_to_string_tag
                #define_fallback_to_string_tag
                self.initialize(scope, object)
            }

            pub fn bind #method_scope_generic(
                &self,
                scope: &mut ::moli_webapi_declare::v8::PinScope<#method_scope_lifetime, '_>,
            ) -> ::std::result::Result<
                ::moli_webapi_declare::v8::Local<#method_scope_lifetime, ::moli_webapi_declare::v8::Object>,
                ::moli_webapi_declare::BindError,
            > {
                let object = ::moli_webapi_declare::v8::Object::new(scope);
                self.bind_into(scope, object)?;
                ::std::result::Result::Ok(object)
            }
        }

        #trait_impl
        #value_impl
    })
}

fn expand_object_generated_constructor(
    fields: &[Field],
    attrs: &ObjectAttrs,
    struct_name: &Ident,
) -> Result<proc_macro2::TokenStream, Error> {
    if attrs.no_dynamic_constructor {
        return Ok(quote!());
    }
    let mut parameters = Vec::new();
    let mut initializers = Vec::new();
    for field in fields {
        let ident = field
            .ident
            .as_ref()
            .ok_or_else(|| Error::new(field.span(), "WebApiObject field requires a name"))?;
        let field_attrs = parse_field_attrs(field)?;
        if type_is_unit(&field.ty) {
            initializers.push(quote!(#ident: ()));
        } else if let Some(default) = field_attrs.constructor_default {
            let default = match default {
                ConstructorDefaultAttr::Default => quote!(::std::default::Default::default()),
                ConstructorDefaultAttr::Expr(expr) => quote!(#expr),
            };
            initializers.push(quote!(#ident: #default));
        } else {
            let ty = &field.ty;
            parameters.push(quote!(#ident: #ty));
            initializers.push(quote!(#ident));
        }
    }
    let constructor = Ident::new("new", struct_name.span());
    Ok(quote! {
        pub fn #constructor(#(#parameters),*) -> Self {
            Self {
                #(#initializers),*
            }
        }
    })
}

struct FunctionTemplateFieldExpansions {
    template_methods: Vec<proc_macro2::TokenStream>,
    prototype_methods: Vec<proc_macro2::TokenStream>,
}

fn expand_function_template_fields(
    fields: &[Field],
    template_attrs: &FunctionTemplateAttrs,
    template_name: &LitStr,
) -> Result<FunctionTemplateFieldExpansions, Error> {
    let mut template_methods = Vec::new();
    let mut prototype_methods = Vec::new();
    let mut method_bindings = HashMap::new();
    for (index, field) in fields.iter().enumerate() {
        let mut attrs = parse_field_attrs(field)?;
        if template_attrs.default_enumerable
            && attrs.symbol.is_none()
            && (attrs.method
                || attrs.static_method
                || attrs.accessor_property
                || attrs.native_data_property
                || attrs.intrinsic_data_property.is_some()
                || attrs.alias.is_some())
        {
            attrs.enumerable = true;
        }
        if attrs.constant {
            template_methods.push(expand_function_template_constant_field(
                field,
                &attrs,
                template_attrs.rename_all,
                quote!(template),
            )?);
            prototype_methods.push(expand_function_template_constant_field(
                field,
                &attrs,
                template_attrs.rename_all,
                quote!(prototype),
            )?);
            continue;
        }
        if attrs.static_method {
            let binding = format_ident!("__webapi_template_static_method_{index}");
            template_methods.push(expand_function_template_static_method_field(
                field,
                &attrs,
                template_attrs.rename_all,
                template_name,
                binding,
            )?);
            continue;
        }
        if attrs.method {
            let binding = format_ident!("__webapi_template_method_{index}");
            if attrs.symbol.is_none()
                && let Some(name) = webapi_field_name_literal(
                    field,
                    attrs.name.as_ref(),
                    template_attrs.rename_all,
                )?
            {
                method_bindings.insert(name.value(), binding.clone());
            }
            prototype_methods.push(expand_function_template_method_field(
                field,
                &attrs,
                template_attrs.rename_all,
                template_name,
                binding,
            )?);
            continue;
        }
        if attrs.accessor_property {
            prototype_methods.push(expand_function_template_accessor_property_field(
                field,
                &attrs,
                template_attrs.rename_all,
                template_name,
            )?);
            continue;
        }
        if attrs.native_data_property {
            prototype_methods.push(expand_function_template_native_data_property_field(
                field,
                &attrs,
                template_attrs.rename_all,
            )?);
            continue;
        }
        if attrs.intrinsic_data_property.is_some() {
            prototype_methods.push(expand_function_template_intrinsic_data_property_field(
                field,
                &attrs,
                template_attrs.rename_all,
            )?);
            continue;
        }
        if attrs.alias.is_some() {
            prototype_methods.push(expand_function_template_alias_field(
                field,
                &attrs,
                template_attrs.rename_all,
                &method_bindings,
            )?);
            continue;
        }
        if attrs.has_installation_kind() || attrs.has_installation_attribute() {
            return Err(Error::new(
                field.span(),
                "function template fields with #[webapi(...)] attributes must declare #[webapi(method)], #[webapi(static_method)], #[webapi(accessor_property)], #[webapi(native_data_property)], #[webapi(intrinsic_data_property = ...)], #[webapi(constant)], or #[webapi(alias = ...)]",
            ));
        }
    }
    Ok(FunctionTemplateFieldExpansions {
        template_methods,
        prototype_methods,
    })
}

fn expand_function_template_constant_field(
    field: &Field,
    attrs: &crate::attrs::FieldAttrs,
    rename_all: RenameRule,
    template: proc_macro2::TokenStream,
) -> Result<proc_macro2::TokenStream, Error> {
    let key = webapi_field_key(field, attrs, rename_all)?;
    let name = key.display_name;
    let property_key = key.property_key;
    let Some(value) = attrs.value.as_ref() else {
        return Err(Error::new(
            field.span(),
            "constant field requires #[webapi(value = expr)]",
        ));
    };
    let attributes = constant_property_attributes();
    let field_read = field
        .ident
        .as_ref()
        .map(|ident| quote!(let _ = ::std::stringify!(#ident);));
    Ok(quote! {
        #field_read
        let __webapi_constant_value = (#value);
        let __webapi_constant_v8_value =
            ::moli_webapi_declare::WebApiTemplateValue::to_v8_template_value(
                &__webapi_constant_value,
                scope,
            )
            .expect("failed to convert Web API constant value");
        #template.set_with_attr(
            #property_key,
            __webapi_constant_v8_value.into(),
            #attributes,
        );
        let _ = #name;
    })
}

fn expand_function_template_static_method_field(
    field: &Field,
    attrs: &crate::attrs::FieldAttrs,
    rename_all: RenameRule,
    template_name: &LitStr,
    binding: syn::Ident,
) -> Result<proc_macro2::TokenStream, Error> {
    let key = webapi_field_key(field, attrs, rename_all)?;
    let name = key.display_name;
    let property_key = key.property_key;
    let set_function_name = key.function_name.map(|function_name| {
        quote! {
            #binding.set_class_name(#function_name);
        }
    });
    let Some(callback) = attrs.callback.as_ref() else {
        return Err(Error::new(
            field.span(),
            "static method field requires #[webapi(callback = path)]",
        ));
    };
    let length = attrs.length.unwrap_or(0);
    let attributes = template_property_attributes(attrs);
    let build_function = expand_template_function_builder(
        callback,
        length,
        attrs.data.as_ref(),
        template_name,
        &name,
    );
    let field_read = field
        .ident
        .as_ref()
        .map(|ident| quote!(let _ = ::std::stringify!(#ident);));
    Ok(quote! {
        #field_read
        let #binding = #build_function;
        #set_function_name
        template.set_with_attr(
            #property_key,
            #binding.into(),
            #attributes,
        );
        let _ = #name;
    })
}

fn expand_function_template_method_field(
    field: &Field,
    attrs: &crate::attrs::FieldAttrs,
    rename_all: RenameRule,
    template_name: &LitStr,
    binding: syn::Ident,
) -> Result<proc_macro2::TokenStream, Error> {
    let key = webapi_field_key(field, attrs, rename_all)?;
    let name = key.display_name;
    let property_key = key.property_key;
    let set_function_name = key.function_name.map(|function_name| {
        quote! {
            #binding.set_class_name(#function_name);
        }
    });
    let Some(callback) = attrs.callback.as_ref() else {
        return Err(Error::new(
            field.span(),
            "method field requires #[webapi(callback = path)]",
        ));
    };
    let length = attrs.length.unwrap_or(0);
    let attributes = template_property_attributes(attrs);
    let build_function = expand_template_function_builder(
        callback,
        length,
        attrs.data.as_ref(),
        template_name,
        &name,
    );
    let field_read = field
        .ident
        .as_ref()
        .map(|ident| quote!(let _ = ::std::stringify!(#ident);));
    Ok(quote! {
        #field_read
        let #binding = #build_function;
        #set_function_name
        prototype.set_with_attr(
            #property_key,
            #binding.into(),
            #attributes,
        );
        let _ = #name;
    })
}

fn expand_function_template_accessor_property_field(
    field: &Field,
    attrs: &crate::attrs::FieldAttrs,
    rename_all: RenameRule,
    template_name: &LitStr,
) -> Result<proc_macro2::TokenStream, Error> {
    if attrs.readonly {
        return Err(Error::new(
            field.span(),
            "function-template `accessor_property` fields have no writable attribute; omit #[webapi(setter)] instead of using readonly",
        ));
    }
    let key = webapi_field_key(field, attrs, rename_all)?;
    let getter_class_name = expand_template_accessor_class_name(
        &format_ident!("__webapi_getter"),
        &format_ident!("__webapi_getter_name"),
        "get",
        &key.display_name,
        key.display_name_literal.as_ref(),
    );
    let setter_class_name = expand_template_accessor_class_name(
        &format_ident!("__webapi_setter"),
        &format_ident!("__webapi_setter_name"),
        "set",
        &key.display_name,
        key.display_name_literal.as_ref(),
    );
    let name = key.display_name;
    let property_key = key.property_key;
    let Some(getter) = attrs.getter.as_ref() else {
        return Err(Error::new(
            field.span(),
            "`accessor_property` field requires #[webapi(getter = path)]",
        ));
    };
    let getter_builder = expand_template_accessor_function_builder(
        getter,
        0,
        attrs.data.as_ref(),
        template_name,
        &name,
    );
    let setter = attrs.setter.as_ref().map(|setter| {
        let setter_data = attrs.setter_data.as_ref().or(attrs.data.as_ref());
        let setter_builder =
            expand_template_accessor_function_builder(setter, 1, setter_data, template_name, &name);
        quote! {
            {
                let __webapi_setter = #setter_builder;
                #setter_class_name
                ::std::option::Option::Some(__webapi_setter)
            }
        }
    });
    let setter = setter.unwrap_or_else(|| quote!(::std::option::Option::None));
    let attributes = accessor_property_attributes(attrs);
    let field_read = field
        .ident
        .as_ref()
        .map(|ident| quote!(let _ = ::std::stringify!(#ident);));
    Ok(quote! {
        #field_read
        let __webapi_getter = #getter_builder;
        #getter_class_name
        let __webapi_setter = #setter;
        prototype.set_accessor_property(
            #property_key,
            ::std::option::Option::Some(__webapi_getter),
            __webapi_setter,
            #attributes,
        );
        let _ = #name;
    })
}

fn expand_template_accessor_class_name(
    binding: &Ident,
    variable: &Ident,
    prefix: &str,
    display_name: &proc_macro2::TokenStream,
    display_name_literal: Option<&LitStr>,
) -> proc_macro2::TokenStream {
    if let Some(display_name_literal) = display_name_literal {
        let class_name = LitStr::new(
            &format!("{prefix} {}", display_name_literal.value()),
            display_name_literal.span(),
        );
        return quote! {
            if let ::std::option::Option::Some(#variable) =
                ::moli_webapi_declare::v8::String::new(scope, #class_name)
            {
                #binding.set_class_name(#variable);
            }
        };
    }

    let format_string = LitStr::new(&format!("{prefix} {{}}"), proc_macro2::Span::call_site());
    quote! {
        let #variable = ::std::format!(#format_string, #display_name);
        if let ::std::option::Option::Some(#variable) =
            ::moli_webapi_declare::v8::String::new(scope, &#variable)
        {
            #binding.set_class_name(#variable);
        }
    }
}

fn expand_function_template_native_data_property_field(
    field: &Field,
    attrs: &crate::attrs::FieldAttrs,
    rename_all: RenameRule,
) -> Result<proc_macro2::TokenStream, Error> {
    if attrs.readonly {
        return Err(Error::new(
            field.span(),
            "function-template `native_data_property` fields have no writable attribute; omit #[webapi(setter)] instead of using readonly",
        ));
    }
    if attrs.setter_data.is_some() {
        return Err(Error::new(
            field.span(),
            "function-template `native_data_property` fields share one callback data value and cannot use #[webapi(setter_data = ...)]",
        ));
    }
    let key = webapi_field_key(field, attrs, rename_all)?;
    let name = key.display_name;
    let property_key = key.property_key;
    let Some(getter) = attrs.getter.as_ref() else {
        return Err(Error::new(
            field.span(),
            "`native_data_property` field requires #[webapi(getter = path)]",
        ));
    };
    let setter = attrs.setter.as_ref().map(|setter| {
        quote! {
            .setter(#setter)
        }
    });
    let data = attrs.data.as_ref().map(|data| {
        quote! {
            .data((#data).into())
        }
    });
    let attributes = accessor_property_attributes(attrs);
    let field_read = field
        .ident
        .as_ref()
        .map(|ident| quote!(let _ = ::std::stringify!(#ident);));
    Ok(quote! {
        #field_read
        let __webapi_native_data_property_configuration =
            ::moli_webapi_declare::v8::NativeDataPropertyConfiguration::new(#getter)
                #setter
                #data
                .property_attribute(#attributes);
        prototype.set_native_data_property_with_configuration(
            #property_key,
            __webapi_native_data_property_configuration,
        );
        let _ = #name;
    })
}

fn expand_function_template_intrinsic_data_property_field(
    field: &Field,
    attrs: &crate::attrs::FieldAttrs,
    rename_all: RenameRule,
) -> Result<proc_macro2::TokenStream, Error> {
    let key = webapi_field_key(field, attrs, rename_all)?;
    let name = key.display_name;
    let property_key = key.property_key;
    let Some(intrinsic) = attrs.intrinsic_data_property.as_ref() else {
        return Err(Error::new(
            field.span(),
            "function template intrinsic data property requires an intrinsic value",
        ));
    };
    let attributes = template_property_attributes(attrs);
    let field_read = field
        .ident
        .as_ref()
        .map(|ident| quote!(let _ = ::std::stringify!(#ident);));
    Ok(quote! {
        #field_read
        prototype.set_intrinsic_data_property(
            #property_key,
            #intrinsic,
            #attributes,
        );
        let _ = #name;
    })
}

fn expand_function_template_alias_field(
    field: &Field,
    attrs: &crate::attrs::FieldAttrs,
    rename_all: RenameRule,
    method_bindings: &HashMap<String, syn::Ident>,
) -> Result<proc_macro2::TokenStream, Error> {
    if attrs.callback.is_some()
        || attrs.getter.is_some()
        || attrs.setter.is_some()
        || attrs.function_name.is_some()
        || attrs.length.is_some()
        || attrs.data.is_some()
        || attrs.value.is_some()
        || attrs.init.is_some()
    {
        return Err(Error::new(
            field.span(),
            "function template alias fields cannot define callbacks, function metadata, data, or values",
        ));
    }
    let Some(source_name) = attrs.alias.as_ref() else {
        return Err(Error::new(
            field.span(),
            "function template alias field requires a source name",
        ));
    };
    let Some(source_binding) = method_bindings.get(&source_name.value()) else {
        return Err(Error::new(
            source_name.span(),
            "function template alias source must refer to an earlier string-named method",
        ));
    };
    let key = webapi_field_key(field, attrs, rename_all)?;
    let name = key.display_name;
    let property_key = key.property_key;
    let attributes = template_property_attributes(attrs);
    let field_read = field
        .ident
        .as_ref()
        .map(|ident| quote!(let _ = ::std::stringify!(#ident);));
    Ok(quote! {
        #field_read
        prototype.set_with_attr(
            #property_key,
            #source_binding.into(),
            #attributes,
        );
        let _ = #name;
    })
}

fn expand_interface_field(
    field: &Field,
    rename_all: RenameRule,
) -> Option<Result<proc_macro2::TokenStream, Error>> {
    let attrs = match parse_field_attrs(field) {
        Ok(attrs) => attrs,
        Err(error) => return Some(Err(error)),
    };
    if !attrs.method && !attrs.accessor_property {
        if attrs.has_installation_kind() || attrs.has_installation_attribute() {
            return Some(Err(Error::new(
                field.span(),
                "interface fields with #[webapi(...)] attributes must declare #[webapi(method)] or #[webapi(accessor_property)]",
            )));
        }
        return None;
    }
    if attrs.method {
        let key = match webapi_field_key(field, &attrs, rename_all) {
            Ok(key) => key,
            Err(error) => return Some(Err(error)),
        };
        let name = key.display_name;
        let property_key = key.property_key;
        let set_function_name = key.function_name.map(|function_name| {
            quote! {
                function.set_name(#function_name);
            }
        });
        let Some(callback) = attrs.callback.as_ref() else {
            return Some(Err(Error::new(
                field.span(),
                "method field requires #[webapi(callback = path)]",
            )));
        };
        let length = attrs.length.unwrap_or(0);
        let enumerable = attrs.enumerable;
        let writable = !attrs.readonly;
        let configurable = !attrs.dont_delete;
        let build_function =
            expand_method_function_builder(callback, length, attrs.data.as_ref(), &name);
        let field_read = field
            .ident
            .as_ref()
            .map(|ident| quote!(let _ = &self.#ident;));
        return Some(Ok(quote! {
            #field_read
            let function = #build_function.ok_or_else(|| {
                ::moli_webapi_declare::BindError::new(
                    ::std::format!("failed to build `{}.{}` function", INTERFACE_NAME, #name)
                )
            })?;
            #set_function_name
            let mut descriptor =
                ::moli_webapi_declare::v8::PropertyDescriptor::new_from_value_writable(
                    function.into(),
                    #writable,
                );
            descriptor.set_configurable(#configurable);
            descriptor.set_enumerable(#enumerable);
            prototype
                .define_property(
                    scope,
                    #property_key,
                    &descriptor,
                )
                .unwrap_or(false)
                .then_some(())
                .ok_or_else(|| {
                    ::moli_webapi_declare::BindError::new(
                        ::std::format!("failed to define `{}.{}` function", INTERFACE_NAME, #name)
                    )
                })?;
        }));
    }
    if attrs.accessor_property {
        return Some(expand_accessor_property_field(
            field,
            &attrs,
            rename_all,
            quote!(prototype),
        ));
    }
    None
}

fn single_lifetime_param(generics: &syn::Generics) -> Option<syn::Lifetime> {
    // Most object declarations carry exactly one lifetime, and that lifetime is
    // the V8 scope lifetime used by `v8::Local<'scope, T>` fields. In that common
    // case the derive can infer the generated `WebApiObjectDeclaration<'scope>`
    // impl. Declarations with multiple lifetimes stay explicit so data lifetimes
    // such as `&'tag str` are not accidentally treated as the V8 scope.
    let mut lifetimes = generics.params.iter().filter_map(|param| match param {
        GenericParam::Lifetime(param) => Some(param.lifetime.clone()),
        _ => None,
    });
    let lifetime = lifetimes.next()?;
    lifetimes.next().is_none().then_some(lifetime)
}

fn expand_object_field(
    field: &Field,
    object_attrs: &ObjectAttrs,
) -> Option<Result<proc_macro2::TokenStream, Error>> {
    let mut attrs = match parse_field_attrs(field) {
        Ok(attrs) => attrs,
        Err(error) => return Some(Err(error)),
    };
    // Struct-level `#[webapi(data_properties)]` is the only mode where an unannotated
    // field becomes part of the JavaScript surface by default. Without it,
    // unannotated fields are declaration-only inputs that can still be consumed
    // by other attributes, for example `#[webapi(method, data = self.cache)]`.
    if object_attrs.default_data_properties
        && !attrs.method
        && !attrs.static_method
        && !attrs.constant
        && !attrs.accessor_property
        && !attrs.native_data_property
        && attrs.intrinsic_data_property.is_none()
        && !attrs.data_property
        && attrs.alias.is_none()
        && !attrs.hidden
        && !attrs.slot
        && !attrs.prototype
        && !attrs.to_string_tag
    {
        attrs.data_property = true;
    }
    if object_attrs.default_enumerable
        && attrs.symbol.is_none()
        && (attrs.data_property
            || attrs.method
            || attrs.accessor_property
            || attrs.native_data_property
            || attrs.alias.is_some())
    {
        attrs.enumerable = true;
    }
    if !attrs.has_installation_kind() {
        if attrs.has_installation_attribute() {
            return Some(Err(Error::new(
                field.span(),
                "declaration-only fields cannot use #[webapi(...)] installation attributes; add an installation kind or remove the attributes",
            )));
        }
        return None;
    }
    if attrs.static_method {
        return Some(Err(Error::new(
            field.span(),
            "object fields with #[webapi(...)] attributes cannot declare #[webapi(static_method)]",
        )));
    }
    if attrs.intrinsic_data_property.is_some() {
        return Some(Err(Error::new(
            field.span(),
            "`intrinsic_data_property` is only supported by WebApiFunctionTemplate",
        )));
    }
    if attrs.constant {
        let name = match webapi_field_name(field, attrs.name.as_ref(), object_attrs.rename_all) {
            Ok(name) => name,
            Err(error) => return Some(Err(error)),
        };
        let value = match expand_object_field_value(field, &attrs, false) {
            Ok(value) => value,
            Err(error) => return Some(Err(error)),
        };
        return Some(Ok(quote! {
            #value
            ::moli_webapi_declare::define_declared_constant_property(
                scope,
                object,
                #name,
                __webapi_value_ref,
            )?;
        }));
    }
    if attrs.method {
        return Some(expand_object_method_field(
            field,
            &attrs,
            object_attrs.rename_all,
        ));
    }
    if attrs.accessor_property {
        return Some(expand_accessor_property_field(
            field,
            &attrs,
            object_attrs.rename_all,
            quote!(object),
        ));
    }
    if attrs.native_data_property {
        return Some(expand_native_data_property_field(
            field,
            &attrs,
            object_attrs.rename_all,
        ));
    }
    if attrs.alias.is_some() {
        return Some(expand_alias_field(field, &attrs, object_attrs.rename_all));
    }
    if attrs.prototype || attrs.to_string_tag {
        let field_is_option = type_is_option(&field.ty);
        let optional = field_is_option && attrs.value.is_none() && attrs.init.is_none();
        let value = match expand_object_field_value(field, &attrs, optional) {
            Ok(value) => value,
            Err(error) => return Some(Err(error)),
        };
        let bind_value = if attrs.prototype {
            quote! {
                ::moli_webapi_declare::set_declared_prototype(
                    scope,
                    object,
                    __webapi_value_ref,
                )?;
            }
        } else {
            let attributes = to_string_tag_property_attributes(&attrs);
            quote! {
                ::moli_webapi_declare::define_declared_to_string_tag_with_attributes(
                    scope,
                    object,
                    __webapi_value_ref,
                    #attributes,
                )?;
            }
        };
        if optional {
            return Some(Ok(quote! {
                if let ::std::option::Option::Some(__webapi_value_ref) = #value {
                    #bind_value
                }
            }));
        }
        return Some(Ok(quote! {
            #value
            #bind_value
        }));
    }
    // No installation kind means the field is intentionally not written to the
    // object. This allows declarations to carry Rust/V8 values used only while
    // generating methods or metadata. Keeping the skip here prevents accidental
    // own-data-property exposure on web-facing wrappers.
    if !attrs.data_property && !attrs.hidden && !attrs.slot {
        return None;
    }
    let rename_all = if attrs.hidden || attrs.slot {
        RenameRule::None
    } else {
        object_attrs.rename_all
    };
    let name = match webapi_field_name(field, attrs.name.as_ref(), rename_all) {
        Ok(name) => name,
        Err(error) => return Some(Err(error)),
    };
    let field_is_option = type_is_option(&field.ty);
    let optional = field_is_option && attrs.value.is_none() && attrs.init.is_none();
    let value = match expand_object_field_value(field, &attrs, optional) {
        Ok(value) => value,
        Err(error) => return Some(Err(error)),
    };
    let define_value = if attrs.slot {
        quote! {
            ::moli_webapi_declare::define_declared_private_slot(
                scope,
                object,
                #name,
                __webapi_value_ref,
            )?;
        }
    } else if attrs.hidden {
        let writable = syn::LitBool::new(!attrs.readonly, proc_macro2::Span::call_site());
        let configurable = syn::LitBool::new(!attrs.dont_delete, proc_macro2::Span::call_site());
        quote! {
            ::moli_webapi_declare::define_declared_hidden_property_with_descriptor(
                scope,
                object,
                #name,
                __webapi_value_ref,
                #writable,
                #configurable,
            )?;
        }
    } else if attrs.readonly || attrs.dont_delete {
        let mut property_attributes = if attrs.enumerable {
            quote!(::moli_webapi_declare::v8::PropertyAttribute::NONE)
        } else {
            quote!(::moli_webapi_declare::v8::PropertyAttribute::DONT_ENUM)
        };
        if attrs.readonly {
            property_attributes = quote! {
                #property_attributes
                    | ::moli_webapi_declare::v8::PropertyAttribute::READ_ONLY
            };
        }
        if attrs.dont_delete {
            property_attributes = quote! {
                #property_attributes
                    | ::moli_webapi_declare::v8::PropertyAttribute::DONT_DELETE
            };
        }
        quote! {
            ::moli_webapi_declare::define_declared_data_property_with_attributes(
                scope,
                object,
                #name,
                __webapi_value_ref,
                #property_attributes,
            )?;
        }
    } else if attrs.enumerable {
        quote! {
            ::moli_webapi_declare::define_declared_enumerable_data_property(
                scope,
                object,
                #name,
                __webapi_value_ref,
            )?;
        }
    } else {
        quote! {
            ::moli_webapi_declare::define_declared_data_property(
                scope,
                object,
                #name,
                __webapi_value_ref,
            )?;
        }
    };
    if optional {
        return Some(Ok(quote! {
            if let ::std::option::Option::Some(__webapi_value_ref) = #value {
                #define_value
            }
        }));
    }
    if attrs.hidden {
        return Some(Ok(quote! {
            #value
            #define_value
        }));
    }
    if attrs.slot {
        return Some(Ok(quote! {
            #value
            ::moli_webapi_declare::define_declared_private_slot(
                scope,
                object,
                #name,
                __webapi_value_ref,
            )?;
        }));
    }
    Some(Ok(quote! {
        #value
        #define_value
    }))
}

fn expand_accessor_property_field(
    field: &Field,
    attrs: &crate::attrs::FieldAttrs,
    rename_all: RenameRule,
    object: proc_macro2::TokenStream,
) -> Result<proc_macro2::TokenStream, Error> {
    if attrs.readonly {
        return Err(Error::new(
            field.span(),
            "runtime-object `accessor_property` fields have no writable attribute; omit #[webapi(setter)] instead of using readonly",
        ));
    }
    let key = webapi_field_key(field, attrs, rename_all)?;
    let name = key.display_name;
    let property_key = key.property_key;
    if attrs.getter.is_some() && attrs.getter_value.is_some() {
        return Err(Error::new(
            field.span(),
            "`accessor_property` field cannot declare both #[webapi(getter = ...)] and #[webapi(getter_value = ...)]",
        ));
    }
    let getter = if let Some(getter) = attrs.getter.as_ref() {
        let getter = expand_accessor_function_builder(getter, 0, attrs.data.as_ref(), &name);
        quote! {
            #getter.ok_or_else(|| {
                ::moli_webapi_declare::BindError::new(
                    ::std::format!("failed to build declared `{}` getter", #name)
                )
            })?
        }
    } else if let Some(getter_value) = attrs.getter_value.as_ref() {
        quote!(#getter_value)
    } else {
        return Err(Error::new(
            field.span(),
            "`accessor_property` field requires #[webapi(getter = path)] or #[webapi(getter_value = expr)]",
        ));
    };
    let setter = attrs.setter.as_ref().map(|setter| {
        let setter_data = attrs.setter_data.as_ref().or(attrs.data.as_ref());
        expand_accessor_function_builder(setter, 1, setter_data, &name)
    });
    let setter = match setter {
        Some(setter) => quote! {
            ::std::option::Option::Some(
                #setter.ok_or_else(|| {
                    ::moli_webapi_declare::BindError::new(
                        ::std::format!("failed to build declared `{}` setter", #name)
                    )
                })?
            )
        },
        None => quote!(::std::option::Option::None),
    };
    let attributes = accessor_property_attributes(attrs);
    let field_read = field
        .ident
        .as_ref()
        .map(|ident| quote!(let _ = &self.#ident;));
    Ok(quote! {
        #field_read
        let __webapi_getter = #getter;
        let __webapi_setter = #setter;
        ::moli_webapi_declare::define_declared_accessor_property_by_key(
            scope,
            #object,
            #property_key,
            #name,
            __webapi_getter,
            __webapi_setter,
            #attributes,
        )?;
    })
}

fn expand_native_data_property_field(
    field: &Field,
    attrs: &crate::attrs::FieldAttrs,
    rename_all: RenameRule,
) -> Result<proc_macro2::TokenStream, Error> {
    if attrs.readonly {
        return Err(Error::new(
            field.span(),
            "`native_data_property` fields have no writable attribute; omit #[webapi(setter)] instead of using readonly",
        ));
    }
    let key = webapi_field_key(field, attrs, rename_all)?;
    let name = key.display_name;
    let property_key = key.property_key;
    let Some(getter) = attrs.getter.as_ref() else {
        return Err(Error::new(
            field.span(),
            "`native_data_property` field requires #[webapi(getter = path)]",
        ));
    };
    let setter = attrs.setter.as_ref().map(|setter| {
        quote! {
            .setter(#setter)
        }
    });
    let data = attrs.data.as_ref().map(|data| {
        quote! {
            .data((#data).into())
        }
    });
    let attributes = accessor_property_attributes(attrs);
    let field_read = field
        .ident
        .as_ref()
        .map(|ident| quote!(let _ = &self.#ident;));
    Ok(quote! {
        #field_read
        let __webapi_native_data_property_configuration =
            ::moli_webapi_declare::v8::NativeDataPropertyConfiguration::new(#getter)
                #setter
                #data
                .property_attribute(#attributes);
        object
            .set_native_data_property_with_configuration(
                scope,
                #property_key,
                __webapi_native_data_property_configuration,
            )
            .unwrap_or(false)
            .then_some(())
            .ok_or_else(|| {
                ::moli_webapi_declare::BindError::new(
                    ::std::format!("failed to define declared `{}` native data property", #name)
                )
            })?;
        let _ = #name;
    })
}

fn expand_object_method_field(
    field: &Field,
    attrs: &crate::attrs::FieldAttrs,
    rename_all: RenameRule,
) -> Result<proc_macro2::TokenStream, Error> {
    let key = webapi_field_key(field, attrs, rename_all)?;
    let name = key.display_name;
    let property_key = key.property_key;
    let set_function_name = key.function_name.map(|function_name| {
        quote! {
            function.set_name(#function_name);
        }
    });
    let Some(callback) = attrs.callback.as_ref() else {
        return Err(Error::new(
            field.span(),
            "method field requires #[webapi(callback = path)]",
        ));
    };
    let length = attrs.length.unwrap_or(0);
    let enumerable = attrs.enumerable;
    let writable = !attrs.readonly;
    let configurable = !attrs.dont_delete;
    let build_function =
        expand_method_function_builder(callback, length, attrs.data.as_ref(), &name);
    let install_method = quote! {
        let function = #build_function.ok_or_else(|| {
            ::moli_webapi_declare::BindError::new(
                ::std::format!("failed to build declared `{}` method", #name)
            )
        })?;
        #set_function_name
        let mut descriptor =
            ::moli_webapi_declare::v8::PropertyDescriptor::new_from_value_writable(
                function.into(),
                #writable,
            );
        descriptor.set_configurable(#configurable);
        descriptor.set_enumerable(#enumerable);
        object
            .define_property(
                scope,
                #property_key,
                &descriptor,
            )
            .unwrap_or(false)
            .then_some(())
            .ok_or_else(|| {
                ::moli_webapi_declare::BindError::new(
                    ::std::format!("failed to define declared `{}` method", #name)
                )
            })?;
    };
    let Some(ident) = field.ident.as_ref() else {
        return Ok(quote! {
            #install_method
        });
    };
    if type_is_option(&field.ty) {
        return Ok(quote! {
            if self.#ident.is_some() {
                #install_method
            }
        });
    }
    Ok(quote! {
        let _ = &self.#ident;
        #install_method
    })
}

fn expand_alias_field(
    field: &Field,
    attrs: &crate::attrs::FieldAttrs,
    rename_all: RenameRule,
) -> Result<proc_macro2::TokenStream, Error> {
    let key = webapi_field_key(field, attrs, rename_all)?;
    let name = key.display_name;
    let property_key = key.property_key;
    let Some(source_name) = attrs.alias.as_ref() else {
        return Err(Error::new(
            field.span(),
            "alias field requires a source name",
        ));
    };
    let enumerable = syn::LitBool::new(attrs.enumerable, proc_macro2::Span::call_site());
    let writable = syn::LitBool::new(!attrs.readonly, proc_macro2::Span::call_site());
    let configurable = syn::LitBool::new(!attrs.dont_delete, proc_macro2::Span::call_site());
    let field_read = field
        .ident
        .as_ref()
        .map(|ident| quote!(let _ = &self.#ident;));
    let optional = type_is_option(&field.ty);
    let install_alias = quote! {
        let __webapi_alias_source_key =
            ::moli_webapi_declare::__private::v8str(scope, #source_name);
        object
            .has_own_property(scope, __webapi_alias_source_key.into())
            .unwrap_or(false)
            .then_some(())
            .ok_or_else(|| {
                ::moli_webapi_declare::BindError::new(
                    ::std::format!(
                        "failed to find declared alias source `{}` for `{}`",
                        #source_name,
                        #name
                    )
                )
            })?;
        let __webapi_alias_value = object
            .get(scope, __webapi_alias_source_key.into())
            .ok_or_else(|| {
                ::moli_webapi_declare::BindError::new(
                    ::std::format!(
                        "failed to read declared alias source `{}` for `{}`",
                        #source_name,
                        #name
                    )
                )
            })?;
        let mut descriptor =
            ::moli_webapi_declare::v8::PropertyDescriptor::new_from_value_writable(
                __webapi_alias_value,
                #writable,
            );
        descriptor.set_configurable(#configurable);
        descriptor.set_enumerable(#enumerable);
        object
            .define_property(scope, #property_key, &descriptor)
            .unwrap_or(false)
            .then_some(())
            .ok_or_else(|| {
                ::moli_webapi_declare::BindError::new(
                    ::std::format!("failed to define declared `{}` alias", #name)
                )
            })?;
    };
    if optional {
        let Some(ident) = field.ident.as_ref() else {
            return Err(Error::new(
                field.span(),
                "optional alias field requires a field name",
            ));
        };
        return Ok(quote! {
            if self.#ident.is_some() {
                #install_alias
            }
        });
    }
    Ok(quote! {
        #field_read
        #install_alias
    })
}

fn expand_accessor_function_builder(
    callback: &syn::Path,
    length: i32,
    data: Option<&syn::Expr>,
    display_name: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    match data {
        Some(data) => quote! {
            {
                let __webapi_callback_data = (#data);
                let __webapi_callback_data =
                    ::moli_webapi_declare::WebApiValue::to_v8_value(
                        &__webapi_callback_data,
                        scope,
                    )
                    .ok_or_else(|| {
                        ::moli_webapi_declare::BindError::new(
                            ::std::format!("failed to convert declared `{}` callback data", #display_name)
                        )
                    })?;
                ::moli_webapi_declare::v8::Function::builder(#callback)
                    .length(#length)
                    .data(__webapi_callback_data)
                    .constructor_behavior(
                        ::moli_webapi_declare::v8::ConstructorBehavior::Throw,
                    )
                    .build(scope)
            }
        },
        None => quote! {
            ::moli_webapi_declare::v8::Function::builder(#callback)
                .length(#length)
                .constructor_behavior(
                    ::moli_webapi_declare::v8::ConstructorBehavior::Throw,
                )
                .build(scope)
        },
    }
}

fn expand_method_function_builder(
    callback: &syn::Path,
    length: i32,
    data: Option<&syn::Expr>,
    display_name: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    // `data` is evaluated inside the generated `initialize` body, so it can use
    // the V8 `scope` parameter and `self` fields. This is how declarations pass
    // non-reflectable callback state without declaring that state as a JS
    // data_property or private slot.
    match data {
        Some(data) => quote! {
            {
                let __webapi_callback_data = (#data);
                let __webapi_callback_data =
                    ::moli_webapi_declare::WebApiValue::to_v8_value(
                        &__webapi_callback_data,
                        scope,
                    )
                    .ok_or_else(|| {
                        ::moli_webapi_declare::BindError::new(
                            ::std::format!("failed to convert declared `{}` callback data", #display_name)
                        )
                    })?;
                ::moli_webapi_declare::v8::Function::builder(#callback)
                    .length(#length)
                    .data(__webapi_callback_data)
                    .constructor_behavior(
                        ::moli_webapi_declare::v8::ConstructorBehavior::Throw,
                    )
                    .build(scope)
            }
        },
        None => quote! {
            ::moli_webapi_declare::v8::Function::builder(#callback)
                .length(#length)
                .constructor_behavior(
                    ::moli_webapi_declare::v8::ConstructorBehavior::Throw,
                )
                .build(scope)
        },
    }
}

fn expand_template_function_builder(
    callback: &syn::Path,
    length: i32,
    data: Option<&syn::Expr>,
    template_name: &LitStr,
    display_name: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    match data {
        Some(data) => quote! {
            {
                let __webapi_callback_data = (#data);
                let __webapi_callback_data =
                    ::moli_webapi_declare::WebApiTemplateValue::to_v8_template_value(
                        &__webapi_callback_data,
                        scope,
                    )
                    .unwrap_or_else(|| {
                        panic!(
                            "failed to convert Web API `{}` member `{}` template callback data",
                            #template_name,
                            #display_name,
                        )
                    });
                ::moli_webapi_declare::v8::FunctionTemplate::builder(#callback)
                    .length(#length)
                    .data(__webapi_callback_data)
                    .constructor_behavior(
                        ::moli_webapi_declare::v8::ConstructorBehavior::Throw,
                    )
                    .build(scope)
            }
        },
        None => quote! {
            ::moli_webapi_declare::v8::FunctionTemplate::builder(#callback)
                .length(#length)
                .constructor_behavior(
                    ::moli_webapi_declare::v8::ConstructorBehavior::Throw,
                )
                .build(scope)
        },
    }
}

fn expand_template_accessor_function_builder(
    callback: &syn::Path,
    length: i32,
    data: Option<&syn::Expr>,
    template_name: &LitStr,
    display_name: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    match data {
        Some(data) => quote! {
            {
                let __webapi_callback_data = (#data);
                let __webapi_callback_data =
                    ::moli_webapi_declare::WebApiTemplateValue::to_v8_template_value(
                        &__webapi_callback_data,
                        scope,
                    )
                    .unwrap_or_else(|| {
                        panic!(
                            "failed to convert Web API `{}` member `{}` template callback data",
                            #template_name,
                            #display_name,
                        )
                    });
                ::moli_webapi_declare::v8::FunctionTemplate::builder(#callback)
                    .length(#length)
                    .data(__webapi_callback_data)
                    .constructor_behavior(
                        ::moli_webapi_declare::v8::ConstructorBehavior::Throw,
                    )
                    .build(scope)
            }
        },
        None => quote! {
            ::moli_webapi_declare::v8::FunctionTemplate::builder(#callback)
                .length(#length)
                .constructor_behavior(
                    ::moli_webapi_declare::v8::ConstructorBehavior::Throw,
                )
                .build(scope)
        },
    }
}

fn accessor_property_attributes(attrs: &crate::attrs::FieldAttrs) -> proc_macro2::TokenStream {
    let mut property_attributes = if attrs.enumerable {
        quote!(::moli_webapi_declare::v8::PropertyAttribute::NONE)
    } else {
        quote!(::moli_webapi_declare::v8::PropertyAttribute::DONT_ENUM)
    };
    if attrs.readonly {
        property_attributes = quote! {
            #property_attributes
                | ::moli_webapi_declare::v8::PropertyAttribute::READ_ONLY
        };
    }
    if attrs.dont_delete {
        property_attributes = quote! {
            #property_attributes
                | ::moli_webapi_declare::v8::PropertyAttribute::DONT_DELETE
        };
    }
    property_attributes
}

fn template_property_attributes(attrs: &crate::attrs::FieldAttrs) -> proc_macro2::TokenStream {
    let mut property_attributes = if attrs.enumerable {
        quote!(::moli_webapi_declare::v8::PropertyAttribute::NONE)
    } else {
        quote!(::moli_webapi_declare::v8::PropertyAttribute::DONT_ENUM)
    };
    if attrs.readonly {
        property_attributes = quote! {
            #property_attributes
                | ::moli_webapi_declare::v8::PropertyAttribute::READ_ONLY
        };
    }
    if attrs.dont_delete {
        property_attributes = quote! {
            #property_attributes
                | ::moli_webapi_declare::v8::PropertyAttribute::DONT_DELETE
        };
    }
    property_attributes
}

fn constant_property_attributes() -> proc_macro2::TokenStream {
    quote! {
        ::moli_webapi_declare::webidl_constant_property_attributes()
    }
}

fn to_string_tag_property_attributes(attrs: &crate::attrs::FieldAttrs) -> proc_macro2::TokenStream {
    let mut property_attributes = quote!(::moli_webapi_declare::v8::PropertyAttribute::DONT_ENUM);
    if attrs.readonly {
        property_attributes = quote! {
            #property_attributes
                | ::moli_webapi_declare::v8::PropertyAttribute::READ_ONLY
        };
    }
    if attrs.dont_delete {
        property_attributes = quote! {
            #property_attributes
                | ::moli_webapi_declare::v8::PropertyAttribute::DONT_DELETE
        };
    }
    property_attributes
}

fn webapi_field_key(
    field: &Field,
    attrs: &crate::attrs::FieldAttrs,
    rename_all: RenameRule,
) -> Result<WebApiFieldKey, Error> {
    let mut key = if let Some(symbol) = attrs.symbol.as_ref() {
        webapi_symbol_field_key(symbol)?
    } else {
        let name = webapi_field_name(field, attrs.name.as_ref(), rename_all)?;
        let name_literal = webapi_field_name_literal(field, attrs.name.as_ref(), rename_all)?;
        WebApiFieldKey {
            display_name: name.clone(),
            display_name_literal: name_literal,
            property_key: quote!(::moli_webapi_declare::__private::v8str(scope, #name).into()),
            function_name: Some(quote!(
                ::moli_webapi_declare::__private::v8str(scope, #name)
            )),
        }
    };
    if let Some(function_name) = attrs.function_name.as_ref() {
        key.function_name = Some(quote!(
            ::moli_webapi_declare::__private::v8str(scope, #function_name)
        ));
    }
    Ok(key)
}

fn webapi_symbol_field_key(symbol: &LitStr) -> Result<WebApiFieldKey, Error> {
    match symbol.value().as_str() {
        "iterator" => {
            let display_name = LitStr::new("[Symbol.iterator]", symbol.span());
            Ok(WebApiFieldKey {
                display_name: quote!(#display_name),
                display_name_literal: Some(display_name),
                property_key: quote!(::moli_webapi_declare::v8::Symbol::get_iterator(scope).into()),
                function_name: None,
            })
        }
        "asyncIterator" => {
            let display_name = LitStr::new("[Symbol.asyncIterator]", symbol.span());
            Ok(WebApiFieldKey {
                display_name: quote!(#display_name),
                display_name_literal: Some(display_name),
                property_key: quote!(
                    ::moli_webapi_declare::v8::Symbol::get_async_iterator(scope).into()
                ),
                function_name: None,
            })
        }
        "toStringTag" => {
            let display_name = LitStr::new("[Symbol.toStringTag]", symbol.span());
            Ok(WebApiFieldKey {
                display_name: quote!(#display_name),
                display_name_literal: Some(display_name),
                property_key: quote!(
                    ::moli_webapi_declare::v8::Symbol::get_to_string_tag(scope).into()
                ),
                function_name: None,
            })
        }
        _ => Err(Error::new(symbol.span(), "unsupported Web API symbol key")),
    }
}

fn webapi_field_name(
    field: &Field,
    explicit: Option<&syn::Expr>,
    rename_all: RenameRule,
) -> Result<proc_macro2::TokenStream, Error> {
    if let Some(name) = explicit {
        return Ok(quote!(#name));
    }
    let Some(ident) = field.ident.as_ref() else {
        return Err(Error::new(field.span(), "object field requires a name"));
    };
    let name = ident.to_string();
    let name = name.strip_prefix("r#").unwrap_or(&name);
    let name = name.strip_prefix('_').unwrap_or(name);
    if name.is_empty() {
        return Err(Error::new(field.span(), "webapi field requires a name"));
    }
    let name = apply_rename_rule(name, rename_all);
    let name = LitStr::new(&name, ident.span());
    Ok(quote!(#name))
}

fn webapi_field_name_literal(
    field: &Field,
    explicit: Option<&syn::Expr>,
    rename_all: RenameRule,
) -> Result<Option<LitStr>, Error> {
    if let Some(name) = explicit {
        return match name {
            syn::Expr::Lit(expr) => match &expr.lit {
                Lit::Str(name) => Ok(Some(name.clone())),
                _ => Ok(None),
            },
            _ => Ok(None),
        };
    }
    let Some(ident) = field.ident.as_ref() else {
        return Err(Error::new(field.span(), "object field requires a name"));
    };
    let name = ident.to_string();
    let name = name.strip_prefix("r#").unwrap_or(&name);
    let name = name.strip_prefix('_').unwrap_or(name);
    if name.is_empty() {
        return Err(Error::new(field.span(), "webapi field requires a name"));
    }
    let name = apply_rename_rule(name, rename_all);
    Ok(Some(LitStr::new(&name, ident.span())))
}

fn apply_rename_rule(name: &str, rename_all: RenameRule) -> String {
    match rename_all {
        RenameRule::None => name.to_owned(),
        RenameRule::CamelCase => snake_to_camel_case(name),
    }
}

fn snake_to_camel_case(name: &str) -> String {
    let mut output = String::with_capacity(name.len());
    let mut uppercase_next = false;
    for character in name.chars() {
        if character == '_' {
            uppercase_next = !output.is_empty();
            continue;
        }
        if uppercase_next {
            if character.is_alphabetic() {
                output.extend(character.to_uppercase());
                uppercase_next = false;
            } else {
                output.push(character);
            }
        } else {
            output.push(character);
        }
    }
    output
}

fn expand_object_field_value(
    field: &Field,
    attrs: &crate::attrs::FieldAttrs,
    optional: bool,
) -> Result<proc_macro2::TokenStream, Error> {
    if let Some(value) = attrs.value.as_ref() {
        let field_read = field
            .ident
            .as_ref()
            .map(|ident| quote!(let _ = &self.#ident;));
        return Ok(quote! {
            #field_read
            let __webapi_value = (#value);
            let __webapi_value_ref = &__webapi_value;
        });
    }
    if let Some(init) = attrs.init.as_ref() {
        let field_read = field
            .ident
            .as_ref()
            .map(|ident| quote!(let _ = &self.#ident;));
        return Ok(match init {
            ValueInitAttr::Null => {
                quote! {
                    #field_read
                    let __webapi_value = ::moli_webapi_declare::v8::null(scope);
                    let __webapi_value_ref = &__webapi_value;
                }
            }
            ValueInitAttr::Object => {
                quote! {
                    #field_read
                    let __webapi_value = ::moli_webapi_declare::v8::Object::new(scope);
                    let __webapi_value_ref = &__webapi_value;
                }
            }
            ValueInitAttr::NullObject => {
                quote! {
                    #field_read
                    let __webapi_value = ::moli_webapi_declare::v8::Object::new(scope);
                    let __webapi_null = ::moli_webapi_declare::v8::null(scope);
                    let _ = __webapi_value.set_prototype(scope, __webapi_null.into());
                    let __webapi_value_ref = &__webapi_value;
                }
            }
            ValueInitAttr::Array => {
                quote! {
                    #field_read
                    let __webapi_value = ::moli_webapi_declare::v8::Array::new(scope, 0);
                    let __webapi_value_ref = &__webapi_value;
                }
            }
            ValueInitAttr::Undefined => {
                quote! {
                    #field_read
                    let __webapi_value = ::moli_webapi_declare::v8::undefined(scope);
                    let __webapi_value_ref = &__webapi_value;
                }
            }
            ValueInitAttr::True => {
                quote! {
                    #field_read
                    let __webapi_value = ::moli_webapi_declare::v8::Boolean::new(scope, true);
                    let __webapi_value_ref = &__webapi_value;
                }
            }
            ValueInitAttr::False => {
                quote! {
                    #field_read
                    let __webapi_value = ::moli_webapi_declare::v8::Boolean::new(scope, false);
                    let __webapi_value_ref = &__webapi_value;
                }
            }
            ValueInitAttr::Zero => {
                quote! {
                    #field_read
                    let __webapi_value = ::moli_webapi_declare::v8::Number::new(scope, 0.0);
                    let __webapi_value_ref = &__webapi_value;
                }
            }
            ValueInitAttr::EmptyString => {
                quote! {
                    #field_read
                    let __webapi_value = ::moli_webapi_declare::v8::String::empty(scope);
                    let __webapi_value_ref = &__webapi_value;
                }
            }
            ValueInitAttr::String(value) => {
                let field_name = field
                    .ident
                    .as_ref()
                    .map(|ident| ident.to_string())
                    .unwrap_or_else(|| "<unnamed>".to_string());
                let field_name = LitStr::new(&field_name, field.span());
                quote! {
                    #field_read
                    let __webapi_init_value = #value;
                    let __webapi_value =
                        ::moli_webapi_declare::WebApiValue::to_v8_value(
                            &__webapi_init_value,
                            scope,
                        )
                        .ok_or_else(|| {
                            ::moli_webapi_declare::BindError::new(
                                ::std::format!("failed to convert declared `{}` initializer", #field_name)
                            )
                        })?;
                    let __webapi_value_ref = &__webapi_value;
                }
            }
        });
    }
    if attrs.data_property || attrs.hidden || attrs.slot || attrs.prototype || attrs.to_string_tag {
        let Some(ident) = field.ident.as_ref() else {
            return Err(Error::new(field.span(), "object field requires a name"));
        };
        if optional {
            return Ok(quote! {
                self.#ident.as_ref()
            });
        }
        return Ok(quote! {
            let __webapi_value_ref = &self.#ident;
        });
    }
    Err(Error::new(
        field.span(),
        "object field requires #[webapi(value = expr)]",
    ))
}

fn type_is_option(ty: &Type) -> bool {
    let Type::Path(path) = ty else {
        return false;
    };
    path.qself.is_none()
        && path
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "Option")
}

fn type_is_unit(ty: &Type) -> bool {
    matches!(ty, Type::Tuple(tuple) if tuple.elems.is_empty())
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

#[cfg(test)]
mod tests {
    use super::{expand_webapi_function_template, expand_webapi_interface, expand_webapi_object};

    #[test]
    fn declaration_only_field_attributes_are_rejected() {
        let input = syn::parse_quote! {
            #[webapi(interface = "Object")]
            struct BadObject {
                #[webapi(init = "null")]
                ignored: (),
            }
        };
        let error = match expand_webapi_object(input) {
            Ok(_) => panic!("declaration-only field attributes should be rejected"),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            "declaration-only fields cannot use #[webapi(...)] installation attributes; add an installation kind or remove the attributes"
        );
    }

    #[test]
    fn default_data_property_fields_can_use_data_property_initializers() {
        let input = syn::parse_quote! {
            #[webapi(interface = "Object", data_properties)]
            struct DefaultObject {
                #[webapi(init = "null")]
                value: (),
            }
        };
        expand_webapi_object(input).expect("default data-property initializer should expand");
    }

    #[test]
    fn object_generated_constructor_defaults_to_new_for_dynamic_fields() {
        let input = syn::parse_quote! {
            #[webapi(interface = "Object")]
            struct DynamicObject {
                #[webapi(data_property, init = true)]
                brand: (),

                #[webapi(data_property)]
                value: u32,
            }
        };
        let tokens = expand_webapi_object(input)
            .expect("dynamic object should expand")
            .to_string();
        assert!(tokens.contains("pub fn new (value : u32) -> Self"));
        assert!(tokens.contains("brand : ()"));
        assert!(tokens.contains("value"));
    }

    #[test]
    fn object_generated_constructor_defaults_to_new_for_unit_fields() {
        let input = syn::parse_quote! {
            #[webapi(interface = "Object")]
            struct StaticObject {
                #[webapi(data_property, init = true)]
                brand: (),

                #[webapi(method, callback = static_callback)]
                action: (),
            }
        };
        let tokens = expand_webapi_object(input)
            .expect("static object should expand")
            .to_string();
        assert!(tokens.contains("pub fn new () -> Self"));
        assert!(tokens.contains("brand : ()"));
        assert!(tokens.contains("action : ()"));
    }

    #[test]
    fn object_generated_constructor_uses_constructor_defaults() {
        let input = syn::parse_quote! {
            #[webapi(interface = "Object")]
            struct DefaultedObject {
                #[webapi(data_property)]
                value: u32,

                #[webapi(data_property, constructor_default = "ready")]
                state: &'static str,

                #[webapi(data_property, constructor_default = Vec::new())]
                items: Vec<u32>,

                #[webapi(data_property, constructor_default)]
                count: usize,
            }
        };
        let tokens = expand_webapi_object(input)
            .expect("defaulted object should expand")
            .to_string();
        assert!(tokens.contains("pub fn new (value : u32) -> Self"));
        assert!(tokens.contains("state : \"ready\""));
        assert!(tokens.contains("items : Vec :: new ()"));
        assert!(tokens.contains("count : :: std :: default :: Default :: default ()"));
    }

    #[test]
    fn object_generated_constructor_defaults_can_reference_previous_fields() {
        let input = syn::parse_quote! {
            #[webapi(interface = "Object")]
            struct DerivedDefaultObject {
                #[webapi(data_property)]
                client_x: i32,

                #[webapi(data_property, constructor_default = client_x)]
                x: i32,
            }
        };
        let tokens = expand_webapi_object(input)
            .expect("derived default object should expand")
            .to_string();
        assert!(tokens.contains("pub fn new (client_x : i32) -> Self"));
        assert!(tokens.contains("client_x"));
        assert!(tokens.contains("x : client_x"));
    }

    #[test]
    fn object_generated_constructor_can_be_suppressed() {
        let input = syn::parse_quote! {
            #[webapi(interface = "Object", no_dynamic_constructor)]
            struct ManualObject {
                #[webapi(data_property)]
                value: u32,
            }
        };
        let tokens = expand_webapi_object(input)
            .expect("manual object should expand")
            .to_string();
        assert!(!tokens.contains("pub fn new"));
    }

    #[test]
    fn interface_fields_with_ignored_attributes_are_rejected() {
        let input = syn::parse_quote! {
            #[webapi(name = "Sample")]
            struct BadInterface {
                #[webapi(enumerable)]
                ignored: (),
            }
        };
        let error = match expand_webapi_interface(input) {
            Ok(_) => panic!("ignored interface field attributes should be rejected"),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            "interface fields with #[webapi(...)] attributes must declare #[webapi(method)] or #[webapi(accessor_property)]"
        );
    }

    #[test]
    fn interface_fields_with_unsupported_installation_kinds_are_rejected() {
        for kind in ["data_property", "slot", "hidden"] {
            let input = syn::parse_str(&format!(
                r#"
                #[webapi(name = "Sample")]
                struct BadInterface {{
                    #[webapi({kind})]
                    ignored: (),
                }}
                "#
            ))
            .expect("parse interface");
            let error = match expand_webapi_interface(input) {
                Ok(_) => panic!("unsupported interface field kind should be rejected"),
                Err(error) => error,
            };
            assert_eq!(
                error.to_string(),
                "interface fields with #[webapi(...)] attributes must declare #[webapi(method)] or #[webapi(accessor_property)]"
            );
        }
    }

    #[test]
    fn function_template_fields_with_unsupported_installation_kinds_are_rejected() {
        for kind in [
            "data_property",
            "slot",
            "hidden",
            "prototype",
            "to_string_tag",
        ] {
            let input = syn::parse_str(&format!(
                r#"
                #[webapi(name = "Sample")]
                struct BadTemplate {{
                    #[webapi({kind})]
                    ignored: (),
                }}
                "#
            ))
            .expect("parse function template");
            let error = match expand_webapi_function_template(input) {
                Ok(_) => panic!("unsupported function template field kind should be rejected"),
                Err(error) => error,
            };
            assert_eq!(
                error.to_string(),
                "function template fields with #[webapi(...)] attributes must declare #[webapi(method)], #[webapi(static_method)], #[webapi(accessor_property)], #[webapi(native_data_property)], #[webapi(intrinsic_data_property = ...)], #[webapi(constant)], or #[webapi(alias = ...)]"
            );
        }
    }

    #[test]
    fn function_template_intrinsic_data_properties_are_supported() {
        let input = syn::parse_quote! {
            #[webapi(name = "Sample", enumerable)]
            struct SampleTemplate {
                #[webapi(
                    intrinsic_data_property = v8::Intrinsic::ArrayProtoValues
                )]
                values: (),

                #[webapi(
                    intrinsic_data_property = v8::Intrinsic::ArrayProtoValues,
                    symbol = "iterator",
                    readonly,
                    dont_delete
                )]
                iterator: (),
            }
        };
        expand_webapi_function_template(input)
            .expect("intrinsic data properties should expand on function templates");
    }

    #[test]
    fn function_template_intrinsic_prototype_parent_is_supported() {
        let input = syn::parse_quote! {
            #[webapi(
                name = "Example Iterator",
                intrinsic_prototype_parent = v8::Intrinsic::IteratorPrototype,
                prototype_to_string_tag = "Example Iterator",
                readonly_prototype
            )]
            struct IteratorTemplate {
                #[webapi(method, callback = sample_callback)]
                value: (),
            }
        };
        expand_webapi_function_template(input)
            .expect("intrinsic prototype parents should expand on function templates");
    }

    #[test]
    fn object_intrinsic_data_properties_are_rejected() {
        let input = syn::parse_quote! {
            #[webapi(interface = "Object")]
            struct BadObject {
                #[webapi(
                    intrinsic_data_property = v8::Intrinsic::ArrayProtoValues
                )]
                values: (),
            }
        };
        let error = match expand_webapi_object(input) {
            Ok(_) => panic!("runtime object intrinsic property should be rejected"),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            "`intrinsic_data_property` is only supported by WebApiFunctionTemplate"
        );
    }

    #[test]
    fn function_template_native_data_properties_are_supported() {
        let input = syn::parse_quote! {
            #[webapi(name = "Sample", enumerable)]
            struct SampleTemplate {
                #[webapi(native_data_property, getter = sample_getter)]
                value: (),

                #[webapi(native_data_property = "named", getter = sample_getter, setter = sample_setter, dont_delete)]
                named: (),
            }
        };
        expand_webapi_function_template(input)
            .expect("native accessors should expand on function templates");
    }

    #[test]
    fn function_template_accessor_property_fields_are_supported() {
        let input = syn::parse_quote! {
            #[webapi(name = "Sample", enumerable)]
            struct SampleTemplate {
                #[webapi(accessor_property, getter = sample_getter)]
                value: (),

                #[webapi(accessor_property = "named", getter = sample_getter, setter = sample_setter, data = getter_data, setter_data = setter_data, dont_delete)]
                named: (),
            }
        };
        let tokens = expand_webapi_function_template(input)
            .expect("template accessors should expand")
            .to_string();
        assert!(tokens.contains("String :: new (scope , \"get value\")"));
        assert!(tokens.contains("String :: new (scope , \"get named\")"));
        assert!(tokens.contains("String :: new (scope , \"set named\")"));
        assert!(!tokens.contains("format ! (\"get {}\""));
        assert!(!tokens.contains("format ! (\"set {}\""));
    }

    #[test]
    fn function_template_dynamic_accessor_names_are_formatted_at_runtime() {
        let input = syn::parse_quote! {
            #[webapi(name = "Sample")]
            struct SampleTemplate {
                #[webapi(accessor_property = DYNAMIC_NAME, getter = sample_getter, setter = sample_setter)]
                value: (),
            }
        };
        let tokens = expand_webapi_function_template(input)
            .expect("dynamic template accessor should expand")
            .to_string();
        assert!(tokens.contains("format ! (\"get {}\" , DYNAMIC_NAME)"));
        assert!(tokens.contains("format ! (\"set {}\" , DYNAMIC_NAME)"));
    }

    #[test]
    fn function_template_readonly_accessor_property_fields_are_rejected() {
        let input = syn::parse_quote! {
            #[webapi(name = "Sample")]
            struct BadTemplate {
                #[webapi(accessor_property, getter = sample_getter, readonly)]
                value: (),
            }
        };
        let error = match expand_webapi_function_template(input) {
            Ok(_) => panic!("readonly function-template accessor property should be rejected"),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            "function-template `accessor_property` fields have no writable attribute; omit #[webapi(setter)] instead of using readonly"
        );
    }

    #[test]
    fn object_readonly_accessor_property_fields_are_rejected() {
        let input = syn::parse_quote! {
            #[webapi(interface = "Sample")]
            struct BadObject {
                #[webapi(accessor_property, getter = sample_getter, readonly)]
                value: (),
            }
        };
        let error = match expand_webapi_object(input) {
            Ok(_) => panic!("readonly runtime-object accessor property should be rejected"),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            "runtime-object `accessor_property` fields have no writable attribute; omit #[webapi(setter)] instead of using readonly"
        );
    }

    #[test]
    fn object_accessor_property_rejects_callback_and_getter_value_together() {
        let input = syn::parse_quote! {
            #[webapi(interface = "Sample")]
            struct BadObject<'scope> {
                getter: ::moli_webapi_declare::v8::Local<'scope, ::moli_webapi_declare::v8::Function>,

                #[webapi(accessor_property, getter = sample_getter, getter_value = self.getter)]
                value: (),
            }
        };
        let error = match expand_webapi_object(input) {
            Ok(_) => panic!("getter and getter_value should be rejected together"),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            "`accessor_property` field cannot declare both #[webapi(getter = ...)] and #[webapi(getter_value = ...)]"
        );
    }

    #[test]
    fn object_native_data_property_fields_are_supported() {
        let input = syn::parse_quote! {
            #[webapi(interface = "Sample", enumerable)]
            struct SampleObject {
                #[webapi(native_data_property, getter = sample_getter)]
                value: (),

                #[webapi(native_data_property = "named", getter = sample_getter, setter = sample_setter, dont_delete)]
                named: (),
            }
        };
        expand_webapi_object(input).expect("native accessors should expand on objects");
    }

    #[test]
    fn object_readonly_native_data_property_fields_are_rejected() {
        let input = syn::parse_quote! {
            #[webapi(interface = "Sample")]
            struct BadObject {
                #[webapi(native_data_property, getter = sample_getter, readonly)]
                value: (),
            }
        };
        let error = match expand_webapi_object(input) {
            Ok(_) => panic!("readonly native data property should be rejected"),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            "`native_data_property` fields have no writable attribute; omit #[webapi(setter)] instead of using readonly"
        );
    }

    #[test]
    fn object_static_method_fields_are_rejected() {
        let input = syn::parse_quote! {
            #[webapi(interface = "Object", data_properties)]
            struct BadObject {
                #[webapi(static_method, callback = sample_callback)]
                create: (),
            }
        };
        let error = match expand_webapi_object(input) {
            Ok(_) => panic!("object static_method should be rejected"),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            "object fields with #[webapi(...)] attributes cannot declare #[webapi(static_method)]"
        );
    }

    #[test]
    fn object_constant_fields_are_supported() {
        let input = syn::parse_quote! {
            #[webapi(interface = "Object", data_properties)]
            struct BadObject {
                #[webapi(constant = "READY", value = 4u32)]
                ready: (),
            }
        };
        expand_webapi_object(input).expect("object constant should expand");
    }

    #[test]
    fn function_template_alias_source_must_be_earlier_string_method() {
        let input = syn::parse_quote! {
            #[webapi(name = "Sample")]
            struct BadTemplate {
                #[webapi(alias = "entries", symbol = "iterator")]
                iterator: (),

                #[webapi(method, callback = sample_callback)]
                entries: (),
            }
        };
        let error = match expand_webapi_function_template(input) {
            Ok(_) => panic!("out-of-order template alias should be rejected"),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            "function template alias source must refer to an earlier string-named method"
        );

        let input = syn::parse_quote! {
            #[webapi(name = "Sample")]
            struct BadTemplate {
                #[webapi(method, symbol = "iterator", callback = sample_callback)]
                entries: (),

                #[webapi(alias = "entries", symbol = "asyncIterator")]
                async_iterator: (),
            }
        };
        let error = match expand_webapi_function_template(input) {
            Ok(_) => panic!("symbol method source should not be a template alias source"),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            "function template alias source must refer to an earlier string-named method"
        );
    }
}
