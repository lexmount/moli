use std::collections::HashMap;

use crate::{
    document_runtime::DomHandle,
    dom_presentation::{
        element_child_box_policy, flat_children, flat_parent, is_list_container,
        layout_element_classification, list_data,
    },
    native_bridge::{JsContextHost, element::StyleObservation},
};
use moli_layout::{
    ChildBoxPolicy, LayoutDisplay, LayoutElementCategory, LayoutFormControlKind,
    LayoutInputControlKind, ListCounter,
};
use style::{properties::ComputedValues, servo_arc::Arc, values::computed::Content};

enum Work {
    Node(DomHandle),
    GeneratedItem,
}

#[derive(Default)]
pub(super) struct ListCounters {
    values: HashMap<DomHandle, i32>,
}

impl ListCounters {
    pub(super) fn value(
        &mut self,
        host: &JsContextHost,
        observation: &mut StyleObservation<'_>,
        root: DomHandle,
        item: DomHandle,
        scripting: bool,
    ) -> i32 {
        if let Some(value) = self.values.get(&item) {
            return *value;
        }
        // A list container's own list-item box is not one of an outer list's
        // items. Its descendants belong to its separate counter scope.
        if !host
            .dom_host()
            .node(item)
            .and_then(|node| node.as_element())
            .is_some_and(is_list_container)
        {
            let mut ancestor = flat_parent(host.dom_host(), root, item);
            while let Some(node) = ancestor {
                let Some(element) = host
                    .dom_host()
                    .node(node)
                    .and_then(|node| node.as_element())
                else {
                    break;
                };
                let Some(style) = observation.read(node).computed_values() else {
                    break;
                };
                if !style.clone_display().is_contents() {
                    if is_list_container(element) {
                        self.collect_scope(host, observation, root, node, scripting);
                        break;
                    }
                    // Container traversal stops at a list item; nested items
                    // without their own HTML list use the renderer's fallback.
                    if style.clone_display().is_list_item() {
                        break;
                    }
                }
                ancestor = flat_parent(host.dom_host(), root, node);
            }
        }
        *self.values.entry(item).or_insert(1)
    }

    fn collect_scope(
        &mut self,
        host: &JsContextHost,
        observation: &mut StyleObservation<'_>,
        root: DomHandle,
        container: DomHandle,
        scripting: bool,
    ) {
        let dom = host.dom_host();
        let Some(element) = dom.node(container).and_then(|node| node.as_element()) else {
            return;
        };
        let data = list_data(element).unwrap_or_default();
        let mut items = Vec::new();
        let Some((style, before, after)) =
            observation.read(container).into_element_computed_values()
        else {
            return;
        };
        let (category, replaced) = layout_element_classification(element);
        if element_child_box_policy(
            element,
            category,
            replaced.is_some(),
            LayoutDisplay::from(&*style),
        ) != ChildBoxPolicy::All
        {
            return;
        }
        let mut pending = Vec::new();
        push_children(&mut pending, host, root, container, before, after);
        while let Some(work) = pending.pop() {
            let node = match work {
                Work::Node(node) => node,
                Work::GeneratedItem => {
                    items.push((None, None));
                    continue;
                }
            };
            let Some(element) = dom.node(node).and_then(|node| node.as_element()) else {
                continue;
            };
            let Some((style, before, after)) =
                observation.read(node).into_element_computed_values()
            else {
                continue;
            };
            let display = style.clone_display();
            let (category, replaced) = layout_element_classification(element);
            let children = element_child_box_policy(
                element,
                category,
                replaced.is_some(),
                LayoutDisplay::from(&*style),
            );
            if display.is_none()
                || (display.is_contents() && children == ChildBoxPolicy::None)
                || (scripting && element.is_html_element("noscript"))
                || category
                    == LayoutElementCategory::FormControl(LayoutFormControlKind::Input(
                        LayoutInputControlKind::Hidden,
                    ))
            {
                continue;
            }
            if !display.is_contents() {
                if is_list_container(element) {
                    continue;
                }
                if display.is_list_item() {
                    items.push((Some(node), list_data(element).and_then(|data| data.value)));
                    continue;
                }
            }
            // Column groups retain only column boxes, which are leaves, so no
            // descendant list-item boxes survive their child filtering.
            // Leaf principal boxes omit pseudo children as well as DOM children.
            // Keep the element's own list-item box counted above, if present.
            if children != ChildBoxPolicy::All {
                continue;
            }
            push_children(&mut pending, host, root, node, before, after);
        }
        let mut counter = ListCounter::new(data, items.len());
        for (node, explicit_value) in items {
            let value = counter.next_value(explicit_value);
            if let Some(node) = node {
                self.values.insert(node, value);
            }
        }
    }
}

fn push_children(
    pending: &mut Vec<Work>,
    host: &JsContextHost,
    root: DomHandle,
    node: DomHandle,
    before: Option<Arc<ComputedValues>>,
    after: Option<Arc<ComputedValues>>,
) {
    let generates_item = |style: Option<Arc<ComputedValues>>| {
        style.is_some_and(|style| {
            style.clone_display().is_list_item()
                && matches!(&style.get_counters().content, Content::Items(_))
        })
    };
    // Match the box builder's before/children/after order. Generated items
    // advance the counter without a DOM handle or an explicit li[value].
    if generates_item(after) {
        pending.push(Work::GeneratedItem);
    }
    pending.extend(
        flat_children(host.dom_host(), root, node)
            .into_iter()
            .rev()
            .map(Work::Node),
    );
    if generates_item(before) {
        pending.push(Work::GeneratedItem);
    }
}
