use syn::ext::IdentExt;
use syn::spanned::Spanned;
use syn::{Error, Expr, Field, Ident, LitInt, LitStr, Path};

#[derive(Default)]
pub(crate) struct ContainerAttrs {
    pub(crate) prefix: Option<LitStr>,
    pub(crate) scope_lifetime: Option<syn::Lifetime>,
    pub(crate) rename_all: Option<RenameRule>,
}

pub(crate) fn parse_container_attrs(attrs: &[syn::Attribute]) -> Result<ContainerAttrs, Error> {
    let mut parsed = ContainerAttrs::default();
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("webidl")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("prefix") {
                parsed.prefix = Some(meta.value()?.parse()?);
                return Ok(());
            }
            if meta.path.is_ident("scope_lifetime") {
                parsed.scope_lifetime = Some(meta.value()?.parse()?);
                return Ok(());
            }
            if meta.path.is_ident("rename_all") {
                let value: LitStr = meta.value()?.parse()?;
                parsed.rename_all = Some(parse_rename_rule(&value)?);
                return Ok(());
            }
            Err(meta.error("unsupported #[webidl(...)] container attribute"))
        })?;
    }
    Ok(parsed)
}

#[derive(Clone, Copy, Default)]
pub(crate) enum RenameRule {
    None,
    #[default]
    CamelCase,
    KebabCase,
    Lowercase,
}

pub(crate) fn parse_rename_rule(value: &LitStr) -> Result<RenameRule, Error> {
    match value.value().as_str() {
        "camelCase" => Ok(RenameRule::CamelCase),
        "kebab-case" => Ok(RenameRule::KebabCase),
        "lowercase" => Ok(RenameRule::Lowercase),
        "none" => Ok(RenameRule::None),
        _ => Err(Error::new(value.span(), "unsupported rename_all rule")),
    }
}

#[derive(Default)]
pub(crate) struct FieldAttrs {
    pub(crate) inherit: bool,
    pub(crate) required: bool,
    pub(crate) name: Option<LitStr>,
    pub(crate) default: Option<Expr>,
    pub(crate) converter: Option<crate::converter::ConverterKind>,
    pub(crate) dictionary: bool,
    pub(crate) sequence: bool,
    pub(crate) interface: Option<Path>,
    pub(crate) brand_check: Option<Path>,
    pub(crate) missing_message: Option<LitStr>,
    pub(crate) index: Option<usize>,
    pub(crate) legacy_nullish: bool,
    pub(crate) treat_null_as_empty_string: bool,
    pub(crate) allow_shared: bool,
    pub(crate) nullable: bool,
    pub(crate) with: Option<Path>,
    pub(crate) variadic: bool,
}

pub(crate) fn parse_field_attrs(field: &Field) -> Result<FieldAttrs, Error> {
    let mut parsed = FieldAttrs::default();
    for attr in field
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("webidl"))
    {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("inherit") {
                if parsed.inherit {
                    return Err(meta.error("duplicate inherit attribute"));
                }
                parsed.inherit = true;
                return Ok(());
            }
            if meta.path.is_ident("required") {
                parsed.required = true;
                return Ok(());
            }
            if meta.path.is_ident("name") {
                parsed.name = Some(meta.value()?.parse()?);
                return Ok(());
            }
            if meta.path.is_ident("default") {
                parsed.default = Some(meta.value()?.parse()?);
                return Ok(());
            }
            if meta.path.is_ident("missing_message") {
                parsed.missing_message = Some(meta.value()?.parse()?);
                return Ok(());
            }
            if meta.path.is_ident("index") {
                let index: LitInt = meta.value()?.parse()?;
                parsed.index = Some(index.base10_parse()?);
                return Ok(());
            }
            if meta.path.is_ident("legacy_nullish") {
                parsed.legacy_nullish = true;
                return Ok(());
            }
            if meta.path.is_ident("treat_null_as_empty_string") {
                parsed.treat_null_as_empty_string = true;
                return Ok(());
            }
            if meta.path.is_ident("allow_shared") {
                if parsed.allow_shared {
                    return Err(meta.error("duplicate allow_shared attribute"));
                }
                parsed.allow_shared = true;
                return Ok(());
            }
            if meta.path.is_ident("nullable") {
                parsed.nullable = true;
                return Ok(());
            }
            if meta.path.is_ident("with") {
                parsed.with = Some(meta.value()?.parse()?);
                return Ok(());
            }
            if meta.path.is_ident("variadic") {
                parsed.variadic = true;
                return Ok(());
            }
            if meta.path.is_ident("converter") {
                let converter: LitStr = meta.value()?.parse()?;
                parsed.converter = Some(crate::converter::ConverterKind::from_lit(&converter)?);
                return Ok(());
            }
            if meta.path.is_ident("interface") {
                if parsed.interface.is_some() {
                    return Err(meta.error("duplicate interface attribute"));
                }
                parsed.interface = Some(meta.value()?.parse()?);
                return Ok(());
            }
            if meta.path.is_ident("dictionary") {
                if parsed.dictionary {
                    return Err(meta.error("duplicate dictionary attribute"));
                }
                parsed.dictionary = true;
                return Ok(());
            }
            if meta.path.is_ident("sequence") {
                if parsed.sequence {
                    return Err(meta.error("duplicate sequence attribute"));
                }
                parsed.sequence = true;
                return Ok(());
            }
            if meta.path.is_ident("brand_check") {
                if parsed.brand_check.is_some() {
                    return Err(meta.error("duplicate brand_check attribute"));
                }
                parsed.brand_check = Some(meta.value()?.parse()?);
                return Ok(());
            }
            Err(meta.error("unsupported #[webidl(...)] field attribute"))
        })?;
    }
    if parsed.inherit
        && (parsed.required
            || parsed.name.is_some()
            || parsed.default.is_some()
            || parsed.converter.is_some()
            || parsed.dictionary
            || parsed.sequence
            || parsed.interface.is_some()
            || parsed.brand_check.is_some()
            || parsed.missing_message.is_some()
            || parsed.index.is_some()
            || parsed.legacy_nullish
            || parsed.treat_null_as_empty_string
            || parsed.allow_shared
            || parsed.nullable
            || parsed.with.is_some()
            || parsed.variadic)
    {
        return Err(Error::new(
            field.span(),
            "inherit cannot be combined with member or conversion attributes",
        ));
    }
    if parsed.allow_shared && (parsed.with.is_some() || parsed.treat_null_as_empty_string) {
        return Err(Error::new(
            field.span(),
            "allow_shared cannot be combined with with or string conversion options",
        ));
    }
    if let Some(brand_check) = &parsed.brand_check
        && parsed.interface.is_none()
    {
        return Err(Error::new(
            brand_check.span(),
            "brand_check requires #[webidl(interface = Type)]",
        ));
    }
    if let Some(interface) = &parsed.interface {
        if parsed.converter.is_some() || parsed.with.is_some() {
            return Err(Error::new(
                interface.span(),
                "interface cannot be combined with converter or with",
            ));
        }
        if parsed.treat_null_as_empty_string {
            return Err(Error::new(
                interface.span(),
                "treat_null_as_empty_string only applies to string converters",
            ));
        }
    }
    if parsed.dictionary {
        if parsed.sequence {
            return Err(Error::new(
                field.span(),
                "dictionary cannot be combined with sequence",
            ));
        }
        if parsed.interface.is_some() || parsed.converter.is_some() || parsed.with.is_some() {
            return Err(Error::new(
                field.span(),
                "dictionary cannot be combined with interface, converter or with",
            ));
        }
        if parsed.variadic {
            return Err(Error::new(
                field.span(),
                "dictionary cannot be combined with variadic",
            ));
        }
        if parsed.treat_null_as_empty_string {
            return Err(Error::new(
                field.span(),
                "treat_null_as_empty_string only applies to string converters",
            ));
        }
    }
    if parsed.sequence && (parsed.variadic || parsed.with.is_some()) {
        return Err(Error::new(
            field.span(),
            "sequence cannot be combined with variadic or with",
        ));
    }
    Ok(parsed)
}

#[derive(Default)]
pub(crate) struct EnumAttrs {
    pub(crate) name: Option<LitStr>,
    pub(crate) rename_all: RenameRule,
    pub(crate) parse_with: Option<Path>,
}

pub(crate) fn parse_enum_attrs(attrs: &[syn::Attribute]) -> Result<EnumAttrs, Error> {
    let mut parsed = EnumAttrs {
        rename_all: RenameRule::Lowercase,
        ..EnumAttrs::default()
    };
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("webidl")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("name") {
                parsed.name = Some(meta.value()?.parse()?);
                return Ok(());
            }
            if meta.path.is_ident("rename_all") {
                let value: LitStr = meta.value()?.parse()?;
                parsed.rename_all = parse_rename_rule(&value)?;
                return Ok(());
            }
            if meta.path.is_ident("parse_with") {
                parsed.parse_with = Some(meta.value()?.parse()?);
                return Ok(());
            }
            Err(meta.error("unsupported #[webidl(...)] enum attribute"))
        })?;
    }
    Ok(parsed)
}

#[derive(Default)]
pub(crate) struct VariantAttrs {
    pub(crate) tokens: Vec<LitStr>,
}

pub(crate) fn parse_variant_attrs(variant: &syn::Variant) -> Result<VariantAttrs, Error> {
    let mut parsed = VariantAttrs::default();
    for attr in variant
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("webidl"))
    {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("token") {
                parsed.tokens.push(meta.value()?.parse()?);
                return Ok(());
            }
            if meta.path.is_ident("alias") {
                parsed.tokens.push(meta.value()?.parse()?);
                return Ok(());
            }
            Err(meta.error("unsupported #[webidl(...)] enum variant attribute"))
        })?;
    }
    Ok(parsed)
}

pub(crate) fn apply_rename_rule(name: &str, rename_all: RenameRule) -> String {
    match rename_all {
        RenameRule::None => name.to_owned(),
        RenameRule::CamelCase => snake_or_pascal_to_camel_case(name),
        RenameRule::KebabCase => word_case(name, '-'),
        RenameRule::Lowercase => word_case(name, '\0'),
    }
}

fn snake_or_pascal_to_camel_case(name: &str) -> String {
    let words = split_words(name);
    let mut output = String::with_capacity(name.len());
    for (index, word) in words.iter().enumerate() {
        if index == 0 {
            output.push_str(&word.to_ascii_lowercase());
        } else {
            push_title_case_word(&mut output, word);
        }
    }
    output
}

fn word_case(name: &str, separator: char) -> String {
    let words = split_words(name);
    let mut output = String::with_capacity(name.len());
    for (index, word) in words.iter().enumerate() {
        if index > 0 && separator != '\0' {
            output.push(separator);
        }
        output.push_str(&word.to_ascii_lowercase());
    }
    output
}

fn push_title_case_word(output: &mut String, word: &str) {
    let mut chars = word.chars();
    if let Some(first) = chars.next() {
        output.extend(first.to_uppercase());
    }
    output.push_str(&chars.as_str().to_ascii_lowercase());
}

fn split_words(name: &str) -> Vec<String> {
    let stripped = name
        .strip_prefix("r#")
        .unwrap_or(name)
        .trim_start_matches('_');
    let mut words = Vec::new();
    let mut current = String::new();
    let mut previous_was_lower_or_digit = false;
    for character in stripped.chars() {
        if character == '_' || character == '-' {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            previous_was_lower_or_digit = false;
            continue;
        }
        if character.is_uppercase() && previous_was_lower_or_digit && !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
        previous_was_lower_or_digit = character.is_lowercase() || character.is_ascii_digit();
        current.push(character);
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

pub(crate) fn default_required_arg_message(
    prefix: &LitStr,
    ident: &Ident,
    attrs: &FieldAttrs,
) -> LitStr {
    let raw_name = attrs
        .name
        .as_ref()
        .map(LitStr::value)
        .unwrap_or_else(|| ident.to_string());
    let label = raw_name.replace('_', " ");
    let article = match label.chars().next().map(|ch| ch.to_ascii_lowercase()) {
        Some('a' | 'e' | 'i' | 'o' | 'u') => "an",
        _ => "a",
    };
    LitStr::new(
        &format!("{} requires {article} {label}", prefix.value()),
        ident.span(),
    )
}

pub(crate) fn field_member_name(
    field: &Field,
    attrs: &FieldAttrs,
    rename_all: RenameRule,
) -> Result<LitStr, Error> {
    if let Some(name) = attrs.name.clone() {
        return Ok(name);
    }
    let ident = field_ident(field)?;
    let raw_name = ident.unraw().to_string();
    let renamed = apply_rename_rule(&raw_name, rename_all);
    Ok(LitStr::new(&renamed, ident.span()))
}

pub(crate) fn field_ident(field: &Field) -> Result<Ident, Error> {
    field
        .ident
        .clone()
        .ok_or_else(|| Error::new(field.span(), "expected a named field"))
}

#[cfg(test)]
mod tests {
    use super::{RenameRule, apply_rename_rule};

    #[test]
    fn rename_rule_converts_snake_case_to_camel_case() {
        assert_eq!(
            apply_rename_rule("ignore_bom", RenameRule::CamelCase),
            "ignoreBom"
        );
        assert_eq!(
            apply_rename_rule("status_text", RenameRule::CamelCase),
            "statusText"
        );
    }

    #[test]
    fn rename_rule_converts_pascal_case_to_tokens() {
        assert_eq!(
            apply_rename_rule("SameOrigin", RenameRule::KebabCase),
            "same-origin"
        );
        assert_eq!(
            apply_rename_rule("NextUnique", RenameRule::Lowercase),
            "nextunique"
        );
    }
}
