use super::*;

/// One text node removed by normalization, optionally merged into an earlier
/// nonempty Text sibling. Consumers apply the steps in document order.
#[derive(Clone, Copy, Debug)]
pub struct DomTextNormalizationStep {
    pub parent: DomHandle,
    pub text: DomHandle,
    pub merge_into: Option<DomHandle>,
}

impl DomHost {
    /// Plan only ordinary descendants: CDATA, template contents and shadow
    /// trees are not contiguous exclusive Text nodes in this subtree.
    pub fn text_normalization_steps(&self, root: DomHandle) -> Vec<DomTextNormalizationStep> {
        let mut steps = Vec::new();
        let mut stack = self.child_handles_reversed(root).collect::<Vec<_>>();
        let mut previous_text = None;
        while let Some(handle) = stack.pop() {
            let Some(node) = self.node(handle) else {
                continue;
            };
            if node.node_type() != NodeType::Text {
                previous_text = None;
                stack.extend(self.child_handles_reversed(handle));
                continue;
            }
            let Some(parent) = node.parent_node() else {
                continue;
            };
            let merge_into = previous_text
                .filter(|(previous_parent, _)| *previous_parent == parent)
                .map(|(_, text)| text);
            if merge_into.is_some() || node.character_data_value().is_some_and(|v| v.is_empty()) {
                steps.push(DomTextNormalizationStep {
                    parent,
                    text: handle,
                    merge_into,
                });
            } else {
                previous_text = Some((parent, handle));
            }
        }
        steps
    }

    pub fn normalize_effects(&mut self, handle: DomHandle) -> DomMutationEffects {
        let mut effects = DomMutationEffects::default();
        for step in self.text_normalization_steps(handle) {
            if let Some(target) = step.merge_into {
                let source = self
                    .node(step.text)
                    .and_then(Node::character_data_value)
                    .cloned();
                let value = self
                    .node(target)
                    .and_then(Node::character_data_value)
                    .cloned();
                if let (Some(source), Some(mut value)) = (source, value)
                    && !source.is_empty()
                {
                    value.append(&source);
                    effects.merge(self.set_character_data_value_effects(target, value, false));
                }
            }
            effects.merge(self.remove_child_effects(step.parent, step.text));
        }
        effects
    }
}
