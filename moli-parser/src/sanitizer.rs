use html5ever::tree_builder::SanitizerAction;
use html5ever::{Attribute, LocalName, Namespace, QualName};

/// A sanitizer name matches the local name and the namespace. Prefixes do
/// not affect whether a name is accepted.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SanitizerName {
    local: LocalName,
    namespace: Namespace,
}

impl SanitizerName {
    pub fn new(local: &str, namespace: Option<&str>) -> Self {
        Self {
            local: LocalName::from(local),
            namespace: Namespace::from(namespace.unwrap_or_default()),
        }
    }

    fn matches(&self, name: &QualName) -> bool {
        self.local == name.local && self.namespace == name.ns
    }
}

/// A normalized list supplied by an HTML sanitizer configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SanitizerFilter<T> {
    Allow(Vec<T>),
    Remove(Vec<T>),
}

impl<T> Default for SanitizerFilter<T> {
    fn default() -> Self {
        Self::Remove(Vec::new())
    }
}

/// An allowed element can add attributes to a global allowlist or restrict
/// the attributes that would otherwise be allowed on this element.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SanitizerElement {
    pub name: SanitizerName,
    pub attributes: Option<Vec<SanitizerName>>,
    pub remove_attributes: Vec<SanitizerName>,
}

/// Parser-facing configuration after the caller resolves Web IDL defaults
/// and validates the supplied members.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HtmlSanitizer {
    pub elements: SanitizerFilter<SanitizerElement>,
    pub replace_with_children: Vec<SanitizerName>,
    pub attributes: SanitizerFilter<SanitizerName>,
    pub processing_instructions: SanitizerFilter<String>,
    pub comments: bool,
    pub data_attributes: bool,
    pub javascript_urls: bool,
}

impl HtmlSanitizer {
    pub(super) fn element_action(&self, name: &QualName) -> SanitizerAction {
        if self
            .replace_with_children
            .iter()
            .any(|candidate| candidate.matches(name))
        {
            return SanitizerAction::ReplaceWithChildren;
        }
        let remove = match &self.elements {
            SanitizerFilter::Allow(elements) => !elements
                .iter()
                .any(|candidate| candidate.name.matches(name)),
            SanitizerFilter::Remove(elements) => elements
                .iter()
                .any(|candidate| candidate.name.matches(name)),
        };
        if remove {
            SanitizerAction::Remove
        } else {
            SanitizerAction::Keep
        }
    }

    pub(super) fn allows_processing_instruction(&self, target: &str) -> bool {
        match &self.processing_instructions {
            SanitizerFilter::Allow(targets) => targets.iter().any(|name| name == target),
            SanitizerFilter::Remove(targets) => !targets.iter().any(|name| name == target),
        }
    }

    pub(super) fn filter_attributes(&self, element: &QualName, attributes: &mut Vec<Attribute>) {
        let local_rule = self.local_attribute_rule(element);
        attributes.retain(|attribute| {
            self.allows_attribute_name_with_rule(local_rule, &attribute.name)
                && (self.javascript_urls || !is_script_url_attribute(element, attribute))
        });
    }

    pub(super) fn allows_attribute(&self, element: &QualName, attribute: &Attribute) -> bool {
        self.allows_attribute_name(element, &attribute.name)
            && (self.javascript_urls || !is_script_url_attribute(element, attribute))
    }

    fn allows_attribute_name(&self, element: &QualName, name: &QualName) -> bool {
        self.allows_attribute_name_with_rule(self.local_attribute_rule(element), name)
    }

    fn local_attribute_rule(&self, element: &QualName) -> Option<&SanitizerElement> {
        match &self.elements {
            SanitizerFilter::Allow(elements) => elements
                .iter()
                .find(|candidate| candidate.name.matches(element)),
            SanitizerFilter::Remove(_) => None,
        }
    }

    fn allows_attribute_name_with_rule(
        &self,
        local: Option<&SanitizerElement>,
        name: &QualName,
    ) -> bool {
        let includes =
            |names: &[SanitizerName]| names.iter().any(|candidate| candidate.matches(name));
        if local.is_some_and(|element| includes(&element.remove_attributes)) {
            return false;
        }
        let local_allow = local.and_then(|element| element.attributes.as_deref());
        match &self.attributes {
            SanitizerFilter::Allow(global) => {
                includes(global)
                    || local_allow.is_some_and(includes)
                    || (self.data_attributes
                        && name.ns.is_empty()
                        && name.local.starts_with("data-"))
            }
            SanitizerFilter::Remove(global) => {
                local_allow.is_none_or(includes) && !includes(global)
            }
        }
    }
}

// These pairs come from the HTML Standard's navigating and animating URL
// attribute lists. Parse the URL only for attributes that take a URL.
fn is_script_url_attribute(element: &QualName, attribute: &Attribute) -> bool {
    const HTML: &str = "http://www.w3.org/1999/xhtml";
    const SVG: &str = "http://www.w3.org/2000/svg";
    const MATHML: &str = "http://www.w3.org/1998/Math/MathML";
    const XLINK: &str = "http://www.w3.org/1999/xlink";
    let attr = (attribute.name.ns.as_ref(), attribute.name.local.as_ref());
    let namespace: &str = element.ns.as_ref();
    let name: &str = element.local.as_ref();
    if namespace == SVG
        && matches!(name, "animate" | "animateTransform" | "set")
        && attr == ("", "attributeName")
    {
        return matches!(attribute.value.as_ref(), "href" | "xlink:href");
    }
    let navigates = matches!(
        (namespace, name, attr),
        (HTML, "a" | "area", ("", "href"))
            | (HTML, "button" | "input", ("", "formaction"))
            | (HTML, "form", ("", "action"))
            | (SVG, "a", ("" | XLINK, "href"))
            | (MATHML, _, ("" | XLINK, "href"))
    );
    navigates
        && url::Url::parse(attribute.value.as_ref()).is_ok_and(|url| url.scheme() == "javascript")
}
