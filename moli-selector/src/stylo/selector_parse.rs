use crate::cssom_selector::{
    dom_api_selector_list_contains_known_pseudo_element,
    dom_api_selector_list_has_only_known_pseudo_elements,
    dom_api_selector_text_with_trailing_attribute_recovery,
    selector_list_has_invalid_terminal_pseudo_element_chain,
    webkit_compat_pseudo_element_validation_selector,
};
use cssparser::ToCss;
use selectors::parser::ParseRelative;
use style::{
    Namespace, Prefix,
    selector_parser::{SelectorImpl, SelectorParser},
    stylesheets::{Namespaces, UrlExtraData},
};
use url::Url;

use crate::{StyleRuleNamespaceContext, dom::native::DomHost, selector::SelectorError};

type StyleRuleSelectorList = selectors::parser::SelectorList<SelectorImpl>;
pub(crate) type DomApiSelectorList = selectors::parser::SelectorList<SelectorImpl>;

pub(crate) enum ParsedDomApiSelectorList {
    EmptyKnownPseudoElement,
    Parsed(DomApiSelectorList),
}

pub(crate) fn parse_dom_api_selector_list(
    host: &DomHost,
    selector: &str,
) -> Result<ParsedDomApiSelectorList, SelectorError> {
    let url = host
        .document_url()
        .cloned()
        .unwrap_or_else(|| Url::parse("about:blank").expect("about:blank is valid"));
    parse_dom_api_selector_list_for_url(selector, url)
}

pub(crate) fn parse_dom_api_selector_list_for_url(
    selector: &str,
    url: Url,
) -> Result<ParsedDomApiSelectorList, SelectorError> {
    let selector = dom_api_selector_text_with_trailing_attribute_recovery(selector);
    if let Err(error) = crate::selector::validation::pre_validate_selector(&selector) {
        if error.message() == "unclosed '(' in selector"
            && dom_api_selector_list_has_only_known_pseudo_elements(&selector)
        {
            return Ok(ParsedDomApiSelectorList::EmptyKnownPseudoElement);
        }
        return Err(error);
    }
    if selector_list_has_invalid_terminal_pseudo_element_chain(&selector) {
        return Err(SelectorError::syntax(
            "terminal pseudo-elements cannot be chained",
        ));
    }
    if dom_api_selector_list_has_only_known_pseudo_elements(&selector)
        || (dom_api_selector_list_contains_known_pseudo_element(&selector)
            && validate_style_rule_selector_list(&selector).is_ok())
    {
        return Ok(ParsedDomApiSelectorList::EmptyKnownPseudoElement);
    }
    parse_dom_api_selector_list_text_for_url(&selector, url).map(ParsedDomApiSelectorList::Parsed)
}

fn parse_dom_api_selector_list_text_for_url(
    selector: &str,
    url: Url,
) -> Result<DomApiSelectorList, SelectorError> {
    let url_data = UrlExtraData::from(url);
    let namespaces = style::stylesheets::Namespaces::default();
    let parser = SelectorParser {
        stylesheet_origin: style::stylesheets::Origin::Author,
        namespaces: &namespaces,
        url_data: &url_data,
        for_supports_rule: false,
    };
    let mut input = cssparser::ParserInput::new(selector);
    selectors::parser::SelectorList::parse(
        &parser,
        &mut cssparser::Parser::new(&mut input),
        ParseRelative::No,
    )
    .map_err(|error| SelectorError::syntax(format!("{error:?}")))
}

pub(crate) fn validate_style_rule_selector_list(selector: &str) -> Result<(), SelectorError> {
    if selector_list_has_invalid_terminal_pseudo_element_chain(selector) {
        return Err(SelectorError::syntax(
            "terminal pseudo-elements cannot be chained",
        ));
    }
    let url = Url::parse("about:blank").expect("about:blank is valid");
    parse_style_rule_selector_list_for_url(selector, url.clone(), false)
        .or_else(|error| {
            let Some(selector) = webkit_compat_pseudo_element_validation_selector(selector) else {
                return Err(error);
            };
            parse_style_rule_selector_list_for_url(&selector, url, false)
        })
        .map(|_| ())
}

pub(crate) fn validate_supports_selector_list(selector: &str) -> Result<(), SelectorError> {
    parse_style_rule_selector_list_for_url(
        selector,
        Url::parse("about:blank").expect("about:blank is valid"),
        true,
    )
    .map(|_| ())
}

pub(crate) fn validate_supports_selector_condition_argument(
    selector: &str,
) -> Result<(), SelectorError> {
    let selector = selector.trim();
    if selector.is_empty() {
        return Err(SelectorError::syntax(
            "CSS.supports selector() requires a selector argument",
        ));
    }
    if has_top_level_selector_list_comma(selector) {
        return Err(SelectorError::syntax(
            "CSS.supports selector() accepts a single selector argument",
        ));
    }
    validate_supports_selector_list(selector)
}

pub(crate) fn validate_style_rule_selector_list_with_namespaces(
    selector: &str,
    namespace_context: &StyleRuleNamespaceContext,
) -> Result<(), SelectorError> {
    if selector_list_has_invalid_terminal_pseudo_element_chain(selector) {
        return Err(SelectorError::syntax(
            "terminal pseudo-elements cannot be chained",
        ));
    }
    let url = Url::parse("about:blank").expect("about:blank is valid");
    parse_style_rule_selector_list_for_url_and_namespaces(
        selector,
        url.clone(),
        namespace_context,
        false,
    )
    .or_else(|error| {
        let Some(selector) = webkit_compat_pseudo_element_validation_selector(selector) else {
            return Err(error);
        };
        parse_style_rule_selector_list_for_url_and_namespaces(
            &selector,
            url,
            namespace_context,
            false,
        )
    })
    .map(|_| ())
}

pub(crate) fn normalize_scope_selector_list(selector: &str) -> Result<String, SelectorError> {
    normalize_style_rule_selector_list(selector, ParseRelative::No)
}

pub(crate) fn normalize_scope_end_selector_list(selector: &str) -> Result<String, SelectorError> {
    normalize_style_rule_selector_list(selector, ParseRelative::ForScope)
}

pub(crate) fn normalize_nested_style_rule_selector_list_with_namespaces(
    selector: &str,
    namespace_context: &StyleRuleNamespaceContext,
) -> Result<String, SelectorError> {
    normalize_style_rule_selector_list_with_namespaces(
        selector,
        namespace_context,
        ParseRelative::ForNesting,
    )
}

pub(crate) fn normalize_scope_style_rule_selector_list_with_namespaces(
    selector: &str,
    namespace_context: &StyleRuleNamespaceContext,
) -> Result<String, SelectorError> {
    normalize_style_rule_selector_list_with_namespaces(
        selector,
        namespace_context,
        ParseRelative::ForScope,
    )
}

fn parse_style_rule_selector_list_for_url(
    selector: &str,
    url: Url,
    for_supports_rule: bool,
) -> Result<StyleRuleSelectorList, SelectorError> {
    parse_style_rule_selector_list_for_url_and_namespaces(
        selector,
        url,
        &StyleRuleNamespaceContext::default(),
        for_supports_rule,
    )
}

fn normalize_style_rule_selector_list(
    selector: &str,
    parse_relative: ParseRelative,
) -> Result<String, SelectorError> {
    normalize_style_rule_selector_list_with_namespaces(
        selector,
        &StyleRuleNamespaceContext::default(),
        parse_relative,
    )
}

fn normalize_style_rule_selector_list_with_namespaces(
    selector: &str,
    namespace_context: &StyleRuleNamespaceContext,
    parse_relative: ParseRelative,
) -> Result<String, SelectorError> {
    let selector_list = parse_style_rule_selector_list_for_url_and_namespaces_with_relative(
        selector,
        Url::parse("about:blank").expect("about:blank is valid"),
        namespace_context,
        false,
        parse_relative,
    )?;
    if selector_list
        .slice()
        .iter()
        .any(|selector| selector.has_pseudo_element())
    {
        return Err(SelectorError::syntax(
            "scope selectors cannot contain pseudo-elements",
        ));
    }
    let mut css_text = String::new();
    selector_list
        .to_css(&mut css_text)
        .map_err(|_| SelectorError::syntax("failed to serialize selector"))?;
    Ok(css_text)
}

fn parse_style_rule_selector_list_for_url_and_namespaces(
    selector: &str,
    url: Url,
    namespace_context: &StyleRuleNamespaceContext,
    for_supports_rule: bool,
) -> Result<StyleRuleSelectorList, SelectorError> {
    parse_style_rule_selector_list_for_url_and_namespaces_with_relative(
        selector,
        url,
        namespace_context,
        for_supports_rule,
        ParseRelative::No,
    )
}

fn parse_style_rule_selector_list_for_url_and_namespaces_with_relative(
    selector: &str,
    url: Url,
    namespace_context: &StyleRuleNamespaceContext,
    for_supports_rule: bool,
    parse_relative: ParseRelative,
) -> Result<StyleRuleSelectorList, SelectorError> {
    if selector_list_has_invalid_terminal_pseudo_element_chain(selector) {
        return Err(SelectorError::syntax(
            "terminal pseudo-elements cannot be chained",
        ));
    }
    let url_data = UrlExtraData::from(url);
    let namespaces = selector_namespaces(namespace_context);
    let parser = SelectorParser {
        stylesheet_origin: style::stylesheets::Origin::Author,
        namespaces: &namespaces,
        url_data: &url_data,
        for_supports_rule,
    };
    let mut input = cssparser::ParserInput::new(selector);
    selectors::parser::SelectorList::parse(
        &parser,
        &mut cssparser::Parser::new(&mut input),
        parse_relative,
    )
    .map_err(|error| SelectorError::syntax(format!("{error:?}")))
}

fn selector_namespaces(namespace_context: &StyleRuleNamespaceContext) -> Namespaces {
    let mut namespaces = Namespaces {
        default: namespace_context
            .default_namespace_uri
            .as_deref()
            .map(Namespace::from),
        ..Default::default()
    };
    for (prefix, namespace_uri) in &namespace_context.namespace_prefixes {
        namespaces.prefixes.insert(
            Prefix::from(prefix.as_str()),
            Namespace::from(namespace_uri.as_str()),
        );
    }
    namespaces
}

fn has_top_level_selector_list_comma(selector: &str) -> bool {
    let mut quote = None;
    let mut escaped = false;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    for ch in selector.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '\'' | '"' if quote == Some(ch) => quote = None,
            '\'' | '"' if quote.is_none() => quote = Some(ch),
            '(' if quote.is_none() => paren_depth += 1,
            ')' if quote.is_none() => paren_depth = paren_depth.saturating_sub(1),
            '[' if quote.is_none() => bracket_depth += 1,
            ']' if quote.is_none() => bracket_depth = bracket_depth.saturating_sub(1),
            ',' if quote.is_none() && paren_depth == 0 && bracket_depth == 0 => return true,
            _ => {}
        }
    }
    false
}
