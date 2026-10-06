pub(crate) fn validate_attribute_name(name: &str) -> bool {
    !name.is_empty() && !name.chars().any(invalid_attribute_name_char)
}

pub(crate) fn validate_element_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if first.is_ascii_alphabetic() {
        return chars.all(|ch| !invalid_element_alpha_continuation_char(ch));
    }
    if first == ':' || first == '_' || !first.is_ascii() {
        return chars.all(valid_element_name_continuation);
    }
    false
}

#[cfg(test)]
pub(crate) fn validate_qualified_name(
    qualified_name: &str,
) -> std::result::Result<(Option<String>, String), (&'static str, i32, &'static str)> {
    let invalid_character = (
        "InvalidCharacterError",
        5,
        "String contains an invalid character",
    );

    if qualified_name.is_empty() || qualified_name.chars().any(invalid_attribute_name_char) {
        return Err(invalid_character);
    }

    let (prefix, local_name) = match qualified_name.split_once(':') {
        Some(("", _)) | Some((_, "")) => return Err(invalid_character),
        Some((prefix, local_name)) => (Some(prefix.to_owned()), local_name.to_owned()),
        None => {
            if !validate_attribute_name(qualified_name) {
                return Err(invalid_character);
            }
            return Ok((None, qualified_name.to_owned()));
        }
    };

    if !validate_namespace_prefix(prefix.as_deref().unwrap_or_default())
        || !validate_attribute_name(&local_name)
    {
        return Err(invalid_character);
    }

    Ok((prefix, local_name))
}

pub(crate) fn validate_qualified_element_name_and_namespace(
    namespace: Option<&str>,
    qualified_name: &str,
) -> std::result::Result<(Option<String>, String), (&'static str, i32, &'static str)> {
    validate_qualified_name_and_namespace_compat_with(
        namespace,
        qualified_name,
        validate_element_name,
    )
}

fn validate_namespace_prefix(prefix: &str) -> bool {
    !prefix.is_empty() && !prefix.chars().any(|ch| invalid_name_char(ch) || ch == '/')
}

fn invalid_attribute_name_char(ch: char) -> bool {
    invalid_name_char(ch) || matches!(ch, '/' | '=')
}

fn invalid_name_char(ch: char) -> bool {
    ch == '\0' || ch.is_ascii_whitespace() || ch == '>'
}

fn invalid_element_alpha_continuation_char(ch: char) -> bool {
    invalid_name_char(ch) || ch == '/'
}

fn valid_element_name_continuation(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '-' | '.' | ':' | '_') || !ch.is_ascii()
}

const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";
const XMLNS_NAMESPACE: &str = "http://www.w3.org/2000/xmlns/";

pub(crate) fn validate_qualified_name_and_namespace(
    namespace: Option<&str>,
    qualified_name: &str,
) -> std::result::Result<(Option<String>, String), (&'static str, i32, &'static str)> {
    validate_qualified_name_and_namespace_compat_with(
        namespace,
        qualified_name,
        validate_attribute_name,
    )
}

fn validate_qualified_name_and_namespace_compat_with(
    namespace: Option<&str>,
    qualified_name: &str,
    validate_local_name: fn(&str) -> bool,
) -> std::result::Result<(Option<String>, String), (&'static str, i32, &'static str)> {
    let (prefix, local_name) = if qualified_name.is_empty() {
        return Err((
            "InvalidCharacterError",
            5,
            "String contains an invalid character",
        ));
    } else {
        match qualified_name.split_once(':') {
            Some(("", _)) | Some((_, "")) => {
                return Err((
                    "InvalidCharacterError",
                    5,
                    "String contains an invalid character",
                ));
            }
            Some((prefix, local_name)) => {
                if !validate_namespace_prefix(prefix) || !validate_local_name(local_name) {
                    return Err((
                        "InvalidCharacterError",
                        5,
                        "String contains an invalid character",
                    ));
                }
                (Some(prefix.to_owned()), local_name.to_owned())
            }
            None => {
                if !validate_local_name(qualified_name) {
                    return Err((
                        "InvalidCharacterError",
                        5,
                        "String contains an invalid character",
                    ));
                }
                (None, qualified_name.to_owned())
            }
        }
    };

    validate_qualified_name_namespace_constraints(namespace, qualified_name, prefix, local_name)
}

fn validate_qualified_name_namespace_constraints(
    namespace: Option<&str>,
    qualified_name: &str,
    prefix: Option<String>,
    local_name: String,
) -> std::result::Result<(Option<String>, String), (&'static str, i32, &'static str)> {
    let namespace = namespace.filter(|namespace| !namespace.is_empty());
    if prefix.is_some() && namespace.is_none() {
        return Err((
            "NamespaceError",
            14,
            "A namespace is required when using a prefix.",
        ));
    }
    if prefix.as_deref() == Some("xml") && namespace != Some(XML_NAMESPACE) {
        return Err((
            "NamespaceError",
            14,
            "The xml prefix is only valid in the XML namespace.",
        ));
    }
    let has_xmlns_name = prefix.as_deref() == Some("xmlns") || qualified_name == "xmlns";
    if has_xmlns_name && namespace != Some(XMLNS_NAMESPACE) {
        return Err((
            "NamespaceError",
            14,
            "The xmlns prefix and qualified name require the XMLNS namespace.",
        ));
    }
    if namespace == Some(XMLNS_NAMESPACE) && !has_xmlns_name {
        return Err((
            "NamespaceError",
            14,
            "The XMLNS namespace requires the xmlns prefix or qualified name.",
        ));
    }

    Ok((prefix, local_name))
}

pub(crate) fn validate_class_list_token(
    token: &[u16],
) -> std::result::Result<(), (&'static str, i32, &'static str)> {
    if token.is_empty() {
        return Err(("SyntaxError", 12, "The token provided must not be empty."));
    }
    if token.iter().copied().any(is_html_space) {
        return Err((
            "InvalidCharacterError",
            5,
            "The token provided contains HTML space characters.",
        ));
    }
    Ok(())
}

pub(crate) fn validate_class_list_token_pair(
    first: &[u16],
    second: &[u16],
) -> std::result::Result<(), (&'static str, i32, &'static str)> {
    if first.is_empty() || second.is_empty() {
        return Err(("SyntaxError", 12, "The token provided must not be empty."));
    }
    if first.iter().copied().any(is_html_space) || second.iter().copied().any(is_html_space) {
        return Err((
            "InvalidCharacterError",
            5,
            "The token provided contains HTML space characters.",
        ));
    }
    Ok(())
}

fn is_html_space(unit: u16) -> bool {
    matches!(unit, 0x09 | 0x0a | 0x0c | 0x0d | 0x20)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_dom_attribute_names() {
        assert!(validate_attribute_name("data-id"));
        assert!(validate_attribute_name("xml:lang"));
        assert!(validate_attribute_name("俄语"));
        assert!(validate_attribute_name("_name.1"));
        assert!(validate_attribute_name("@slotchange$lit$"));
        assert!(validate_attribute_name(".ariahidden$lit$"));
        assert!(validate_attribute_name("?inert$lit$"));
        assert!(validate_attribute_name("1name"));
        assert!(validate_attribute_name("invalid^Name"));
        assert!(validate_attribute_name("\\"));
        assert!(validate_attribute_name("'"));
        assert!(validate_attribute_name("\""));
        assert!(validate_attribute_name("~"));
        assert!(validate_attribute_name("\u{1}"));
        assert!(validate_attribute_name("name<"));

        assert!(!validate_attribute_name(""));
        assert!(!validate_attribute_name("has space"));
        assert!(!validate_attribute_name("name>"));
        assert!(!validate_attribute_name("name/name"));
        assert!(!validate_attribute_name("name=name"));
        assert!(!validate_attribute_name("name\0"));
    }

    #[test]
    fn validates_dom_element_names() {
        assert!(validate_element_name("foo"));
        assert!(validate_element_name(":"));
        assert!(validate_element_name(":foo"));
        assert!(validate_element_name("_foo"));
        assert!(validate_element_name("f<oo"));
        assert!(validate_element_name("A\u{1}"));
        assert!(validate_element_name("\u{300}foo"));

        assert!(!validate_element_name(""));
        assert!(!validate_element_name("1foo"));
        assert!(!validate_element_name("-foo"));
        assert!(!validate_element_name(".foo"));
        assert!(!validate_element_name("<foo"));
        assert!(!validate_element_name("foo>"));
        assert!(!validate_element_name("fo o"));
        assert!(!validate_element_name("foo/bar"));
    }

    #[test]
    fn validates_qnames_without_namespace_constraints() {
        assert_eq!(
            validate_qualified_name("svg:circle").unwrap(),
            (Some("svg".to_owned()), "circle".to_owned())
        );
        assert_eq!(
            validate_qualified_name("俄语").unwrap(),
            (None, "俄语".to_owned())
        );
        assert_eq!(
            validate_qualified_name("f:o:o").unwrap(),
            (Some("f".to_owned()), "o:o".to_owned())
        );
        assert_eq!(
            validate_qualified_name("prefix::local").unwrap(),
            (Some("prefix".to_owned()), ":local".to_owned())
        );
        assert_eq!(
            validate_qualified_name("@slotchange$lit$").unwrap(),
            (None, "@slotchange$lit$".to_owned())
        );
        assert_eq!(
            validate_qualified_name("1name").unwrap(),
            (None, "1name".to_owned())
        );
        assert_eq!(
            validate_qualified_name("a:0").unwrap(),
            (Some("a".to_owned()), "0".to_owned())
        );
        assert_eq!(
            validate_qualified_name("f<oo").unwrap(),
            (None, "f<oo".to_owned())
        );
        assert!(validate_qualified_name("").is_err());
        assert!(validate_qualified_name(":foo").is_err());
        assert!(validate_qualified_name("a:").is_err());
        assert!(validate_qualified_name("a b").is_err());
        assert!(validate_qualified_name("foo>").is_err());
        assert!(validate_qualified_name("a=0").is_err());
    }

    #[test]
    fn validates_qualified_attribute_names_with_browser_compat_rules() {
        assert_eq!(
            validate_qualified_name_and_namespace(Some("urn:test"), "svg:circle").unwrap(),
            (Some("svg".to_owned()), "circle".to_owned())
        );
        assert_eq!(
            validate_qualified_name_and_namespace(None, "俄语").unwrap(),
            (None, "俄语".to_owned())
        );
        assert_eq!(
            validate_qualified_name_and_namespace(None, "@slotchange$lit$").unwrap(),
            (None, "@slotchange$lit$".to_owned())
        );
        assert_eq!(
            validate_qualified_name_and_namespace(None, "1bad").unwrap(),
            (None, "1bad".to_owned())
        );
        assert_eq!(
            validate_qualified_name_and_namespace(Some("urn:test"), "a:0").unwrap(),
            (Some("a".to_owned()), "0".to_owned())
        );
        assert_eq!(
            validate_qualified_name_and_namespace(Some("urn:test"), "a:b:c").unwrap(),
            (Some("a".to_owned()), "b:c".to_owned())
        );
        assert_eq!(
            validate_qualified_name_and_namespace(Some("urn:test"), "=:attr").unwrap(),
            (Some("=".to_owned()), "attr".to_owned())
        );

        assert!(validate_qualified_name_and_namespace(None, "").is_err());
        assert!(validate_qualified_name_and_namespace(None, "a:b").is_err());
        assert!(validate_qualified_name_and_namespace(Some("urn:test"), "a:").is_err());
        assert!(validate_qualified_name_and_namespace(Some("urn:test"), "a b").is_err());
        assert!(validate_qualified_name_and_namespace(Some("urn:test"), "a/b").is_err());
        assert!(validate_qualified_name_and_namespace(Some("urn:test"), "a=0").is_err());
    }

    #[test]
    fn validates_qualified_element_names_with_browser_compat_rules() {
        assert_eq!(
            validate_qualified_element_name_and_namespace(Some("urn:test"), "=:div").unwrap(),
            (Some("=".to_owned()), "div".to_owned())
        );
        assert!(validate_qualified_element_name_and_namespace(Some("urn:test"), "a:5").is_err());
    }

    #[test]
    fn keeps_dom_namespace_error_mapping_local() {
        assert_eq!(
            validate_qualified_name_and_namespace(Some("urn:test"), "xml:lang")
                .unwrap_err()
                .0,
            "NamespaceError"
        );
        assert_eq!(
            validate_qualified_name_and_namespace(Some("http://www.w3.org/2000/xmlns/"), "x:lang",)
                .unwrap_err()
                .0,
            "NamespaceError"
        );
    }
}
