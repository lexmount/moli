use syn::spanned::Spanned;
use syn::{Error, Expr, ExprLit, Field, Lit, LitInt, LitStr, Path, Token};

#[derive(Default)]
pub(crate) struct InterfaceAttrs {
    pub(crate) receiver: Option<Path>,
    pub(crate) name: Option<LitStr>,
    pub(crate) parent: Option<LitStr>,
    pub(crate) constructor: Option<ConstructorAttr>,
    pub(crate) constructor_length: Option<i32>,
    pub(crate) rename_all: RenameRule,
}

#[derive(Clone)]
pub(crate) enum ConstructorAttr {
    Illegal,
    Callback(Path),
}

#[derive(Clone)]
pub(crate) enum ValueInitAttr {
    Null,
    Object,
    NullObject,
    Array,
    Undefined,
    True,
    False,
    Zero,
    EmptyString,
    String(LitStr),
}

#[derive(Clone)]
pub(crate) enum ConstructorDefaultAttr {
    Default,
    Expr(Box<Expr>),
}

#[derive(Clone, Copy, Default)]
pub(crate) enum RenameRule {
    None,
    #[default]
    CamelCase,
}

#[derive(Default)]
pub(crate) struct ObjectAttrs {
    pub(crate) receiver: Option<Path>,
    pub(crate) interface: Option<LitStr>,
    pub(crate) parent: Option<LitStr>,
    pub(crate) prototype: Option<LitStr>,
    pub(crate) own_to_string_tag: Option<LitStr>,
    pub(crate) fallback_to_string_tag: Option<LitStr>,
    pub(crate) readonly_to_string_tag: bool,
    pub(crate) scope_lifetime: Option<syn::Lifetime>,
    pub(crate) require_prototype: bool,
    pub(crate) rename_all: RenameRule,
    pub(crate) allow_empty: bool,
    pub(crate) default_data_properties: bool,
    pub(crate) default_enumerable: bool,
    pub(crate) no_dynamic_constructor: bool,
    pub(crate) unbranded: bool,
}

#[derive(Default)]
pub(crate) struct FunctionTemplateAttrs {
    pub(crate) receiver: Option<Path>,
    pub(crate) name: Option<LitStr>,
    pub(crate) constructor: Option<ConstructorAttr>,
    pub(crate) constructor_length: Option<i32>,
    pub(crate) intrinsic_prototype_parent: Option<Expr>,
    pub(crate) prototype_to_string_tag: Option<LitStr>,
    pub(crate) readonly_prototype: bool,
    pub(crate) rename_all: RenameRule,
    pub(crate) default_enumerable: bool,
}

#[derive(Clone, Default)]
pub(crate) struct FieldAttrs {
    pub(crate) receiver: Option<Path>,
    pub(crate) returns_promise: bool,
    pub(crate) method: bool,
    pub(crate) static_method: bool,
    pub(crate) constant: bool,
    pub(crate) accessor_property: bool,
    pub(crate) native_data_property: bool,
    pub(crate) intrinsic_data_property: Option<Expr>,
    pub(crate) data_property: bool,
    pub(crate) alias: Option<LitStr>,
    pub(crate) enumerable: bool,
    pub(crate) hidden: bool,
    pub(crate) slot: bool,
    pub(crate) prototype: bool,
    pub(crate) to_string_tag: bool,
    pub(crate) readonly: bool,
    pub(crate) dont_delete: bool,
    pub(crate) name: Option<Expr>,
    pub(crate) symbol: Option<LitStr>,
    pub(crate) function_name: Option<LitStr>,
    pub(crate) length: Option<i32>,
    pub(crate) callback: Option<Path>,
    pub(crate) getter: Option<Path>,
    pub(crate) getter_value: Option<Expr>,
    pub(crate) setter: Option<Path>,
    pub(crate) data: Option<Expr>,
    pub(crate) setter_data: Option<Expr>,
    pub(crate) value: Option<Expr>,
    pub(crate) init: Option<ValueInitAttr>,
    pub(crate) constructor_default: Option<ConstructorDefaultAttr>,
}

impl FieldAttrs {
    pub(crate) fn inherit_receiver(&mut self, receiver: Option<&Path>) -> Result<(), Error> {
        if self.method || self.accessor_property {
            self.receiver = self.receiver.take().or_else(|| receiver.cloned());
        }
        if (self.receiver.is_some() || self.returns_promise)
            && let Some(getter_value) = &self.getter_value
        {
            return Err(Error::new(
                getter_value.span(),
                "receiver and returns_promise require a Rust callback, not getter_value",
            ));
        }
        Ok(())
    }

    pub(crate) fn has_installation_kind(&self) -> bool {
        self.method
            || self.static_method
            || self.constant
            || self.accessor_property
            || self.native_data_property
            || self.intrinsic_data_property.is_some()
            || self.data_property
            || self.alias.is_some()
            || self.hidden
            || self.slot
            || self.prototype
            || self.to_string_tag
    }

    pub(crate) fn has_installation_attribute(&self) -> bool {
        self.receiver.is_some()
            || self.returns_promise
            || self.enumerable
            || self.readonly
            || self.dont_delete
            || self.alias.is_some()
            || self.name.is_some()
            || self.symbol.is_some()
            || self.function_name.is_some()
            || self.length.is_some()
            || self.callback.is_some()
            || self.getter.is_some()
            || self.getter_value.is_some()
            || self.setter.is_some()
            || self.data.is_some()
            || self.setter_data.is_some()
            || self.value.is_some()
            || self.init.is_some()
    }
}

pub(crate) fn parse_interface_attrs(attrs: &[syn::Attribute]) -> Result<InterfaceAttrs, Error> {
    let mut parsed = InterfaceAttrs::default();
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("webapi")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("receiver") {
                parsed.receiver = Some(meta.value()?.parse()?);
                return Ok(());
            }
            if meta.path.is_ident("name") {
                parsed.name = Some(meta.value()?.parse()?);
                return Ok(());
            }
            if meta.path.is_ident("parent") {
                parsed.parent = Some(meta.value()?.parse()?);
                return Ok(());
            }
            if meta.path.is_ident("constructor") {
                let value: LitStr = meta.value()?.parse()?;
                parsed.constructor = Some(match value.value().as_str() {
                    "illegal" => ConstructorAttr::Illegal,
                    _ => return Err(Error::new(value.span(), "unsupported constructor kind")),
                });
                return Ok(());
            }
            if meta.path.is_ident("constructor_callback") {
                parsed.constructor = Some(ConstructorAttr::Callback(meta.value()?.parse()?));
                return Ok(());
            }
            if meta.path.is_ident("constructor_length") {
                let length: LitInt = meta.value()?.parse()?;
                parsed.constructor_length = Some(length.base10_parse()?);
                return Ok(());
            }
            if meta.path.is_ident("rename_all") {
                let value: LitStr = meta.value()?.parse()?;
                parsed.rename_all = parse_rename_rule(&value)?;
                return Ok(());
            }
            Err(meta.error("unsupported #[webapi(...)] interface attribute"))
        })?;
    }
    Ok(parsed)
}

pub(crate) fn parse_object_attrs(attrs: &[syn::Attribute]) -> Result<ObjectAttrs, Error> {
    let mut parsed = ObjectAttrs::default();
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("webapi")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("parent") {
                parsed.parent = Some(meta.value()?.parse()?);
                return Ok(());
            }
            if meta.path.is_ident("unbranded") {
                parsed.unbranded = true;
                return Ok(());
            }
            if meta.path.is_ident("receiver") {
                parsed.receiver = Some(meta.value()?.parse()?);
                return Ok(());
            }
            if meta.path.is_ident("interface") {
                parsed.interface = Some(meta.value()?.parse()?);
                return Ok(());
            }
            if meta.path.is_ident("prototype") {
                parsed.prototype = Some(meta.value()?.parse()?);
                return Ok(());
            }
            if meta.path.is_ident("own_to_string_tag") {
                parsed.own_to_string_tag = Some(meta.value()?.parse()?);
                return Ok(());
            }
            if meta.path.is_ident("fallback_to_string_tag") {
                parsed.fallback_to_string_tag = Some(meta.value()?.parse()?);
                return Ok(());
            }
            if meta.path.is_ident("readonly_to_string_tag") {
                parsed.readonly_to_string_tag = true;
                return Ok(());
            }
            if meta.path.is_ident("scope_lifetime") {
                parsed.scope_lifetime = Some(meta.value()?.parse()?);
                return Ok(());
            }
            if meta.path.is_ident("require_prototype") {
                parsed.require_prototype = true;
                return Ok(());
            }
            if meta.path.is_ident("rename_all") {
                let value: LitStr = meta.value()?.parse()?;
                parsed.rename_all = parse_rename_rule(&value)?;
                return Ok(());
            }
            if meta.path.is_ident("allow_empty") {
                parsed.allow_empty = true;
                return Ok(());
            }
            if meta.path.is_ident("data_properties") {
                parsed.default_data_properties = true;
                return Ok(());
            }
            if meta.path.is_ident("enumerable") {
                parsed.default_enumerable = true;
                return Ok(());
            }
            if meta.path.is_ident("no_dynamic_constructor") {
                parsed.no_dynamic_constructor = true;
                return Ok(());
            }
            Err(meta.error("unsupported #[webapi(...)] object attribute"))
        })?;
    }
    Ok(parsed)
}

pub(crate) fn parse_function_template_attrs(
    attrs: &[syn::Attribute],
) -> Result<FunctionTemplateAttrs, Error> {
    let mut parsed = FunctionTemplateAttrs::default();
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("webapi")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("receiver") {
                parsed.receiver = Some(meta.value()?.parse()?);
                return Ok(());
            }
            if meta.path.is_ident("name") {
                parsed.name = Some(meta.value()?.parse()?);
                return Ok(());
            }
            if meta.path.is_ident("constructor") {
                let value: LitStr = meta.value()?.parse()?;
                parsed.constructor = Some(match value.value().as_str() {
                    "illegal" => ConstructorAttr::Illegal,
                    _ => return Err(Error::new(value.span(), "unsupported constructor kind")),
                });
                return Ok(());
            }
            if meta.path.is_ident("constructor_callback") {
                parsed.constructor = Some(ConstructorAttr::Callback(meta.value()?.parse()?));
                return Ok(());
            }
            if meta.path.is_ident("constructor_length") {
                let length: LitInt = meta.value()?.parse()?;
                parsed.constructor_length = Some(length.base10_parse()?);
                return Ok(());
            }
            if meta.path.is_ident("intrinsic_prototype_parent") {
                parsed
                    .intrinsic_prototype_parent
                    .replace(meta.value()?.parse()?)
                    .is_none()
                    .then_some(())
                    .ok_or_else(|| {
                        meta.error(
                            "function template intrinsic_prototype_parent can only be specified once",
                        )
                    })?;
                return Ok(());
            }
            if meta.path.is_ident("prototype_to_string_tag") {
                parsed
                    .prototype_to_string_tag
                    .replace(meta.value()?.parse()?)
                    .is_none()
                    .then_some(())
                    .ok_or_else(|| {
                        meta.error(
                            "function template prototype_to_string_tag can only be specified once",
                        )
                    })?;
                return Ok(());
            }
            if meta.path.is_ident("readonly_prototype") {
                parsed.readonly_prototype = true;
                return Ok(());
            }
            if meta.path.is_ident("rename_all") {
                let value: LitStr = meta.value()?.parse()?;
                parsed.rename_all = parse_rename_rule(&value)?;
                return Ok(());
            }
            if meta.path.is_ident("enumerable") {
                parsed.default_enumerable = true;
                return Ok(());
            }
            Err(meta.error("unsupported #[webapi(...)] function template attribute"))
        })?;
    }
    Ok(parsed)
}

fn parse_rename_rule(value: &LitStr) -> Result<RenameRule, Error> {
    match value.value().as_str() {
        "none" => Ok(RenameRule::None),
        _ => Err(Error::new(
            value.span(),
            "unsupported rename_all rule; field names are camelCase by default, use rename_all = \"none\" only for explicit Rust spelling",
        )),
    }
}

pub(crate) fn parse_field_attrs(field: &Field) -> Result<FieldAttrs, Error> {
    let mut parsed = FieldAttrs::default();
    for attr in field
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("webapi"))
    {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("receiver") {
                if parsed.receiver.replace(meta.value()?.parse()?).is_some() {
                    return Err(meta.error("field receiver can only be specified once"));
                }
                return Ok(());
            }
            if meta.path.is_ident("returns_promise") {
                parsed.returns_promise = true;
                return Ok(());
            }
            if meta.path.is_ident("method") {
                parsed.method = true;
                if meta.input.peek(Token![=]) {
                    set_field_name(&mut parsed.name, meta.value()?.parse()?, meta.path.span())?;
                }
                return Ok(());
            }
            if meta.path.is_ident("static_method") {
                parsed.static_method = true;
                if meta.input.peek(Token![=]) {
                    set_field_name(&mut parsed.name, meta.value()?.parse()?, meta.path.span())?;
                }
                return Ok(());
            }
            if meta.path.is_ident("constant") {
                parsed.constant = true;
                if meta.input.peek(Token![=]) {
                    set_field_name(&mut parsed.name, meta.value()?.parse()?, meta.path.span())?;
                }
                return Ok(());
            }
            if meta.path.is_ident("accessor_property") {
                parsed.accessor_property = true;
                if meta.input.peek(Token![=]) {
                    set_field_name(&mut parsed.name, meta.value()?.parse()?, meta.path.span())?;
                }
                return Ok(());
            }
            if meta.path.is_ident("native_data_property") {
                parsed.native_data_property = true;
                if meta.input.peek(Token![=]) {
                    set_field_name(&mut parsed.name, meta.value()?.parse()?, meta.path.span())?;
                }
                return Ok(());
            }
            if meta.path.is_ident("intrinsic_data_property") {
                if parsed
                    .intrinsic_data_property
                    .replace(meta.value()?.parse()?)
                    .is_some()
                {
                    return Err(
                        meta.error("field intrinsic_data_property can only be specified once")
                    );
                }
                return Ok(());
            }
            if meta.path.is_ident("data_property") {
                parsed.data_property = true;
                if meta.input.peek(Token![=]) {
                    set_field_name(&mut parsed.name, meta.value()?.parse()?, meta.path.span())?;
                }
                return Ok(());
            }
            if meta.path.is_ident("alias") {
                if parsed.alias.replace(meta.value()?.parse()?).is_some() {
                    return Err(meta.error("field alias can only be specified once"));
                }
                return Ok(());
            }
            if meta.path.is_ident("enumerable") {
                parsed.enumerable = true;
                return Ok(());
            }
            if meta.path.is_ident("hidden") {
                parsed.hidden = true;
                if meta.input.peek(Token![=]) {
                    set_field_name(&mut parsed.name, meta.value()?.parse()?, meta.path.span())?;
                }
                return Ok(());
            }
            if meta.path.is_ident("slot") {
                parsed.slot = true;
                if meta.input.peek(Token![=]) {
                    set_field_name(&mut parsed.name, meta.value()?.parse()?, meta.path.span())?;
                }
                return Ok(());
            }
            if meta.path.is_ident("prototype") {
                if meta.input.peek(Token![=]) {
                    return Err(meta.error("field prototype uses #[webapi(prototype)]"));
                }
                parsed.prototype = true;
                return Ok(());
            }
            if meta.path.is_ident("to_string_tag") {
                if meta.input.peek(Token![=]) {
                    return Err(meta.error("field to_string_tag uses #[webapi(to_string_tag)]"));
                }
                parsed.to_string_tag = true;
                return Ok(());
            }
            if meta.path.is_ident("readonly") {
                parsed.readonly = true;
                return Ok(());
            }
            if meta.path.is_ident("dont_delete") {
                parsed.dont_delete = true;
                return Ok(());
            }
            if meta.path.is_ident("name") {
                set_field_name(&mut parsed.name, meta.value()?.parse()?, meta.path.span())?;
                return Ok(());
            }
            if meta.path.is_ident("symbol") {
                if parsed.symbol.replace(meta.value()?.parse()?).is_some() {
                    return Err(meta.error("field symbol can only be specified once"));
                }
                return Ok(());
            }
            if meta.path.is_ident("function_name") {
                if parsed
                    .function_name
                    .replace(meta.value()?.parse()?)
                    .is_some()
                {
                    return Err(meta.error("field function_name can only be specified once"));
                }
                return Ok(());
            }
            if meta.path.is_ident("length") {
                let length: LitInt = meta.value()?.parse()?;
                parsed.length = Some(length.base10_parse()?);
                return Ok(());
            }
            if meta.path.is_ident("callback") {
                parsed.callback = Some(meta.value()?.parse()?);
                return Ok(());
            }
            if meta.path.is_ident("getter") {
                if parsed.getter.replace(meta.value()?.parse()?).is_some() {
                    return Err(meta.error("field getter can only be specified once"));
                }
                return Ok(());
            }
            if meta.path.is_ident("getter_value") {
                if parsed
                    .getter_value
                    .replace(meta.value()?.parse()?)
                    .is_some()
                {
                    return Err(meta.error("field getter_value can only be specified once"));
                }
                return Ok(());
            }
            if meta.path.is_ident("setter") {
                if parsed.setter.replace(meta.value()?.parse()?).is_some() {
                    return Err(meta.error("field setter can only be specified once"));
                }
                return Ok(());
            }
            if meta.path.is_ident("data") {
                if parsed.data.replace(meta.value()?.parse()?).is_some() {
                    return Err(meta.error("field data can only be specified once"));
                }
                return Ok(());
            }
            if meta.path.is_ident("setter_data") {
                if parsed.setter_data.replace(meta.value()?.parse()?).is_some() {
                    return Err(meta.error("field setter_data can only be specified once"));
                }
                return Ok(());
            }
            if meta.path.is_ident("value") {
                parsed.value = Some(meta.value()?.parse()?);
                return Ok(());
            }
            if meta.path.is_ident("init") {
                let value: Expr = meta.value()?.parse()?;
                parsed.init = Some(parse_value_init(value)?);
                return Ok(());
            }
            if meta.path.is_ident("constructor_default") {
                let default = if meta.input.peek(Token![=]) {
                    ConstructorDefaultAttr::Expr(Box::new(meta.value()?.parse()?))
                } else {
                    ConstructorDefaultAttr::Default
                };
                if parsed.constructor_default.replace(default).is_some() {
                    return Err(meta.error("field constructor_default can only be specified once"));
                }
                return Ok(());
            }
            Err(meta.error("unsupported #[webapi(...)] field attribute"))
        })?;
    }

    if parsed.receiver.is_some() && !parsed.method && !parsed.accessor_property {
        return Err(Error::new(
            field.span(),
            "receiver is only supported on instance methods and accessor_property fields",
        ));
    }
    if parsed.returns_promise
        && !parsed.method
        && !parsed.static_method
        && !parsed.accessor_property
    {
        return Err(Error::new(
            field.span(),
            "returns_promise is only supported on methods and accessor_property getters",
        ));
    }
    parsed.inherit_receiver(None)?;

    let kinds = [
        parsed.method,
        parsed.static_method,
        parsed.constant,
        parsed.accessor_property,
        parsed.native_data_property,
        parsed.intrinsic_data_property.is_some(),
        parsed.data_property,
        parsed.alias.is_some(),
        parsed.hidden,
        parsed.slot,
        parsed.prototype,
        parsed.to_string_tag,
    ]
    .into_iter()
    .filter(|enabled| *enabled)
    .count();
    if kinds > 1 {
        return Err(Error::new(
            field.span(),
            "field can only be one of #[webapi(method)], #[webapi(static_method)], #[webapi(constant)], #[webapi(accessor_property)], #[webapi(native_data_property)], #[webapi(intrinsic_data_property = ...)], #[webapi(data_property)], #[webapi(alias = ...)], #[webapi(hidden)], #[webapi(slot)], #[webapi(prototype)], or #[webapi(to_string_tag)]",
        ));
    }
    if parsed.value.is_some() && parsed.init.is_some() {
        return Err(Error::new(
            field.span(),
            "field can only define one of #[webapi(value = expr)] or #[webapi(init = \"...\")]",
        ));
    }
    if parsed.data.is_some()
        && !parsed.method
        && !parsed.static_method
        && !parsed.accessor_property
        && !parsed.native_data_property
    {
        return Err(Error::new(
            field.span(),
            "field data can only be specified for #[webapi(method)], #[webapi(static_method)], #[webapi(accessor_property)], or #[webapi(native_data_property)] fields",
        ));
    }
    if parsed.setter_data.is_some() && !parsed.accessor_property {
        return Err(Error::new(
            field.span(),
            "field setter_data can only be specified for #[webapi(accessor_property)] fields",
        ));
    }
    if parsed.setter_data.is_some() && parsed.setter.is_none() {
        return Err(Error::new(
            field.span(),
            "field setter_data requires #[webapi(setter = path)]",
        ));
    }
    if let Some(function_name) = parsed.function_name.as_ref() {
        if !parsed.method && !parsed.static_method {
            return Err(Error::new(
                field.span(),
                "field function_name can only be specified for #[webapi(method)] or #[webapi(static_method)] fields",
            ));
        }
        if function_name.value().is_empty() {
            return Err(Error::new(
                function_name.span(),
                "field function_name cannot be empty",
            ));
        }
    }
    if parsed.symbol.is_some() {
        if parsed.name.is_some() {
            return Err(Error::new(
                field.span(),
                "field can only specify one of #[webapi(name = ...)] or #[webapi(symbol = ...)]",
            ));
        }
        if !parsed.method
            && !parsed.static_method
            && !parsed.accessor_property
            && !parsed.native_data_property
            && parsed.intrinsic_data_property.is_none()
            && parsed.alias.is_none()
        {
            return Err(Error::new(
                field.span(),
                "field symbol can only be specified for #[webapi(method)], #[webapi(static_method)], #[webapi(accessor_property)], #[webapi(native_data_property)], #[webapi(intrinsic_data_property = ...)], or #[webapi(alias = ...)] fields",
            ));
        }
    }
    if parsed.getter.is_some() && !parsed.accessor_property && !parsed.native_data_property {
        return Err(Error::new(
            field.span(),
            "field getter can only be specified for #[webapi(accessor_property)] or #[webapi(native_data_property)] fields",
        ));
    }
    if parsed.setter.is_some() && !parsed.accessor_property && !parsed.native_data_property {
        return Err(Error::new(
            field.span(),
            "field setter can only be specified for #[webapi(accessor_property)] or #[webapi(native_data_property)] fields",
        ));
    }
    if parsed.accessor_property && parsed.callback.is_some() {
        return Err(Error::new(
            field.span(),
            "`accessor_property` fields use #[webapi(getter = path)] and optional #[webapi(setter = path)] instead of callback",
        ));
    }
    if parsed.accessor_property
        && (parsed.length.is_some() || parsed.value.is_some() || parsed.init.is_some())
    {
        return Err(Error::new(
            field.span(),
            "`accessor_property` fields cannot use length, value, or init attributes",
        ));
    }
    if parsed.native_data_property && parsed.callback.is_some() {
        return Err(Error::new(
            field.span(),
            "`native_data_property` fields use #[webapi(getter = path)] and optional #[webapi(setter = path)] instead of callback",
        ));
    }
    if parsed.native_data_property
        && (parsed.length.is_some() || parsed.value.is_some() || parsed.init.is_some())
    {
        return Err(Error::new(
            field.span(),
            "`native_data_property` fields cannot use length, value, or init attributes",
        ));
    }
    if parsed.intrinsic_data_property.is_some()
        && (parsed.function_name.is_some()
            || parsed.length.is_some()
            || parsed.callback.is_some()
            || parsed.getter.is_some()
            || parsed.getter_value.is_some()
            || parsed.setter.is_some()
            || parsed.data.is_some()
            || parsed.setter_data.is_some()
            || parsed.value.is_some()
            || parsed.init.is_some())
    {
        return Err(Error::new(
            field.span(),
            "`intrinsic_data_property` fields can only use name, symbol, enumerable, readonly, or dont_delete attributes",
        ));
    }
    if (parsed.method || parsed.static_method)
        && (parsed.getter.is_some()
            || parsed.setter.is_some()
            || parsed.value.is_some()
            || parsed.init.is_some())
    {
        return Err(Error::new(
            field.span(),
            "method fields cannot use getter, setter, value, or init attributes",
        ));
    }
    if parsed.constant
        && (parsed.symbol.is_some()
            || parsed.function_name.is_some()
            || parsed.length.is_some()
            || parsed.callback.is_some()
            || parsed.getter.is_some()
            || parsed.setter.is_some()
            || parsed.data.is_some()
            || parsed.setter_data.is_some()
            || parsed.init.is_some()
            || parsed.readonly
            || parsed.dont_delete)
    {
        return Err(Error::new(
            field.span(),
            "constant fields can only use name, value, and optional enumerable attributes",
        ));
    }
    if parsed.constant && parsed.value.is_none() {
        return Err(Error::new(
            field.span(),
            "constant field requires #[webapi(value = expr)]",
        ));
    }
    if parsed.alias.is_some()
        && (parsed.callback.is_some()
            || parsed.function_name.is_some()
            || parsed.length.is_some()
            || parsed.getter.is_some()
            || parsed.setter.is_some()
            || parsed.data.is_some()
            || parsed.setter_data.is_some()
            || parsed.value.is_some()
            || parsed.init.is_some())
    {
        return Err(Error::new(
            field.span(),
            "alias fields cannot use callback, length, getter, setter, data, value, or init attributes",
        ));
    }
    if parsed.data_property
        && (parsed.callback.is_some()
            || parsed.length.is_some()
            || parsed.getter.is_some()
            || parsed.setter.is_some())
    {
        return Err(Error::new(
            field.span(),
            "`data_property` fields cannot use callback, length, getter, or setter attributes",
        ));
    }
    if parsed.hidden
        && (parsed.callback.is_some()
            || parsed.length.is_some()
            || parsed.getter.is_some()
            || parsed.setter.is_some()
            || parsed.setter_data.is_some()
            || parsed.enumerable)
    {
        return Err(Error::new(
            field.span(),
            "hidden fields cannot use callback, length, getter, setter, or enumerable attributes",
        ));
    }
    if parsed.slot
        && (parsed.callback.is_some()
            || parsed.length.is_some()
            || parsed.getter.is_some()
            || parsed.setter.is_some()
            || parsed.setter_data.is_some()
            || parsed.enumerable
            || parsed.readonly
            || parsed.dont_delete)
    {
        return Err(Error::new(
            field.span(),
            "slot fields cannot use callback, length, getter, setter, enumerable, readonly, or dont_delete attributes",
        ));
    }
    if parsed.prototype
        && (parsed.name.is_some()
            || parsed.symbol.is_some()
            || parsed.function_name.is_some()
            || parsed.length.is_some()
            || parsed.callback.is_some()
            || parsed.getter.is_some()
            || parsed.setter.is_some()
            || parsed.data.is_some()
            || parsed.setter_data.is_some()
            || parsed.enumerable
            || parsed.readonly
            || parsed.dont_delete)
    {
        return Err(Error::new(
            field.span(),
            "prototype fields can only use value, init, or optional field state",
        ));
    }
    if parsed.to_string_tag
        && (parsed.name.is_some()
            || parsed.symbol.is_some()
            || parsed.function_name.is_some()
            || parsed.length.is_some()
            || parsed.callback.is_some()
            || parsed.getter.is_some()
            || parsed.setter.is_some()
            || parsed.data.is_some()
            || parsed.setter_data.is_some()
            || parsed.enumerable)
    {
        return Err(Error::new(
            field.span(),
            "to_string_tag fields can only use value, init, readonly, dont_delete, or optional field state",
        ));
    }

    Ok(parsed)
}

fn parse_value_init(value: Expr) -> Result<ValueInitAttr, Error> {
    match value {
        Expr::Call(call) if expr_path_is_ident(call.func.as_ref(), "string") => {
            let span = call.span();
            let mut args = call.args.into_iter();
            let Some(arg) = args.next() else {
                return Err(Error::new(span, "string initializer requires one literal"));
            };
            if args.next().is_some() {
                return Err(Error::new(span, "string initializer requires one literal"));
            }
            match arg {
                Expr::Lit(ExprLit {
                    lit: Lit::Str(value),
                    ..
                }) => Ok(ValueInitAttr::String(value)),
                other => Err(Error::new(
                    other.span(),
                    "string initializer requires a string literal",
                )),
            }
        }
        Expr::Lit(ExprLit {
            lit: Lit::Str(value),
            ..
        }) => match value.value().as_str() {
            "null" => Ok(ValueInitAttr::Null),
            "object" => Ok(ValueInitAttr::Object),
            "null_object" => Ok(ValueInitAttr::NullObject),
            "array" => Ok(ValueInitAttr::Array),
            "undefined" => Ok(ValueInitAttr::Undefined),
            "true" => Ok(ValueInitAttr::True),
            "false" => Ok(ValueInitAttr::False),
            "zero" => Ok(ValueInitAttr::Zero),
            "empty_string" | "" => Ok(ValueInitAttr::EmptyString),
            _ => Err(Error::new(value.span(), "unsupported value initializer")),
        },
        Expr::Lit(ExprLit {
            lit: Lit::Bool(value),
            ..
        }) => Ok(if value.value {
            ValueInitAttr::True
        } else {
            ValueInitAttr::False
        }),
        Expr::Lit(ExprLit {
            lit: Lit::Int(value),
            ..
        }) if value.base10_digits() == "0" => Ok(ValueInitAttr::Zero),
        Expr::Lit(ExprLit {
            lit: Lit::Float(value),
            ..
        }) if value.base10_parse::<f64>().ok() == Some(0.0) => Ok(ValueInitAttr::Zero),
        _ => Err(Error::new(value.span(), "unsupported value initializer")),
    }
}

fn expr_path_is_ident(expr: &Expr, ident: &str) -> bool {
    matches!(expr, Expr::Path(path) if path.path.is_ident(ident))
}

fn set_field_name(
    target: &mut Option<Expr>,
    name: Expr,
    span: proc_macro2::Span,
) -> Result<(), Error> {
    if target.replace(name).is_some() {
        return Err(Error::new(span, "field name can only be specified once"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        RenameRule, ValueInitAttr, parse_field_attrs, parse_function_template_attrs,
        parse_object_attrs,
    };
    use syn::Field;

    #[test]
    fn object_enumerable_default_can_be_used_without_default_data_properties() {
        let attrs: Vec<syn::Attribute> = syn::parse_quote! {
            #[webapi(interface = "Object", enumerable)]
        };
        let attrs = parse_object_attrs(&attrs)
            .expect("enumerable default should parse without data properties");
        assert!(attrs.default_enumerable);
        assert!(!attrs.default_data_properties);
    }

    #[test]
    fn object_data_properties_default_is_parsed() {
        let attrs: Vec<syn::Attribute> = syn::parse_quote! {
            #[webapi(interface = "Object", data_properties)]
        };
        let attrs = parse_object_attrs(&attrs).expect("data-properties default should parse");
        assert!(attrs.default_data_properties);
    }

    #[test]
    fn rename_all_none_is_the_only_explicit_rename_rule() {
        let attrs: Vec<syn::Attribute> = syn::parse_quote! {
            #[webapi(interface = "Object", rename_all = "none")]
        };
        let attrs = parse_object_attrs(&attrs).expect("rename_all none should parse");
        assert!(matches!(attrs.rename_all, RenameRule::None));

        let attrs: Vec<syn::Attribute> = syn::parse_quote! {
            #[webapi(interface = "Object", rename_all = "camelCase")]
        };
        let error = match parse_object_attrs(&attrs) {
            Ok(_) => panic!("explicit camelCase rename_all should be rejected"),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            "unsupported rename_all rule; field names are camelCase by default, use rename_all = \"none\" only for explicit Rust spelling"
        );
    }

    #[test]
    fn readonly_accessor_property_attribute_is_parsed() {
        let field = syn::parse_quote! {
            #[webapi(accessor_property, readonly, getter = sample_getter)]
            value: ()
        };
        let attrs = parse_field_attrs(&field).expect("readonly accessor property should parse");
        assert!(attrs.accessor_property);
        assert!(attrs.readonly);
    }

    #[test]
    fn accessor_property_setter_data_attribute_is_parsed() {
        let field = syn::parse_quote! {
            #[webapi(accessor_property, getter = sample_getter, setter = sample_setter, data = getter_data, setter_data = setter_data)]
            value: ()
        };
        let attrs = parse_field_attrs(&field).expect("setter_data accessor property should parse");
        assert!(attrs.accessor_property);
        assert!(attrs.data.is_some());
        assert!(attrs.setter_data.is_some());
    }

    #[test]
    fn literal_init_attributes_are_parsed() {
        let field = syn::parse_quote! {
            #[webapi(data_property, init = true)]
            value: ()
        };
        let attrs = parse_field_attrs(&field).expect("literal true init should parse");
        assert!(matches!(attrs.init, Some(ValueInitAttr::True)));

        let field = syn::parse_quote! {
            #[webapi(data_property, init = false)]
            value: ()
        };
        let attrs = parse_field_attrs(&field).expect("literal false init should parse");
        assert!(matches!(attrs.init, Some(ValueInitAttr::False)));

        let field = syn::parse_quote! {
            #[webapi(data_property, init = 0)]
            value: ()
        };
        let attrs = parse_field_attrs(&field).expect("literal zero init should parse");
        assert!(matches!(attrs.init, Some(ValueInitAttr::Zero)));

        let field = syn::parse_quote! {
            #[webapi(data_property, init = "")]
            value: ()
        };
        let attrs = parse_field_attrs(&field).expect("empty string init should parse");
        assert!(matches!(attrs.init, Some(ValueInitAttr::EmptyString)));

        let field = syn::parse_quote! {
            #[webapi(data_property, init = string("none"))]
            value: ()
        };
        let attrs = parse_field_attrs(&field).expect("literal string init should parse");
        assert!(
            matches!(attrs.init, Some(ValueInitAttr::String(value)) if value.value() == "none")
        );
    }

    #[test]
    fn setter_data_without_setter_is_rejected() {
        let field = syn::parse_quote! {
            #[webapi(accessor_property, getter = sample_getter, setter_data = setter_data)]
            value: ()
        };
        let error = match parse_field_attrs(&field) {
            Ok(_) => panic!("setter_data without setter should be rejected"),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            "field setter_data requires #[webapi(setter = path)]"
        );
    }

    #[test]
    fn symbol_name_conflict_is_rejected() {
        let field = syn::parse_quote! {
            #[webapi(method, name = "named", symbol = "iterator", callback = sample_callback)]
            value: ()
        };
        let error = match parse_field_attrs(&field) {
            Ok(_) => panic!("name + symbol should be rejected"),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            "field can only specify one of #[webapi(name = ...)] or #[webapi(symbol = ...)]"
        );
    }

    #[test]
    fn symbol_on_non_callback_field_is_rejected() {
        let field = syn::parse_quote! {
            #[webapi(data_property, symbol = "iterator")]
            value: ()
        };
        let error = match parse_field_attrs(&field) {
            Ok(_) => panic!("symbol on a data_property field should be rejected"),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            "field symbol can only be specified for #[webapi(method)], #[webapi(static_method)], #[webapi(accessor_property)], #[webapi(native_data_property)], #[webapi(intrinsic_data_property = ...)], or #[webapi(alias = ...)] fields"
        );
    }

    #[test]
    fn removed_function_accessor_attribute_is_rejected() {
        let field = syn::parse_quote! {
            #[webapi(function_accessor, getter = sample_getter)]
            value: ()
        };
        let error = match parse_field_attrs(&field) {
            Ok(_) => panic!("removed function_accessor spelling should be rejected"),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            "unsupported #[webapi(...)] field attribute"
        );
    }

    #[test]
    fn removed_property_attribute_spellings_are_rejected() {
        let fields: [Field; 3] = [
            syn::parse_quote! {
                #[webapi(property)]
                value: ()
            },
            syn::parse_quote! {
                #[webapi(accessor, getter = sample_getter)]
                value: ()
            },
            syn::parse_quote! {
                #[webapi(native_accessor, getter = sample_getter)]
                value: ()
            },
        ];

        for field in fields {
            let error = match parse_field_attrs(&field) {
                Ok(_) => panic!("removed property spelling should be rejected"),
                Err(error) => error,
            };
            assert_eq!(
                error.to_string(),
                "unsupported #[webapi(...)] field attribute"
            );
        }
    }

    #[test]
    fn removed_object_properties_spelling_is_rejected() {
        let attrs: Vec<syn::Attribute> = syn::parse_quote! {
            #[webapi(interface = "Object", properties)]
        };
        let error = match parse_object_attrs(&attrs) {
            Ok(_) => panic!("removed object properties spelling should be rejected"),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            "unsupported #[webapi(...)] object attribute"
        );
    }

    #[test]
    fn native_data_property_attribute_is_parsed() {
        let field = syn::parse_quote! {
            #[webapi(native_data_property = "value", enumerable, getter = sample_getter, setter = sample_setter)]
            value: ()
        };
        let attrs = parse_field_attrs(&field).expect("native data property should parse");
        assert!(attrs.native_data_property);
        assert!(attrs.enumerable);
        assert!(attrs.getter.is_some());
        assert!(attrs.setter.is_some());
    }

    #[test]
    fn intrinsic_data_property_attribute_is_parsed() {
        let field = syn::parse_quote! {
            #[webapi(
                intrinsic_data_property = v8::Intrinsic::ArrayProtoValues,
                symbol = "iterator",
                readonly,
                dont_delete
            )]
            iterator: ()
        };
        let attrs = parse_field_attrs(&field).expect("intrinsic data property should parse");
        assert!(attrs.intrinsic_data_property.is_some());
        assert_eq!(
            attrs
                .symbol
                .as_ref()
                .map(|symbol| symbol.value())
                .as_deref(),
            Some("iterator")
        );
        assert!(attrs.readonly);
        assert!(attrs.dont_delete);
    }

    #[test]
    fn intrinsic_data_property_callback_attributes_are_rejected() {
        let field = syn::parse_quote! {
            #[webapi(
                intrinsic_data_property = v8::Intrinsic::ArrayProtoValues,
                callback = sample_callback
            )]
            values: ()
        };
        let error = match parse_field_attrs(&field) {
            Ok(_) => panic!("callback on intrinsic data property should be rejected"),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            "`intrinsic_data_property` fields can only use name, symbol, enumerable, readonly, or dont_delete attributes"
        );
    }

    #[test]
    fn intrinsic_prototype_parent_attribute_is_parsed() {
        let attrs: Vec<syn::Attribute> = syn::parse_quote! {
            #[webapi(
                name = "ErrorLike",
                intrinsic_prototype_parent = v8::Intrinsic::ErrorPrototype
            )]
        };
        let attrs =
            parse_function_template_attrs(&attrs).expect("intrinsic prototype parent should parse");
        assert!(attrs.intrinsic_prototype_parent.is_some());
    }

    #[test]
    fn iterator_prototype_shape_attributes_are_parsed() {
        let attrs: Vec<syn::Attribute> = syn::parse_quote! {
            #[webapi(
                name = "Example Iterator",
                intrinsic_prototype_parent = v8::Intrinsic::IteratorPrototype,
                prototype_to_string_tag = "Example Iterator",
                readonly_prototype
            )]
        };
        let attrs =
            parse_function_template_attrs(&attrs).expect("iterator prototype shape should parse");
        assert_eq!(
            attrs
                .prototype_to_string_tag
                .as_ref()
                .map(|tag| tag.value())
                .as_deref(),
            Some("Example Iterator")
        );
        assert!(attrs.readonly_prototype);
    }

    #[test]
    fn method_descriptor_attributes_are_allowed() {
        let field = syn::parse_quote! {
            #[webapi(method, callback = sample_callback, readonly, dont_delete)]
            value: ()
        };
        let attrs = parse_field_attrs(&field).expect("method descriptor attrs should parse");
        assert!(attrs.method);
        assert!(attrs.readonly);
        assert!(attrs.dont_delete);
    }

    #[test]
    fn method_function_name_attribute_is_allowed() {
        let field = syn::parse_quote! {
            #[webapi(method, symbol = "iterator", function_name = "values", callback = sample_callback)]
            value: ()
        };
        let attrs = parse_field_attrs(&field).expect("method function_name attr should parse");
        assert_eq!(
            attrs.function_name.as_ref().map(|name| name.value()),
            Some("values".to_string())
        );
    }

    #[test]
    fn function_name_on_non_method_field_is_rejected() {
        let field = syn::parse_quote! {
            #[webapi(accessor_property, function_name = "value", getter = sample_getter)]
            value: ()
        };
        let error = match parse_field_attrs(&field) {
            Ok(_) => panic!("function_name on accessor_property should be rejected"),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            "field function_name can only be specified for #[webapi(method)] or #[webapi(static_method)] fields"
        );
    }

    #[test]
    fn native_data_property_callback_attribute_is_rejected() {
        let field = syn::parse_quote! {
            #[webapi(native_data_property, callback = sample_callback)]
            value: ()
        };
        let error = match parse_field_attrs(&field) {
            Ok(_) => panic!("callback on native_data_property should be rejected"),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            "`native_data_property` fields use #[webapi(getter = path)] and optional #[webapi(setter = path)] instead of callback"
        );
    }

    #[test]
    fn constant_without_value_is_rejected() {
        let field = syn::parse_quote! {
            #[webapi(constant = "READY")]
            ready: ()
        };
        let error = match parse_field_attrs(&field) {
            Ok(_) => panic!("constant without a value should be rejected"),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            "constant field requires #[webapi(value = expr)]"
        );
    }

    #[test]
    fn constant_callback_attributes_are_rejected() {
        let field = syn::parse_quote! {
            #[webapi(constant = "READY", value = 4u32, callback = sample_callback)]
            ready: ()
        };
        let error = match parse_field_attrs(&field) {
            Ok(_) => panic!("constant callback should be rejected"),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            "constant fields can only use name, value, and optional enumerable attributes"
        );
    }

    #[test]
    fn data_property_callback_attributes_are_rejected() {
        let field = syn::parse_quote! {
            #[webapi(data_property, callback = sample_callback)]
            value: ()
        };
        let error = match parse_field_attrs(&field) {
            Ok(_) => panic!("callback on data_property should be rejected"),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            "`data_property` fields cannot use callback, length, getter, or setter attributes"
        );
    }

    #[test]
    fn hidden_enumerable_attribute_is_rejected() {
        let field = syn::parse_quote! {
            #[webapi(hidden, enumerable)]
            value: ()
        };
        let error = match parse_field_attrs(&field) {
            Ok(_) => panic!("enumerable hidden field should be rejected"),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            "hidden fields cannot use callback, length, getter, setter, or enumerable attributes"
        );
    }

    #[test]
    fn slot_descriptor_attributes_are_rejected() {
        let field = syn::parse_quote! {
            #[webapi(slot, readonly)]
            value: ()
        };
        let error = match parse_field_attrs(&field) {
            Ok(_) => panic!("readonly slot should be rejected"),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            "slot fields cannot use callback, length, getter, setter, enumerable, readonly, or dont_delete attributes"
        );
    }

    #[test]
    fn prototype_installation_attributes_are_rejected() {
        let field = syn::parse_quote! {
            #[webapi(prototype, name = "ignored")]
            value: ()
        };
        let error = match parse_field_attrs(&field) {
            Ok(_) => panic!("name on prototype field should be rejected"),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            "prototype fields can only use value, init, or optional field state"
        );
    }

    #[test]
    fn to_string_tag_descriptor_attributes_are_allowed() {
        let field = syn::parse_quote! {
            #[webapi(to_string_tag, readonly, dont_delete, value = "Sample")]
            value: ()
        };
        let attrs = parse_field_attrs(&field).expect("toStringTag attrs should parse");
        assert!(attrs.to_string_tag);
        assert!(attrs.readonly);
        assert!(attrs.dont_delete);
    }

    #[test]
    fn to_string_tag_enumerable_attribute_is_rejected() {
        let field = syn::parse_quote! {
            #[webapi(to_string_tag, enumerable, value = "Sample")]
            value: ()
        };
        let error = match parse_field_attrs(&field) {
            Ok(_) => panic!("enumerable toStringTag should be rejected"),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            "to_string_tag fields can only use value, init, readonly, dont_delete, or optional field state"
        );
    }
}
