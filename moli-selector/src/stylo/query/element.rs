use std::{ptr::NonNull, vec};

use crate::CssDirection;
use app_units::Au;
use dom::ElementState;
use euclid::default::Size2D;
use selectors::{
    Element as SelectorsElement, OpaqueElement,
    attr::{AttrSelectorOperation, CaseSensitivity, NamespaceConstraint},
    bloom::BloomFilter,
    matching::{ElementSelectorFlags, MatchingContext, VisitedHandlingMode},
};
use style::{
    Atom, LocalName, Namespace,
    context::{QuirksMode, SharedStyleContext},
    data::{ElementDataMut, ElementDataRef},
    dom::{LayoutIterator, TElement, TNode},
    properties::{PropertyDeclarationBlock, longhands::display::computed_value::T as Display},
    selector_parser::{
        AttrValue as SelectorAttrValue, HorizontalDirection, Lang, NonTSPseudoClass, PseudoElement,
        SelectorImpl,
    },
    servo_arc::{Arc, ArcBorrow},
    shared_lock::{Locked, SharedRwLock},
    values::AtomIdent,
};

use crate::{
    dom::{
        NodeId,
        custom_elements::is_valid_custom_element_name,
        native::{Attribute, CustomElementState, DomHost, Element, Node},
    },
    stylo::{
        StyloElementDataStore,
        atoms::{QueryAtomCache, lang_matches},
    },
};

use super::{QueryElement, QueryNode, QueryShadowRoot};

impl<'a> QueryElement<'a> {
    pub(in crate::stylo) fn new(
        host: &'a DomHost,
        handle: NodeId,
        shared_lock: &'a SharedRwLock,
        style_data: Option<&'a StyloElementDataStore>,
        atom_cache: &'a QueryAtomCache,
    ) -> Self {
        Self {
            host,
            handle,
            shared_lock,
            style_data,
            atom_cache,
        }
    }

    pub fn handle(self) -> NodeId {
        self.handle
    }

    pub(in crate::stylo) fn host(self) -> &'a DomHost {
        self.host
    }

    pub(in crate::stylo) fn shared_lock(self) -> &'a SharedRwLock {
        self.shared_lock
    }

    pub(in crate::stylo) fn author_styles_enabled(self) -> bool {
        self.style_data
            .is_none_or(StyloElementDataStore::author_styles_enabled)
    }

    pub(crate) fn read_quirks_mode(self) -> QuirksMode {
        self.host
            .owner_document_handle(self.handle)
            .and_then(|document| self.host.node(document))
            .and_then(Node::as_document)
            .map(|document| document.quirks_mode())
            .unwrap_or(QuirksMode::NoQuirks)
    }

    pub(super) fn node(self) -> &'a Node {
        self.host
            .node(self.handle)
            .expect("query element node should exist")
    }

    pub(in crate::stylo) fn element(self) -> &'a Element {
        self.node()
            .as_element()
            .expect("query element should wrap an element node")
    }

    pub(super) fn attr_value(self, local_name: &str, namespace: Option<&str>) -> Option<&'a str> {
        self.element()
            .attributes()
            .iter()
            .find(|attribute| {
                attribute.local_name() == local_name
                    && namespace.is_none_or(|expected| attribute.namespace() == expected)
            })
            .map(Attribute::value)
    }

    pub(super) fn lang_attr_value(self) -> Option<&'a str> {
        self.attr_value("lang", Some("http://www.w3.org/XML/1998/namespace"))
            .or_else(|| self.attr_value("lang", Some("")))
            .or_else(|| self.attr_value("lang", None))
    }

    pub(super) fn matches_defined_pseudo(self) -> bool {
        let element = self.element();
        if element.namespace() != "http://www.w3.org/1999/xhtml" {
            return true;
        }
        match element.custom_element_state() {
            CustomElementState::Custom => true,
            CustomElementState::Undefined | CustomElementState::Failed => false,
            CustomElementState::Uncustomized => !is_valid_custom_element_name(element.local_name()),
        }
    }
}

impl<'a> TElement for QueryElement<'a> {
    type ConcreteNode = QueryNode<'a>;
    type TraversalChildrenIterator = vec::IntoIter<QueryNode<'a>>;

    fn get_attr(&self, attr: &LocalName, namespace: &Namespace) -> Option<String> {
        let local_name = attr.as_ref();
        let namespace = namespace.as_ref();
        self.element()
            .attributes()
            .iter()
            .find(|attribute| {
                attribute.local_name() == local_name
                    && (namespace.is_empty() || attribute.namespace() == namespace)
            })
            .map(|attribute| attribute.value().to_owned())
    }

    fn as_node(&self) -> Self::ConcreteNode {
        QueryNode {
            host: self.host,
            handle: self.handle,
            shared_lock: self.shared_lock,
            style_data: self.style_data,
            atom_cache: self.atom_cache,
        }
    }

    fn traversal_children(&self) -> LayoutIterator<Self::TraversalChildrenIterator> {
        let children = self
            .host
            .child_handles(self.handle)
            .map(|handle| QueryNode {
                host: self.host,
                handle,
                shared_lock: self.shared_lock,
                style_data: self.style_data,
                atom_cache: self.atom_cache,
            })
            .collect::<Vec<_>>();
        LayoutIterator(children.into_iter())
    }

    fn is_html_element(&self) -> bool {
        self.element().namespace() == "http://www.w3.org/1999/xhtml"
    }

    fn is_mathml_element(&self) -> bool {
        self.element().namespace() == "http://www.w3.org/1998/Math/MathML"
    }

    fn is_svg_element(&self) -> bool {
        self.element().namespace() == "http://www.w3.org/2000/svg"
    }

    fn style_attribute(&self) -> Option<ArcBorrow<'_, Locked<PropertyDeclarationBlock>>> {
        self.style_data
            .and_then(|style_data| style_data.borrow_inline_style_for_host(self.host, self.handle))
    }

    fn animation_rule(
        &self,
        _: &SharedStyleContext,
    ) -> Option<Arc<Locked<PropertyDeclarationBlock>>> {
        None
    }

    fn transition_rule(
        &self,
        _: &SharedStyleContext,
    ) -> Option<Arc<Locked<PropertyDeclarationBlock>>> {
        None
    }

    fn state(&self) -> ElementState {
        self.computed_state()
    }

    fn has_part_attr(&self) -> bool {
        self.element().has_attribute("part")
    }

    fn exports_any_part(&self) -> bool {
        self.element().has_attribute("exportparts")
    }

    fn id(&self) -> Option<&Atom> {
        self.element().id().map(|id| self.atom_cache.atom(id))
    }

    fn each_class<F>(&self, mut callback: F)
    where
        F: FnMut(&AtomIdent),
    {
        let Some(classes) = self.element().attribute("class") else {
            return;
        };
        for class in classes.split_ascii_whitespace() {
            callback(self.atom_cache.atom_ident(class));
        }
    }

    fn each_part<F>(&self, mut callback: F)
    where
        F: FnMut(&AtomIdent),
    {
        let Some(parts) = self.element().attribute("part") else {
            return;
        };
        for part in parts.split_ascii_whitespace() {
            callback(self.atom_cache.atom_ident(part));
        }
    }

    fn each_exported_part<F>(&self, name: &AtomIdent, mut callback: F)
    where
        F: FnMut(&AtomIdent),
    {
        let Some(exportparts) = self.element().attribute("exportparts") else {
            return;
        };
        for (_, outer) in
            exported_part_mappings(exportparts).filter(|(inner, _)| *inner == name.as_ref())
        {
            callback(self.atom_cache.atom_ident(outer));
        }
    }

    fn each_custom_state<F>(&self, mut callback: F)
    where
        F: FnMut(&AtomIdent),
    {
        for state in self.element().custom_states() {
            callback(self.atom_cache.atom_ident(state));
        }
    }

    fn each_attr_name<F>(&self, mut callback: F)
    where
        F: FnMut(&LocalName),
    {
        for attribute in self.element().attributes() {
            callback(self.atom_cache.local_name(attribute.local_name()));
        }
    }

    fn has_dirty_descendants(&self) -> bool {
        false
    }

    fn has_snapshot(&self) -> bool {
        false
    }

    fn handled_snapshot(&self) -> bool {
        true
    }

    unsafe fn set_handled_snapshot(&self) {}

    unsafe fn set_dirty_descendants(&self) {}

    unsafe fn unset_dirty_descendants(&self) {}

    fn store_children_to_process(&self, _n: isize) {}

    fn did_process_child(&self) -> isize {
        0
    }

    unsafe fn ensure_data(&self) -> ElementDataMut<'_> {
        self.style_data
            .expect("style traversal requires StyloDomStyleAdapter")
            .ensure_for_host(self.host, self.handle)
    }

    unsafe fn clear_data(&self) {
        if let Some(style_data) = self.style_data {
            style_data.clear_for_host(self.host, self.handle);
        }
    }

    fn has_data(&self) -> bool {
        self.style_data
            .is_some_and(|style_data| style_data.has_for_host(self.host, self.handle))
    }

    fn borrow_data(&self) -> Option<ElementDataRef<'_>> {
        self.style_data
            .and_then(|style_data| style_data.borrow_for_host(self.host, self.handle))
    }

    fn mutate_data(&self) -> Option<ElementDataMut<'_>> {
        self.style_data
            .and_then(|style_data| style_data.mutate_for_host(self.host, self.handle))
    }

    fn skip_item_display_fixup(&self) -> bool {
        false
    }

    fn may_have_animations(&self) -> bool {
        false
    }

    fn has_animations(&self, _: &SharedStyleContext) -> bool {
        false
    }

    fn has_css_animations(&self, _: &SharedStyleContext, _: Option<PseudoElement>) -> bool {
        false
    }

    fn has_css_transitions(&self, _: &SharedStyleContext, _: Option<PseudoElement>) -> bool {
        false
    }

    fn shadow_root(&self) -> Option<<Self::ConcreteNode as TNode>::ConcreteShadowRoot> {
        self.host
            .shadow_root_handle(self.handle)
            .map(|handle| QueryShadowRoot {
                host: self.host,
                handle,
                shared_lock: self.shared_lock,
                style_data: self.style_data,
                atom_cache: self.atom_cache,
            })
    }

    fn containing_shadow(&self) -> Option<<Self::ConcreteNode as TNode>::ConcreteShadowRoot> {
        self.host
            .containing_shadow_root(self.handle)
            .map(|handle| QueryShadowRoot {
                host: self.host,
                handle,
                shared_lock: self.shared_lock,
                style_data: self.style_data,
                atom_cache: self.atom_cache,
            })
    }

    fn lang_attr(&self) -> Option<SelectorAttrValue> {
        self.lang_attr_value().map(SelectorAttrValue::from)
    }

    fn match_element_lang(
        &self,
        override_lang: Option<Option<SelectorAttrValue>>,
        value: &Lang,
    ) -> bool {
        if let Some(lang) = override_lang {
            return lang
                .as_deref()
                .is_some_and(|lang| lang_matches(lang, value.as_ref()));
        }

        let mut current = Some(self.as_node());
        while let Some(node) = current {
            if let Some(element) = node.as_element()
                && let Some(lang) = element.lang_attr()
            {
                return lang_matches(lang.as_ref(), value.as_ref());
            }
            current = node.parent_node().or_else(|| {
                self.host
                    .shadow_root_host(node.handle())
                    .map(|handle| QueryNode {
                        host: self.host,
                        handle,
                        shared_lock: self.shared_lock,
                        style_data: self.style_data,
                        atom_cache: self.atom_cache,
                    })
            });
        }
        self.host
            .document_default_language_for_node(self.handle)
            .is_some_and(|lang| lang_matches(&lang, value.as_ref()))
    }

    fn is_html_document_body_element(&self) -> bool {
        self.host.document_body_handle() == Some(self.handle)
    }

    fn synthesize_presentational_hints_for_legacy_attributes<V>(
        &self,
        _: VisitedHandlingMode,
        hints: &mut V,
    ) where
        V: selectors::sink::Push<style::applicable_declarations::ApplicableDeclarationBlock>,
    {
        self.synthesize_presentational_hints(hints);
    }

    fn local_name(&self) -> &<SelectorImpl as selectors::parser::SelectorImpl>::BorrowedLocalName {
        self.atom_cache.local_name(self.element().local_name())
    }

    fn namespace(
        &self,
    ) -> &<SelectorImpl as selectors::parser::SelectorImpl>::BorrowedNamespaceUrl {
        self.atom_cache.namespace(self.element().namespace())
    }

    fn query_container_size(&self, _: &Display) -> Size2D<Option<Au>> {
        Size2D::new(None, None)
    }

    fn has_selector_flags(&self, _: ElementSelectorFlags) -> bool {
        false
    }

    fn relative_selector_search_direction(&self) -> ElementSelectorFlags {
        ElementSelectorFlags::empty()
    }
}

impl SelectorsElement for QueryElement<'_> {
    type Impl = SelectorImpl;

    fn opaque(&self) -> OpaqueElement {
        let node = self
            .host
            .node(self.handle)
            .expect("query element should exist");
        OpaqueElement::from_non_null_ptr(
            NonNull::new(node as *const Node as *mut ()).expect("node pointers are never null"),
        )
    }

    fn parent_element(&self) -> Option<Self> {
        let parent = self.node().parent_node()?;
        self.host.node(parent)?.as_element()?;
        Some(QueryElement {
            host: self.host,
            handle: parent,
            shared_lock: self.shared_lock,
            style_data: self.style_data,
            atom_cache: self.atom_cache,
        })
    }

    fn parent_node_is_shadow_root(&self) -> bool {
        self.node()
            .parent_node()
            .is_some_and(|parent| self.host.is_shadow_root(parent))
    }

    fn containing_shadow_host(&self) -> Option<Self> {
        let root = self.host.containing_shadow_root(self.handle)?;
        let host = self.host.shadow_root_host(root)?;
        Some(QueryElement {
            host: self.host,
            handle: host,
            shared_lock: self.shared_lock,
            style_data: self.style_data,
            atom_cache: self.atom_cache,
        })
    }

    fn is_pseudo_element(&self) -> bool {
        false
    }

    fn prev_sibling_element(&self) -> Option<Self> {
        self.node()
            .previous_element_sibling(self.host.dom())
            .map(|handle| QueryElement {
                host: self.host,
                handle,
                shared_lock: self.shared_lock,
                style_data: self.style_data,
                atom_cache: self.atom_cache,
            })
    }

    fn next_sibling_element(&self) -> Option<Self> {
        self.node()
            .next_element_sibling(self.host.dom())
            .map(|handle| QueryElement {
                host: self.host,
                handle,
                shared_lock: self.shared_lock,
                style_data: self.style_data,
                atom_cache: self.atom_cache,
            })
    }

    fn first_element_child(&self) -> Option<Self> {
        self.node()
            .first_element_child(self.host.dom())
            .map(|handle| QueryElement {
                host: self.host,
                handle,
                shared_lock: self.shared_lock,
                style_data: self.style_data,
                atom_cache: self.atom_cache,
            })
    }

    fn first_element_child_for_featureless_host_has(&self) -> Option<Self> {
        let shadow_root = self.host.shadow_root_handle(self.handle)?;
        self.host
            .node(shadow_root)?
            .first_element_child(self.host.dom())
            .map(|handle| QueryElement {
                host: self.host,
                handle,
                shared_lock: self.shared_lock,
                style_data: self.style_data,
                atom_cache: self.atom_cache,
            })
    }

    fn is_html_element_in_html_document(&self) -> bool {
        self.is_html_element()
            && self
                .host
                .owner_document_handle(self.handle)
                .and_then(|document| self.host.node(document))
                .and_then(Node::as_document)
                .is_some_and(|document| document.is_html_document())
    }

    fn has_local_name(
        &self,
        local_name: &<Self::Impl as selectors::parser::SelectorImpl>::BorrowedLocalName,
    ) -> bool {
        self.element().local_name() == local_name.as_ref()
    }

    fn has_namespace(
        &self,
        ns: &<Self::Impl as selectors::parser::SelectorImpl>::BorrowedNamespaceUrl,
    ) -> bool {
        self.element().namespace() == ns.as_ref()
    }

    fn is_same_type(&self, other: &Self) -> bool {
        self.element().local_name() == other.element().local_name()
            && self.element().namespace() == other.element().namespace()
    }

    fn attr_matches(
        &self,
        ns: &NamespaceConstraint<&<Self::Impl as selectors::parser::SelectorImpl>::NamespaceUrl>,
        local_name: &<Self::Impl as selectors::parser::SelectorImpl>::LocalName,
        operation: &AttrSelectorOperation<
            &<Self::Impl as selectors::parser::SelectorImpl>::AttrValue,
        >,
    ) -> bool {
        let wanted_name = local_name.as_ref();
        self.element().attributes().iter().any(|attribute| {
            if attribute.local_name() != wanted_name {
                return false;
            }
            let namespace_matches = match ns {
                NamespaceConstraint::Any => true,
                NamespaceConstraint::Specific(namespace) => {
                    attribute.namespace() == namespace.as_ref()
                }
            };
            namespace_matches && operation.eval_str(attribute.value())
        })
    }

    fn match_non_ts_pseudo_class(
        &self,
        pc: &<Self::Impl as selectors::parser::SelectorImpl>::NonTSPseudoClass,
        _context: &mut MatchingContext<Self::Impl>,
    ) -> bool {
        match pc {
            NonTSPseudoClass::Link | NonTSPseudoClass::AnyLink => self.is_link(),
            NonTSPseudoClass::Visited => false,
            NonTSPseudoClass::Hover => self.host.element_matches_hover(self.handle),
            NonTSPseudoClass::Active
            | NonTSPseudoClass::Fullscreen
            | NonTSPseudoClass::Open
            | NonTSPseudoClass::ServoNonZeroBorder
            | NonTSPseudoClass::MozMeterOptimum
            | NonTSPseudoClass::MozMeterSubOptimum
            | NonTSPseudoClass::MozMeterSubSubOptimum => false,
            NonTSPseudoClass::Autofill => self.element().autofilled(),
            NonTSPseudoClass::Modal => {
                self.element().dialog_modal()
                    && self.element().attribute("open").is_some()
                    && self.host.is_connected(self.handle)
            }
            NonTSPseudoClass::Muted => {
                self.element().is_html_media() && self.element().media_muted()
            }
            NonTSPseudoClass::Paused => {
                self.element().is_html_media() && self.element().media_paused()
            }
            NonTSPseudoClass::Playing => {
                self.element().is_html_media() && !self.element().media_paused()
            }
            NonTSPseudoClass::Seeking => {
                self.element().is_html_media() && self.element().media_seeking()
            }
            NonTSPseudoClass::Heading(levels) => levels.matches_state(self.heading_state()),
            NonTSPseudoClass::Target => self.matches_target_pseudo(),
            NonTSPseudoClass::PopoverOpen => {
                self.element().popover_open() && self.host.is_connected(self.handle)
            }
            NonTSPseudoClass::Focus | NonTSPseudoClass::FocusVisible => {
                self.host.element_matches_focus(self.handle)
            }
            NonTSPseudoClass::FocusWithin => self.host.element_matches_focus_within(self.handle),
            NonTSPseudoClass::Defined => self.matches_defined_pseudo(),
            NonTSPseudoClass::Checked => self.matches_checked_pseudo(),
            NonTSPseudoClass::Disabled => self.matches_disabled_pseudo(),
            NonTSPseudoClass::Enabled => {
                self.is_disableable_element() && !self.matches_disabled_pseudo()
            }
            NonTSPseudoClass::Required => self.matches_required_pseudo(),
            NonTSPseudoClass::Optional => self.matches_optional_pseudo(),
            NonTSPseudoClass::ReadOnly => self.matches_read_only_pseudo(),
            NonTSPseudoClass::ReadWrite => self.matches_read_write_pseudo(),
            NonTSPseudoClass::PlaceholderShown => self.matches_placeholder_shown_pseudo(),
            NonTSPseudoClass::InRange => self.matches_in_range_pseudo(),
            NonTSPseudoClass::OutOfRange => self.matches_out_of_range_pseudo(),
            NonTSPseudoClass::Valid => {
                !self.is_barred_from_constraint_validation() && !self.is_invalid()
            }
            NonTSPseudoClass::Invalid => {
                !self.is_barred_from_constraint_validation() && self.is_invalid()
            }
            NonTSPseudoClass::UserInvalid | NonTSPseudoClass::UserValid => false,
            NonTSPseudoClass::Indeterminate => self.matches_indeterminate_pseudo(),
            NonTSPseudoClass::Default => self.matches_default_pseudo(),
            NonTSPseudoClass::Lang(lang) => self.match_element_lang(None, lang),
            NonTSPseudoClass::Dir(direction) => match direction.as_horizontal_direction() {
                Some(HorizontalDirection::Ltr) => self.resolved_direction() == CssDirection::Ltr,
                Some(HorizontalDirection::Rtl) => self.resolved_direction() == CssDirection::Rtl,
                None => false,
            },
            NonTSPseudoClass::CustomState(state) => self.has_custom_state(&state.0),
        }
    }

    fn match_pseudo_element(
        &self,
        _: &<Self::Impl as selectors::parser::SelectorImpl>::PseudoElement,
        _: &mut MatchingContext<Self::Impl>,
    ) -> bool {
        false
    }

    fn apply_selector_flags(&self, _: ElementSelectorFlags) {}

    fn is_link(&self) -> bool {
        matches!(self.element().local_name(), "a" | "area")
            && self.element().attribute("href").is_some()
    }

    fn is_html_slot_element(&self) -> bool {
        self.element().is_html_element("slot")
    }

    fn has_id(
        &self,
        id: &<Self::Impl as selectors::parser::SelectorImpl>::Identifier,
        case_sensitivity: CaseSensitivity,
    ) -> bool {
        self.element()
            .id()
            .is_some_and(|actual| match case_sensitivity {
                CaseSensitivity::CaseSensitive => actual == id.as_ref(),
                CaseSensitivity::AsciiCaseInsensitive => actual.eq_ignore_ascii_case(id.as_ref()),
            })
    }

    fn has_class(
        &self,
        name: &<Self::Impl as selectors::parser::SelectorImpl>::Identifier,
        case_sensitivity: CaseSensitivity,
    ) -> bool {
        self.element().attribute("class").is_some_and(|classes| {
            classes
                .split_ascii_whitespace()
                .any(|class| match case_sensitivity {
                    CaseSensitivity::CaseSensitive => class == name.as_ref(),
                    CaseSensitivity::AsciiCaseInsensitive => {
                        class.eq_ignore_ascii_case(name.as_ref())
                    }
                })
        })
    }

    fn has_custom_state(
        &self,
        state: &<Self::Impl as selectors::parser::SelectorImpl>::Identifier,
    ) -> bool {
        self.element().has_custom_state(state.as_ref())
    }

    fn imported_part(
        &self,
        name: &<Self::Impl as selectors::parser::SelectorImpl>::Identifier,
    ) -> Option<<Self::Impl as selectors::parser::SelectorImpl>::Identifier> {
        let inner = exported_part_inner_name(self.element().attribute("exportparts")?, name)?;
        Some(self.atom_cache.atom_ident(inner).clone())
    }

    fn is_part(&self, name: &<Self::Impl as selectors::parser::SelectorImpl>::Identifier) -> bool {
        self.element().attribute("part").is_some_and(|parts| {
            parts
                .split_ascii_whitespace()
                .any(|part| part == name.as_ref())
        })
    }

    fn is_empty(&self) -> bool {
        self.host.child_handles(self.handle).all(|child| {
            let node = self.host.node(child).expect("child should exist");
            if node.is_element() {
                return false;
            }
            if node.is_text() {
                return node.node_value().unwrap_or_default().is_empty();
            }
            true
        })
    }

    fn is_root(&self) -> bool {
        self.node()
            .parent_node()
            .and_then(|parent| self.host.node(parent))
            .is_some_and(Node::is_document)
    }

    fn add_element_unique_hashes(&self, _: &mut BloomFilter) -> bool {
        false
    }
}

fn exported_part_inner_name<'a>(exportparts: &'a str, outer_name: &str) -> Option<&'a str> {
    for (inner, outer) in exported_part_mappings(exportparts) {
        if outer == outer_name {
            return Some(inner);
        }
    }
    None
}

fn exported_part_mappings(exportparts: &str) -> impl Iterator<Item = (&str, &str)> {
    exportparts.split(',').filter_map(parse_part_mapping)
}

fn is_css_ascii_whitespace(character: char) -> bool {
    matches!(character, '\t' | '\n' | '\x0C' | '\r' | ' ')
}

fn parse_part_mapping(input: &str) -> Option<(&str, &str)> {
    let input = input.trim_start_matches(is_css_ascii_whitespace);
    let first_end = input
        .char_indices()
        .find(|(_, character)| *character == ':' || is_css_ascii_whitespace(*character))
        .map(|(index, _)| index)
        .unwrap_or(input.len());
    let (first, input) = input.split_at(first_end);
    if first.is_empty() {
        return None;
    }

    let input = input.trim_start_matches(is_css_ascii_whitespace);
    if input.is_empty() {
        return Some((first, first));
    }
    let input = input.strip_prefix(':')?;
    let input = input.trim_start_matches(is_css_ascii_whitespace);
    let second_end = input
        .char_indices()
        .find(|(_, character)| *character == ':' || is_css_ascii_whitespace(*character))
        .map(|(index, _)| index)
        .unwrap_or(input.len());
    let (second, input) = input.split_at(second_end);
    if second.is_empty() {
        return None;
    }

    let input = input.trim_start_matches(is_css_ascii_whitespace);
    input.is_empty().then_some((first, second))
}

#[cfg(test)]
mod tests {
    use super::{exported_part_inner_name, exported_part_mappings, parse_part_mapping};

    #[test]
    fn exported_part_inner_name_maps_outer_names_to_inner_names() {
        assert_eq!(
            exported_part_inner_name("private: public, passthrough", "public"),
            Some("private")
        );
        assert_eq!(
            exported_part_inner_name("private: public, passthrough", "passthrough"),
            Some("passthrough")
        );
        assert_eq!(
            exported_part_inner_name("private: public, passthrough", "private"),
            None
        );
        assert_eq!(
            exported_part_inner_name("bad inner: public, private: bad outer", "public"),
            None
        );
    }

    #[test]
    fn exported_part_mappings_follow_css_shadow_parts_parsing() {
        assert_eq!(parse_part_mapping("inner"), Some(("inner", "inner")));
        assert_eq!(
            parse_part_mapping(" inner : outer "),
            Some(("inner", "outer"))
        );
        assert_eq!(parse_part_mapping("inner:outer"), Some(("inner", "outer")));
        assert_eq!(parse_part_mapping("inner outer"), None);
        assert_eq!(parse_part_mapping("inner:outer extra"), None);
        assert_eq!(parse_part_mapping("inner::outer"), None);
        assert_eq!(
            parse_part_mapping("inner:\touter"),
            Some(("inner", "outer"))
        );
        assert_eq!(
            parse_part_mapping("\ninner\t:\x0Couter\r"),
            Some(("inner", "outer"))
        );

        let mappings =
            exported_part_mappings("bad mapping, inner: outer, pass, :bad").collect::<Vec<_>>();
        assert_eq!(mappings, vec![("inner", "outer"), ("pass", "pass")]);
    }
}
