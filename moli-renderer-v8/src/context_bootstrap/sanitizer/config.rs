use std::collections::{BTreeMap, BTreeSet};

use moli_parser::{HtmlSanitizer, SanitizerElement, SanitizerFilter, SanitizerName};

pub(super) const HTML: &str = "http://www.w3.org/1999/xhtml";
pub(super) const SVG: &str = "http://www.w3.org/2000/svg";
pub(super) const MATHML: &str = "http://www.w3.org/1998/Math/MathML";

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct Name {
    // Dictionary results order null namespaces first, then compare UTF-16 units.
    pub(super) namespace: Option<Vec<u16>>,
    pub(super) local: Vec<u16>,
}

impl Name {
    pub(super) fn new(local: &str, namespace: Option<&str>) -> Self {
        Self {
            namespace: namespace.map(|value| value.encode_utf16().collect()),
            local: local.encode_utf16().collect(),
        }
    }

    pub(super) fn is_data_attribute(&self) -> bool {
        const DATA: &[u16] = &[
            b'd' as u16,
            b'a' as u16,
            b't' as u16,
            b'a' as u16,
            b'-' as u16,
        ];
        self.namespace.is_none() && self.local.starts_with(DATA)
    }

    fn parser_name(&self) -> Option<SanitizerName> {
        let local = String::from_utf16(&self.local).ok()?;
        let namespace = match &self.namespace {
            Some(units) => Some(String::from_utf16(units).ok()?),
            None => None,
        };
        Some(SanitizerName::new(&local, namespace.as_deref()))
    }
}

pub(super) type Names = BTreeSet<Name>;
pub(super) type Targets = BTreeSet<Vec<u16>>;

#[derive(Clone, Debug)]
pub(super) enum Filter<A, R = A> {
    Allow(A),
    Remove(R),
}

impl<T: Ord> Filter<BTreeSet<T>> {
    pub(super) fn allow(&mut self, item: T) -> bool {
        match self {
            Self::Allow(items) => items.insert(item),
            Self::Remove(items) => items.remove(&item),
        }
    }

    pub(super) fn remove(&mut self, item: T) -> bool {
        match self {
            Self::Allow(items) => items.remove(&item),
            Self::Remove(items) => items.insert(item),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ElementAttributes {
    pub(super) allow: Option<Names>,
    pub(super) remove: Option<Names>,
}

impl ElementAttributes {
    fn remove_from_lists(&mut self, name: &Name) -> bool {
        let removed = self.allow.as_mut().is_some_and(|list| list.remove(name));
        if let Some(list) = &mut self.remove {
            list.remove(name);
        }
        removed
    }
}

#[derive(Clone, Debug)]
pub(super) struct Configuration {
    pub(super) elements: Filter<BTreeMap<Name, ElementAttributes>, Names>,
    pub(super) attributes: Filter<Names>,
    pub(super) processing_instructions: Filter<Targets>,
    pub(super) replace_with_children: Option<Names>,
    pub(super) comments: bool,
    pub(super) data_attributes: Option<bool>,
    pub(super) javascript_urls: bool,
}

pub(super) fn can_replace_with_children(name: &Name) -> bool {
    ![("html", HTML), ("svg", SVG), ("math", MATHML)]
        .into_iter()
        .any(|(local, namespace)| name == &Name::new(local, Some(namespace)))
}

impl Configuration {
    fn remove_replacement(&mut self, name: &Name) -> bool {
        self.replace_with_children
            .as_mut()
            .is_some_and(|names| names.remove(name))
    }

    pub(super) fn allow_element(&mut self, name: Name, mut attributes: ElementAttributes) -> bool {
        if matches!(self.elements, Filter::Remove(_))
            && (attributes.allow.is_some()
                || attributes
                    .remove
                    .as_ref()
                    .is_some_and(|list| !list.is_empty()))
        {
            return false;
        }
        let replaced = self.remove_replacement(&name);
        match &mut self.elements {
            Filter::Remove(elements) => elements.remove(&name) | replaced,
            Filter::Allow(elements) => {
                match &self.attributes {
                    Filter::Allow(global) => {
                        if let Some(local) = &mut attributes.allow {
                            let data = self.data_attributes == Some(true);
                            local.retain(|name| {
                                !(global.contains(name) || data && name.is_data_attribute())
                            });
                        }
                        if let Some(local) = &mut attributes.remove {
                            local.retain(|name| global.contains(name));
                        }
                    }
                    Filter::Remove(global) => {
                        if let Some(local) = &mut attributes.allow {
                            let removed = attributes.remove.take();
                            local.retain(|name| {
                                !global.contains(name)
                                    && removed.as_ref().is_none_or(|list| !list.contains(name))
                            });
                        } else if let Some(local) = &mut attributes.remove {
                            local.retain(|name| !global.contains(name));
                        }
                    }
                }
                if elements.get(&name) == Some(&attributes) {
                    return replaced;
                }
                elements.insert(name, attributes);
                true
            }
        }
    }

    pub(super) fn remove_element(&mut self, name: Name) -> bool {
        let replaced = self.remove_replacement(&name);
        let changed = match &mut self.elements {
            Filter::Allow(elements) => elements.remove(&name).is_some(),
            Filter::Remove(elements) => elements.insert(name),
        };
        replaced || changed
    }

    pub(super) fn replace_element_with_children(&mut self, name: Name) -> bool {
        if !can_replace_with_children(&name) {
            return false;
        }
        let changed = match &mut self.elements {
            Filter::Allow(elements) => elements.remove(&name).is_some(),
            Filter::Remove(elements) => elements.remove(&name),
        };
        let inserted = self
            .replace_with_children
            .get_or_insert_default()
            .insert(name);
        inserted || changed
    }

    pub(super) fn allow_attribute(&mut self, name: Name) -> bool {
        match &mut self.attributes {
            Filter::Remove(attributes) => attributes.remove(&name),
            Filter::Allow(attributes) => {
                if self.data_attributes == Some(true) && name.is_data_attribute()
                    || attributes.contains(&name)
                {
                    return false;
                }
                if let Filter::Allow(elements) = &mut self.elements {
                    for attributes in elements.values_mut() {
                        if let Some(local) = &mut attributes.allow {
                            local.remove(&name);
                        }
                    }
                }
                attributes.insert(name)
            }
        }
    }

    pub(super) fn remove_attribute(&mut self, name: Name) -> bool {
        let mut changed = match &mut self.attributes {
            Filter::Allow(attributes) => attributes.remove(&name),
            Filter::Remove(attributes) => attributes.insert(name.clone()),
        };
        if let Filter::Allow(elements) = &mut self.elements {
            for attributes in elements.values_mut() {
                changed |= attributes.remove_from_lists(&name);
            }
        }
        changed
    }

    pub(super) fn set_data_attributes(&mut self, allow: bool) -> bool {
        let Filter::Allow(attributes) = &mut self.attributes else {
            return false;
        };
        if self.data_attributes == Some(allow) {
            return false;
        }
        if allow {
            attributes.retain(|name| !name.is_data_attribute());
            if let Filter::Allow(elements) = &mut self.elements {
                for attributes in elements.values_mut() {
                    if let Some(local) = &mut attributes.allow {
                        local.retain(|name| !name.is_data_attribute());
                    }
                }
            }
        }
        self.data_attributes = Some(allow);
        true
    }

    pub(super) fn remove_unsafe(&mut self) -> bool {
        let mut changed = std::mem::replace(&mut self.javascript_urls, false);
        for (namespace, names) in [
            (
                HTML,
                &["base", "embed", "frame", "iframe", "object", "script"][..],
            ),
            (SVG, &["script", "use"][..]),
        ] {
            for name in names {
                changed |= self.remove_element(Name::new(name, Some(namespace)));
            }
        }
        for name in crate::native_bridge::element::event_handler_content_attribute_names() {
            changed |= self.remove_attribute(Name::new(name, None));
        }
        changed
    }

    pub(super) fn to_parser_config(&self) -> HtmlSanitizer {
        fn names(list: &Names) -> Vec<SanitizerName> {
            list.iter().filter_map(Name::parser_name).collect()
        }
        fn targets(list: &Targets) -> Vec<String> {
            list.iter()
                .filter_map(|units| String::from_utf16(units).ok())
                .collect()
        }
        let elements = match &self.elements {
            Filter::Allow(elements) => SanitizerFilter::Allow(
                elements
                    .iter()
                    .filter_map(|(name, attributes)| {
                        Some(SanitizerElement {
                            name: name.parser_name()?,
                            attributes: attributes.allow.as_ref().map(names),
                            remove_attributes: attributes
                                .remove
                                .as_ref()
                                .map_or_else(Vec::new, names),
                        })
                    })
                    .collect(),
            ),
            Filter::Remove(elements) => SanitizerFilter::Remove(
                names(elements)
                    .into_iter()
                    .map(|name| SanitizerElement {
                        name,
                        attributes: None,
                        remove_attributes: Vec::new(),
                    })
                    .collect(),
            ),
        };
        HtmlSanitizer {
            elements,
            replace_with_children: self
                .replace_with_children
                .as_ref()
                .map_or_else(Vec::new, names),
            attributes: match &self.attributes {
                Filter::Allow(values) => SanitizerFilter::Allow(names(values)),
                Filter::Remove(values) => SanitizerFilter::Remove(names(values)),
            },
            processing_instructions: match &self.processing_instructions {
                Filter::Allow(values) => SanitizerFilter::Allow(targets(values)),
                Filter::Remove(values) => SanitizerFilter::Remove(targets(values)),
            },
            comments: self.comments,
            data_attributes: self.data_attributes.unwrap_or(false),
            javascript_urls: self.javascript_urls,
        }
    }
}
