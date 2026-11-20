use super::{DomHandle, DomHost};
use crate::native::DomStringValue;

/// The DOM range selected by a parser-created template, captured before any
/// removal can invoke events or move the remaining nodes.
pub struct HtmlContentPatchPlan {
    pub target: DomHandle,
    pub start_marker: DomHandle,
    pub end_marker: Option<DomHandle>,
    pub nodes_to_remove: Vec<DomHandle>,
}

impl DomHost {
    pub fn plan_html_content_patch(
        &self,
        scope: DomHandle,
        name: &str,
    ) -> Option<HtmlContentPatchPlan> {
        if name.is_empty() {
            return None;
        }
        let name = DomStringValue::from(name);
        let attribute = DomStringValue::from("name");
        let mut descendants: Vec<_> = self.child_handles_reversed(scope).collect();
        while let Some(handle) = descendants.pop() {
            let Some(node) = self.node(handle) else {
                continue;
            };
            descendants.extend(self.child_handles_reversed(handle));
            let Some(instruction) = node.as_processing_instruction() else {
                continue;
            };
            if instruction.attributes().get(&attribute) != Some(&name) {
                continue;
            }
            let start_marker = handle;
            let target = node.parent_node()?;
            match instruction.target() {
                "marker" => {
                    return Some(HtmlContentPatchPlan {
                        target,
                        start_marker,
                        end_marker: Some(start_marker),
                        nodes_to_remove: Vec::new(),
                    });
                }
                "start" => {
                    let mut nesting = 0;
                    let mut end_marker = None;
                    let mut nodes_to_remove = Vec::new();
                    let siblings = std::iter::successors(node.next_sibling(), |&sibling| {
                        self.next_sibling(sibling)
                    });
                    for sibling in siblings {
                        match self
                            .node(sibling)
                            .and_then(|node| node.as_processing_instruction())
                            .map(|instruction| instruction.target())
                        {
                            Some("start") => nesting += 1,
                            Some("end") if nesting == 0 => {
                                end_marker = Some(sibling);
                                break;
                            }
                            Some("end") => nesting -= 1,
                            _ => {}
                        }
                        nodes_to_remove.push(sibling);
                    }
                    return Some(HtmlContentPatchPlan {
                        target,
                        start_marker,
                        end_marker,
                        nodes_to_remove,
                    });
                }
                _ => {}
            }
        }
        None
    }
}
