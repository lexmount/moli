use moli_webapi::performance::long_task::TaskAttribution;

use super::{OwnerDispatchScope, PageScriptHost};
use crate::long_task::LongTask;

impl PageScriptHost {
    pub(crate) fn report_long_task(&mut self, scope: &mut v8::PinScope<'_, '_>, task: LongTask) {
        let address = self as *mut Self as usize;
        let source = match task.first_window {
            Some(window) if window.host_address != address => return,
            Some(window) => Some(window.identity),
            None => None,
        };
        let source_scope = source.map_or(OwnerDispatchScope::Top, |id| id.dispatch_scope());
        let root = match source_scope {
            OwnerDispatchScope::Child(child) => {
                let Some(root) = self.child_browsing_context_root_scope(child) else {
                    return;
                };
                root
            }
            root => root,
        };
        let document = match root {
            OwnerDispatchScope::Top => self.document_handle(),
            OwnerDispatchScope::LightweightPopup(popup) => {
                let Some(document) = self.lightweight_popup_document_handle(popup) else {
                    return;
                };
                document
            }
            OwnerDispatchScope::Child(_) => unreachable!("a frame tree has a top-level root"),
        };
        let culprit = source
            .filter(|identity| self.window_execution_context_identity_is_current(*identity))
            .map(|identity| identity.dispatch_scope());

        let destinations = std::iter::once(root).chain(
            self.child_browsing_context_handles_in_document_order_for_document(document)
                .into_iter()
                .map(OwnerDispatchScope::Child),
        );
        for destination in destinations {
            let Some(identity) =
                self.current_registered_window_execution_context_identity(destination)
            else {
                continue;
            };
            let Some(binding) =
                self.clone_window_execution_context_binding(scope, identity.owner(), destination)
            else {
                continue;
            };
            let (name, attribution) = if task.multiple_contexts {
                ("multiple-contexts", TaskAttribution::default())
            } else if let Some(culprit) = culprit {
                self.long_task_attribution(destination, culprit)
            } else {
                ("unknown", TaskAttribution::default())
            };
            let host_ptr = self as *mut Self;
            binding.with_current_scope(scope, host_ptr, |scope, _| {
                crate::context_bootstrap::record_long_task_entry(
                    scope,
                    name,
                    task.start_unix_millis,
                    task.duration,
                    attribution,
                );
            });
        }
    }

    fn long_task_attribution(
        &self,
        destination: OwnerDispatchScope,
        culprit: OwnerDispatchScope,
    ) -> (&'static str, TaskAttribution) {
        if destination == culprit {
            return ("self", TaskAttribution::default());
        }
        let same_origin = self.window_scopes_have_same_origin(destination, culprit);
        let name = match (
            same_origin,
            self.is_long_task_ancestor(culprit, destination),
            self.is_long_task_ancestor(destination, culprit),
        ) {
            (true, true, _) => "same-origin-ancestor",
            (true, false, true) => "same-origin-descendant",
            (true, false, false) => "same-origin",
            (false, true, _) => "cross-origin-ancestor",
            (false, false, true) => "cross-origin-descendant",
            (false, false, false) => "cross-origin-unreachable",
        };
        // Container attributes belong to the embedding Document. Exposing
        // them requires local access to both that Document and the culprit.
        let attribution = match culprit {
            OwnerDispatchScope::Child(child) if same_origin => self
                .long_task_container(destination, child)
                .unwrap_or_default(),
            _ => TaskAttribution::default(),
        };
        (name, attribution)
    }

    fn is_long_task_ancestor(
        &self,
        ancestor: OwnerDispatchScope,
        mut scope: OwnerDispatchScope,
    ) -> bool {
        while let OwnerDispatchScope::Child(child) = scope {
            let Some(parent) = self.owner_dispatch_scope_for_node(child) else {
                return false;
            };
            if parent == ancestor {
                return true;
            }
            scope = parent;
        }
        false
    }

    fn long_task_container(
        &self,
        destination: OwnerDispatchScope,
        child: crate::document_runtime::DomHandle,
    ) -> Option<TaskAttribution> {
        let embedding = self.owner_dispatch_scope_for_node(child)?;
        if !self.window_scopes_have_same_origin(destination, embedding) {
            return None;
        }
        let dom = self.dom_host();
        let element = dom.node(child)?.as_element()?;
        let (container_type, url_attribute) = match element.local_name() {
            "iframe" => ("iframe", "src"),
            "frame" => ("frame", "src"),
            "object" => ("object", "data"),
            "embed" => ("embed", "src"),
            _ => return None,
        };
        Some(TaskAttribution {
            container_type,
            container_src: element
                .attribute(url_attribute)
                .unwrap_or_default()
                .to_owned(),
            container_id: element.attribute("id").unwrap_or_default().to_owned(),
            container_name: if container_type == "embed" {
                String::new()
            } else {
                element.attribute("name").unwrap_or_default().to_owned()
            },
        })
    }
}
