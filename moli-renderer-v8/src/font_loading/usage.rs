//! Text/style resource discovery. This module never constructs layout boxes,
//! shapes text, queries geometry, paints, or starts a network operation.

use std::collections::{BTreeSet, HashMap, HashSet};

use moli_layout::{
    AnonymousInlineRunRole, ChildBoxPolicy, LayoutDisplay, LayoutElementCategory,
    LayoutFormControlKind, LayoutInputControlKind, LayoutListMarkerType, TextCharacter,
    TextTransform, TextTransformState, TextWhiteSpaceCollapse, classify_text_character,
    form_control_text, list_marker_text, select_control_texts, text_is_ignorable_in_anonymous_run,
};
use style::{
    computed_values::{list_style_position::T as MarkerPosition, position::T as Position},
    properties::ComputedValues,
    servo_arc::Arc,
    values::{
        computed::{Content, ContentItem, Float},
        specified::box_::{DisplayInside, DisplayOutside},
    },
};

use crate::{
    document_runtime::DomHandle,
    dom_presentation::{
        element_child_box_policy, flat_children, layout_element_classification, select_control_data,
    },
    native_bridge::{
        JsContextHost,
        element::{StyleObservation, text_control_value},
    },
};

use super::demand::{FontUse, FontUses};

mod lists;
use lists::ListCounters;

enum Work {
    Node(DomHandle, Option<Arc<ComputedValues>>),
    Text(Arc<ComputedValues>, String),
    Generated(Arc<ComputedValues>, bool, Option<String>),
    Restore(TextContext),
}

pub(crate) fn collect_font_uses(
    host: &JsContextHost,
    document: DomHandle,
    families: &HashSet<String>,
) -> FontUses {
    let Some(root) = host
        .dom_host()
        .dom()
        .document_element_handle_for_document(document)
    else {
        return FontUses::new();
    };
    let mut observation = StyleObservation::new(host);
    let mut counters = ListCounters::default();
    let scripting = host.document_scripting_enabled(document);
    let mut collector = TextCollector {
        families,
        queries: HashMap::new(),
        indices: HashMap::new(),
        uses: Vec::new(),
        context: TextContext::default(),
    };
    let mut pending = vec![Work::Node(root, None)];
    while let Some(work) = pending.pop() {
        let (node, inherited) = match work {
            Work::Restore(context) => {
                collector.restore(context);
                continue;
            }
            Work::Text(style, text) => {
                collector.text(&style, &text);
                continue;
            }
            Work::Generated(style, separate, default_text) => {
                if style.clone_display().is_none()
                    || (!matches!(&style.get_counters().content, Content::Items(_))
                        && default_text.is_none())
                {
                    continue;
                }
                let saved = collector.enter(&style, false, separate);
                // Columns discard text children; column groups retain only
                // column boxes. Their principal boxes still form boundaries.
                if LayoutElementCategory::Generic
                    .child_box_policy(false, LayoutDisplay::from(&*style))
                    == ChildBoxPolicy::All
                {
                    collector.generated_text(&style);
                    if let Some(text) = default_text {
                        collector.text(&style, &text);
                    }
                }
                if let Some(context) = saved {
                    collector.restore(context);
                }
                continue;
            }
            Work::Node(node, inherited) => (node, inherited),
        };
        let Some(native) = host.dom_host().node(node) else {
            continue;
        };
        if let Some(text) = native
            .data_value()
            .filter(|_| native.is_text() || native.is_cdata_section())
        {
            if let Some(style) = inherited {
                collector.text(&style, text);
            }
            continue;
        }
        let Some(element) = native.as_element() else {
            continue;
        };
        let read = observation.read(node);
        let Some(style) = read.computed_values() else {
            continue;
        };
        let display = LayoutDisplay::from(&*style);
        let (category, replaced) = layout_element_classification(element);
        let children = element_child_box_policy(element, category, replaced.is_some(), display);
        if display == LayoutDisplay::None
            || (display == LayoutDisplay::Contents && children == ChildBoxPolicy::None)
            || (scripting && host.dom_host().is_html_element_named(node, "noscript"))
        {
            continue;
        }
        if category == LayoutElementCategory::LineBreak {
            collector.break_text();
            continue;
        }
        let control = match category {
            LayoutElementCategory::FormControl(LayoutFormControlKind::Input(
                LayoutInputControlKind::Hidden,
            )) => continue,
            LayoutElementCategory::FormControl(
                kind @ (LayoutFormControlKind::Input(_) | LayoutFormControlKind::TextArea),
            ) => Some(kind),
            _ => None,
        };
        if let Some(context) = collector.enter(&style, replaced.is_some(), false) {
            pending.push(Work::Restore(context));
        }
        // Hidden contents still leave a principal box and its text boundary.
        // Inline/contents elements for which the property is inapplicable
        // keep their visible text, using the same facts as rendered text APIs.
        if read
            .rendered_style_facts()
            .is_some_and(|facts| facts.hides_contents)
        {
            continue;
        }
        if category == LayoutElementCategory::FormControl(LayoutFormControlKind::Select) {
            let data = select_control_data(host.dom_host(), node, element);
            for text in select_control_texts(&data) {
                // Each rendered listbox row has its own inline text context.
                collector.context = TextContext::default();
                collector.text(&style, &text);
            }
            // Option children supply labels, not independent rendered text.
            continue;
        }
        if let Some(kind) = control {
            let (text, is_placeholder) = form_control_text(
                kind,
                &text_control_value(host, node),
                element.attribute("placeholder").unwrap_or(""),
            );
            let text_style = if is_placeholder {
                read.pseudo_computed_values("placeholder").unwrap_or(style)
            } else {
                style
            };
            pending.push(Work::Text(text_style, text));
            // A textarea's children are its default value, not its live value.
            continue;
        }
        // Collect control labels above, but omit pseudo/DOM fallback streams
        // that the box builder never renders. Column groups retain only leaf
        // column boxes, so none of their descendants contribute text either.
        if children != ChildBoxPolicy::All {
            continue;
        }
        let marker = style
            .clone_display()
            .is_list_item()
            .then(|| read.pseudo_computed_values("marker"))
            .flatten();
        let marker_separate = style.clone_list_style_position() == MarkerPosition::Outside;
        // Outside markers have their own text context, so an unrelated font
        // cannot affect either this discovery or the item's inline state.
        let marker = marker.filter(|marker| !marker_separate || collector.query(marker).is_some());
        let marker_text = marker.as_ref().and_then(|marker| {
            (matches!(&marker.get_counters().content, Content::Normal)
                && !marker.clone_display().is_none()
                && !marker.clone_display().is_contents())
            .then(|| {
                list_marker_text(
                    &LayoutListMarkerType::from(&*style),
                    counters.value(host, &mut observation, root, node, scripting),
                )
            })
            .flatten()
        });
        let (_, before, after) = read.into_element_computed_values().expect("observed style");
        if let Some(after) = after {
            pending.push(Work::Generated(after, false, None));
        }
        // Follow DOM slot assignment rather than an intermediate layout tree.
        let children = flat_children(host.dom_host(), root, node);
        pending.extend(
            children
                .into_iter()
                .rev()
                .map(|child| Work::Node(child, Some(style.clone()))),
        );
        if let Some(before) = before {
            pending.push(Work::Generated(before, false, None));
        }
        if let Some(marker) = marker {
            pending.push(Work::Generated(marker, marker_separate, marker_text));
        }
    }
    collector
        .uses
        .into_iter()
        .filter(|(_, characters)| !characters.is_empty())
        .collect()
}

#[derive(Default)]
struct TextContext {
    transform: TextTransformState,
    // Outer None means no pending whitespace; inner None is an irrelevant font.
    pending_space: Option<Option<usize>>,
    line_has_content: bool,
    carriage_return: bool,
    anonymous_role: AnonymousInlineRunRole,
    anonymous_retained: bool,
    // Only the undecided prefix is buffered. A later non-whitespace text box
    // keeps these characters and their original styles in the same item.
    anonymous_pending: Vec<(Arc<ComputedValues>, String)>,
}

struct TextCollector<'a> {
    families: &'a HashSet<String>,
    queries: HashMap<usize, (Arc<ComputedValues>, Option<usize>)>,
    indices: HashMap<FontUse, usize>,
    uses: Vec<(FontUse, BTreeSet<char>)>,
    context: TextContext,
}

impl TextCollector<'_> {
    fn enter(
        &mut self,
        style: &ComputedValues,
        replaced: bool,
        separate: bool,
    ) -> Option<TextContext> {
        let anonymous_role = if replaced {
            AnonymousInlineRunRole::MixedFlow
        } else {
            match LayoutDisplay::from(style) {
                LayoutDisplay::Flex | LayoutDisplay::InlineFlex => AnonymousInlineRunRole::FlexItem,
                LayoutDisplay::Grid | LayoutDisplay::InlineGrid => AnonymousInlineRunRole::GridItem,
                _ => AnonymousInlineRunRole::MixedFlow,
            }
        };
        if separate {
            self.finish_anonymous_run();
            let saved = std::mem::take(&mut self.context);
            self.context.anonymous_role = anonymous_role;
            return Some(saved);
        }
        let display = style.clone_display();
        let out_of_flow = matches!(style.clone_position(), Position::Absolute | Position::Fixed);
        let floated = style.clone_float() != Float::None;
        // Contents never introduces a principal box, including when position
        // or float is specified. Its text stays in the parent's direct run.
        if display.is_contents()
            || (!replaced
                && !out_of_flow
                && !floated
                && display.outside() == DisplayOutside::Inline
                && display.inside() == DisplayInside::Flow
                && self.context.anonymous_role == AnonymousInlineRunRole::MixedFlow)
        {
            return None;
        }
        // A principal box terminates a direct-text item, including an empty,
        // replaced or out-of-flow box. Contents nodes stay transparent above.
        self.finish_anonymous_run();
        // The CR has already contributed a normalized newline. An object
        // ends CRLF pairing before saving the surrounding text context.
        self.context.carriage_return = false;
        if !out_of_flow && self.context.anonymous_role == AnonymousInlineRunRole::MixedFlow {
            if floated || display.outside() == DisplayOutside::Inline {
                self.flush_space();
                self.context.line_has_content = true;
            } else {
                self.break_text();
            }
        }
        let saved = std::mem::take(&mut self.context);
        self.context.anonymous_role = anonymous_role;
        Some(saved)
    }

    fn restore(&mut self, context: TextContext) {
        self.finish_anonymous_run();
        self.context = context;
    }

    fn break_text(&mut self) {
        self.context = TextContext {
            anonymous_role: self.context.anonymous_role,
            ..TextContext::default()
        };
    }

    fn finish_anonymous_run(&mut self) {
        if self.context.anonymous_role != AnonymousInlineRunRole::MixedFlow {
            // An all-whitespace prefix produces no anonymous flex/grid item.
            // Retained items have already been collected; the next one has an
            // independent inline text context.
            self.break_text();
        }
    }

    fn retain_anonymous_run(&mut self) {
        self.context.anonymous_retained = true;
        for (style, text) in std::mem::take(&mut self.context.anonymous_pending) {
            self.normalized_text(&style, &text);
        }
    }

    fn query(&mut self, style: &Arc<ComputedValues>) -> Option<usize> {
        let identity = std::ptr::from_ref(style.get_font()) as usize;
        if let Some((_, query)) = self.queries.get(&identity) {
            return *query;
        }
        let usage = FontUse::from_style(style);
        let relevant = usage
            .families
            .iter()
            .any(|name| self.families.contains(name));
        let query = relevant.then(|| {
            if let Some(index) = self.indices.get(&usage) {
                *index
            } else {
                let index = self.uses.len();
                self.indices.insert(usage.clone(), index);
                self.uses.push((usage, BTreeSet::new()));
                index
            }
        });
        // Retaining the style keeps its font-struct identity from being recycled.
        self.queries.insert(identity, (style.clone(), query));
        query
    }

    fn flush_space(&mut self) {
        if let Some(Some(index)) = self.context.pending_space.take()
            && self.context.line_has_content
        {
            self.uses[index].1.insert(' ');
        }
    }

    fn text(&mut self, style: &Arc<ComputedValues>, text: &str) {
        if text.is_empty() {
            return;
        }
        if self.context.anonymous_role != AnonymousInlineRunRole::MixedFlow
            && !self.context.anonymous_retained
        {
            if text_is_ignorable_in_anonymous_run(
                text,
                TextWhiteSpaceCollapse::from(style.clone_white_space_collapse()),
                self.context.anonymous_role,
            ) {
                self.context
                    .anonymous_pending
                    .push((style.clone(), text.to_owned()));
                return;
            }
            self.retain_anonymous_run();
        }
        self.normalized_text(style, text);
    }

    fn normalized_text(&mut self, style: &Arc<ComputedValues>, text: &str) {
        let query = self.query(style);
        let transform = TextTransform::from(style.clone_text_transform().case());
        let whitespace = TextWhiteSpaceCollapse::from(style.clone_white_space_collapse());
        // Even irrelevant fonts must advance the common inline word/space state.
        for source in text.chars() {
            if std::mem::take(&mut self.context.carriage_return) && source == '\n' {
                continue;
            }
            let source = if source == '\r' {
                self.context.carriage_return = true;
                '\n'
            } else {
                source
            };
            for character in self.context.transform.transform(source, transform) {
                match classify_text_character(character, whitespace) {
                    TextCharacter::Collapsible { .. } => {
                        self.context.pending_space.get_or_insert(query);
                    }
                    TextCharacter::ForcedBreak { preserve_pending } => {
                        if preserve_pending {
                            self.flush_space();
                        }
                        self.context.pending_space = None;
                        self.context.line_has_content = false;
                        self.context.transform.reset();
                    }
                    TextCharacter::Preserved { character, .. } => {
                        self.flush_space();
                        if let Some(index) = query
                            && !character.is_control()
                        {
                            self.uses[index].1.insert(character);
                        }
                        self.context.line_has_content = true;
                    }
                }
            }
        }
    }

    fn generated_text(&mut self, style: &Arc<ComputedValues>) {
        if let Content::Items(items) = &style.get_counters().content {
            let items = &items.items[..items.alt_start];
            if self.context.anonymous_role != AnonymousInlineRunRole::MixedFlow
                && items
                    .iter()
                    .any(|item| !matches!(item, ContentItem::String(_)))
            {
                self.retain_anonymous_run();
            }
            for item in items {
                if let ContentItem::String(text) = item {
                    self.text(style, text);
                }
            }
        }
    }
}
