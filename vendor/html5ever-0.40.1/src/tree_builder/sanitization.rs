use super::{Attribute, InsertionPoint, QualName, Tag, TreeBuilder, TreeSink};
use markup5ever::{local_name, ns};

/// The action for an element in an HTML parser sanitizer configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SanitizerAction {
    Keep,
    Remove,
    ReplaceWithChildren,
}

/// Operations the tree builder needs when the sink sanitizes elements as it
/// creates and inserts them.
pub struct Sanitization<Handle, Sink> {
    pub element_action: fn(&Sink, &Handle) -> SanitizerAction,
    pub allows_attribute: fn(&Sink, &QualName, &Attribute) -> bool,
    pub parent_node: fn(&Sink, &Handle) -> Option<Handle>,
    pub has_redirect: fn(&Sink, &Handle) -> bool,
    pub redirect: fn(&Sink, &Handle, &Handle, Option<&Handle>),
}

impl<Handle: Clone, Sink: TreeSink<Handle = Handle>> TreeBuilder<Handle, Sink> {
    pub fn set_sanitization(&mut self, hooks: Sanitization<Handle, Sink>) {
        self.sanitization = Some(hooks);
    }

    pub(super) fn sanitizer_action(&self, element: &Handle) -> SanitizerAction {
        self.sanitization
            .as_ref()
            .map_or(SanitizerAction::Keep, |hooks| {
                (hooks.element_action)(&self.sink, element)
            })
    }

    pub(super) fn sanitizer_allows_attribute(&self, element: &QualName, attr: &Attribute) -> bool {
        self.sanitization.as_ref().map_or(true, |hooks| {
            (hooks.allows_attribute)(&self.sink, element, attr)
        })
    }

    pub(super) fn filter_template_attributes(&self, tag: &mut Tag) {
        if self.sanitization.is_some() {
            let name = QualName::new(None, ns!(html), local_name!("template"));
            tag.attrs
                .retain(|attr| self.sanitizer_allows_attribute(&name, attr));
        }
    }

    pub(super) fn redirect_sanitized_elements(
        &self,
        elements: Vec<Handle>,
        location: &InsertionPoint<Handle>,
    ) {
        if elements.is_empty() {
            return;
        }
        let hooks = self.sanitization.as_ref().expect("sanitization is enabled");
        let table_parent;
        let (parent, reference) = match location {
            InsertionPoint::LastChild(parent) => (parent, None),
            InsertionPoint::BeforeSibling { parent, sibling } => (parent, Some(sibling)),
            InsertionPoint::TableFosterParenting {
                element,
                prev_element,
            } => {
                table_parent = (hooks.parent_node)(&self.sink, element);
                match table_parent.as_ref() {
                    Some(parent) => (parent, Some(element)),
                    None => (prev_element, None),
                }
            }
        };
        for element in elements {
            (hooks.redirect)(&self.sink, &element, parent, reference);
        }
    }

    pub(super) fn redirect_elements_below(&self, index: usize, parent: &Handle) {
        let Some(hooks) = &self.sanitization else {
            return;
        };
        for element in self.open_elems.borrow().iter().skip(index) {
            if !(hooks.has_redirect)(&self.sink, element) {
                break;
            }
            (hooks.redirect)(&self.sink, element, parent, None);
        }
    }

    pub(super) fn has_sanitizer_redirect(&self, node: &Handle) -> bool {
        self.sanitization
            .as_ref()
            .is_some_and(|hooks| (hooks.has_redirect)(&self.sink, node))
    }
}
