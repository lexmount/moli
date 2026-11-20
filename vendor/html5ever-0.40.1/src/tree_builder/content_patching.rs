use super::{InsertionPoint, TreeBuilder, TreeSink};
use crate::tokenizer::Tag;
use crate::QualName;
use markup5ever::interface::tree_builder::{create_element_with_flags, AppendNode};
use markup5ever::{expanded_name, local_name, ns};

/// Optional DOM operations for the HTML parser's <template for> handling.
/// The sink owns patch state for each template until the template is popped.
pub struct ContentPatching<Handle, Sink> {
    /// The current parent of a DOM node, if any.
    pub parent_node: fn(&Sink, node: &Handle) -> Option<Handle>,
    /// Find the first matching range and prepare the template's insertion
    /// target. The return value controls whether the template itself is inserted.
    pub prepare: fn(
        &Sink,
        template: &Handle,
        scope: &Handle,
        scope_is_open: bool,
        marker_name: &str,
    ) -> bool,
    /// The active patch's destination and current reference child, if any.
    pub insertion_location: fn(&Sink, template: &Handle) -> Option<(Handle, Option<Handle>)>,
    /// Remove the active patch's markers for a template closed by an end tag.
    pub finish: fn(&Sink, template: &Handle),
}

impl<Handle: Clone, Sink: TreeSink<Handle = Handle>> TreeBuilder<Handle, Sink> {
    /// Enable parser-time template patching using this sink's DOM operations.
    pub fn with_content_patching(mut self, hooks: ContentPatching<Handle, Sink>) -> Self {
        self.content_patching = Some(hooks);
        self
    }

    pub(super) fn sibling_still_under_parent(&self, sibling: &Handle, parent: &Handle) -> bool {
        self.content_patching
            .as_ref()
            .and_then(|hooks| (hooks.parent_node)(&self.sink, sibling))
            .is_some_and(|actual| self.sink.same_node(&actual, parent))
    }

    pub(super) fn template_insertion_point(&self, template: &Handle) -> InsertionPoint<Handle> {
        if let Some((parent, reference)) = self
            .content_patching
            .as_ref()
            .and_then(|hooks| (hooks.insertion_location)(&self.sink, template))
        {
            match reference {
                Some(sibling) => InsertionPoint::BeforeSibling { parent, sibling },
                None => InsertionPoint::LastChild(parent),
            }
        } else {
            InsertionPoint::LastChild(self.sink.get_template_contents(template))
        }
    }

    pub(super) fn insert_template_with_optional_patch(&self, tag: Tag) {
        let Some(hooks) = &self.content_patching else {
            self.insert_element_for(tag);
            return;
        };
        let Some(name) = tag.attrs.iter().find_map(|attribute| {
            (attribute.name.expanded() == expanded_name!("", "for") && !attribute.value.is_empty())
                .then(|| attribute.value.clone())
        }) else {
            self.insert_element_for(tag);
            return;
        };

        let location = self.appropriate_place_for_insertion(None);
        let (scope, reference_child) = match &location {
            InsertionPoint::LastChild(scope) => (scope.clone(), None),
            InsertionPoint::BeforeSibling { parent, sibling } => {
                (parent.clone(), Some(sibling.clone()))
            }
            InsertionPoint::TableFosterParenting {
                element,
                prev_element,
            } => match (hooks.parent_node)(&self.sink, element) {
                Some(parent) => (parent, Some(element.clone())),
                None => (prev_element.clone(), None),
            },
        };
        let template = create_element_with_flags(
            &self.sink,
            QualName::new(None, ns!(html), tag.name),
            tag.attrs,
            tag.had_duplicate_attributes,
        );
        if reference_child
            .as_ref()
            .is_some_and(|child| !self.sibling_still_under_parent(child, &scope))
        {
            self.push(&template);
            return;
        }
        let scope_is_open = self
            .open_elems
            .borrow()
            .iter()
            .any(|open| self.sink.same_node(open, &scope));
        self.push(&template);
        if !(hooks.prepare)(&self.sink, &template, &scope, scope_is_open, &name) {
            self.insert_at(location, AppendNode(template));
        }
    }

    pub(super) fn finish_content_patch(&self) {
        if let Some(hooks) = &self.content_patching {
            let template = self
                .open_elems
                .borrow()
                .iter()
                .rev()
                .find(|element| self.html_elem_named(element, local_name!("template")))
                .cloned();
            if let Some(template) = template {
                (hooks.finish)(&self.sink, &template);
            }
        }
    }
}
