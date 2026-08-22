//! Bridge between parser-blocking import discovery and live CSSOM owners.
//!
//! Inline `<style>` imports remain owner-bound. Linked stylesheet imports are
//! fetch-bound: the parser may have started their graph for one link, but the
//! retained result belongs to the shared physical fetch and can be installed
//! into every current or future client.

use super::*;

#[derive(Debug)]
pub(crate) struct ReadyBlockingStyleImportGraph {
    source: ReadyBlockingStyleImportGraphSource,
    roots: Vec<ConnectedStyleImportRoot>,
    graph: Arc<crate::stylesheet_blocking::StylesheetImportGraphFetchResult>,
}

#[derive(Debug)]
pub(crate) enum ReadyBlockingStyleImportGraphSource {
    Inline(Arc<ConnectedLoadOperation>),
    Linked(crate::stylesheet_blocking::StylesheetFetch),
}

impl ReadyBlockingStyleImportGraph {
    fn inline(
        operation: Arc<ConnectedLoadOperation>,
        roots: Vec<ConnectedStyleImportRoot>,
        graph: Arc<crate::stylesheet_blocking::StylesheetImportGraphFetchResult>,
    ) -> Self {
        Self {
            source: ReadyBlockingStyleImportGraphSource::Inline(operation),
            roots,
            graph,
        }
    }

    fn linked(
        fetch: crate::stylesheet_blocking::StylesheetFetch,
        roots: Vec<ConnectedStyleImportRoot>,
        graph: Arc<crate::stylesheet_blocking::StylesheetImportGraphFetchResult>,
    ) -> Self {
        Self {
            source: ReadyBlockingStyleImportGraphSource::Linked(fetch),
            roots,
            graph,
        }
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        ReadyBlockingStyleImportGraphSource,
        Vec<ConnectedStyleImportRoot>,
        Arc<crate::stylesheet_blocking::StylesheetImportGraphFetchResult>,
    ) {
        (self.source, self.roots, self.graph)
    }
}

impl DocumentRuntime {
    pub(crate) fn reconcile_connected_style_imports_with_blocking_stylesheets(&mut self) {
        self.restart_invalidated_linked_stylesheet_import_blocking_operations();
        let pending_imports = self
            .stylesheet_lifecycle
            .owner_states
            .pending_operations()
            .into_iter()
            .filter(|operation| {
                matches!(
                    &operation.parameters,
                    ConnectedLoadParameters::StyleImports { .. }
                ) && operation.blocking_operation.is_some()
            })
            .collect::<Vec<_>>();
        for operation in pending_imports {
            if self.connected_style_import_uses_blocking_stylesheet(&operation) {
                continue;
            }
            let handle = operation.owner;
            if self
                .stylesheet_lifecycle
                .owner_states
                .pending_operation(handle)
                .is_none_or(|pending| !ConnectedLoadOperation::ptr_eq(pending, &operation))
            {
                continue;
            }
            self.stylesheet_lifecycle
                .owner_states
                .clear_connected_operation(handle);
            let inline_source = match &operation.parameters {
                ConnectedLoadParameters::StyleImports { source, .. } => {
                    Some(Arc::clone(source.source()))
                }
                ConnectedLoadParameters::ImmediateOwnerProcessing
                | ConnectedLoadParameters::PreloadLikeLink { .. } => None,
            };
            self.stylesheet_lifecycle.pending_connected_loads.push_back(
                QueuedConnectedStyleLoad::new(
                    handle,
                    inline_source,
                    operation
                        .load_event_binding()
                        .map(ConnectedStyleLoadEventAdmission::LoadDelaying),
                ),
            );
        }
    }

    pub(super) fn connected_style_import_uses_blocking_stylesheet(
        &self,
        operation: &Arc<ConnectedLoadOperation>,
    ) -> bool {
        let ConnectedLoadParameters::StyleImports { urls, .. } = &operation.parameters else {
            return false;
        };
        let Some(blocking_operation) = &operation.blocking_operation else {
            return false;
        };
        debug_assert_eq!(
            blocking_operation.signature(),
            &DocumentBlockingStylesheetSignature::ParserCreatedStyleImport {
                urls: urls.to_vec(),
            }
        );
        self.stylesheet_lifecycle
            .fetches
            .status_for_blocking_operation(blocking_operation)
            .is_some()
    }

    pub(crate) fn take_ready_blocking_style_import_graphs(
        &mut self,
    ) -> Vec<ReadyBlockingStyleImportGraph> {
        let pending_inline_imports = self
            .stylesheet_lifecycle
            .owner_states
            .pending_operations()
            .into_iter()
            .filter(|operation| {
                matches!(
                    &operation.parameters,
                    ConnectedLoadParameters::StyleImports { .. }
                ) && operation.blocking_operation.is_some()
            })
            .collect::<Vec<_>>();
        let mut ready = Vec::new();
        for operation in pending_inline_imports {
            let Some(blocking_operation) = operation.blocking_operation.as_ref() else {
                continue;
            };
            let Some(status) = self
                .stylesheet_lifecycle
                .fetches
                .status_for_blocking_operation(blocking_operation)
            else {
                continue;
            };
            if status == StylesheetBlockingStatus::Pending {
                continue;
            }
            let ConnectedLoadParameters::StyleImports { roots, .. } = &operation.parameters else {
                continue;
            };
            let roots = roots.clone();
            let Some(graph) = self
                .stylesheet_lifecycle
                .fetches
                .take_completed_import_graph_for_blocking_operation(blocking_operation)
            else {
                continue;
            };
            ready.push(ReadyBlockingStyleImportGraph::inline(
                operation, roots, graph,
            ));
        }

        for (fetch, blocking_operation, _) in self.linked_stylesheet_import_blocking_operations() {
            let Some(status) = self
                .stylesheet_lifecycle
                .fetches
                .status_for_blocking_operation(&blocking_operation)
            else {
                continue;
            };
            if status == StylesheetBlockingStatus::Pending {
                continue;
            }
            let Some(graph) = self
                .stylesheet_lifecycle
                .fetches
                .take_completed_import_graph_for_blocking_operation(&blocking_operation)
            else {
                continue;
            };
            let roots = self.linked_stylesheet_import_roots(&fetch);
            ready.push(ReadyBlockingStyleImportGraph::linked(fetch, roots, graph));
        }
        ready
    }

    pub(crate) fn complete_ready_blocking_style_import_graph(
        &mut self,
        source: ReadyBlockingStyleImportGraphSource,
        owner_install_successful: bool,
        graph: Arc<crate::stylesheet_blocking::StylesheetImportGraphFetchResult>,
    ) {
        match source {
            ReadyBlockingStyleImportGraphSource::Linked(fetch) => {
                self.note_stylesheet_import_graph_completion(&fetch, graph);
            }
            ReadyBlockingStyleImportGraphSource::Inline(operation) => {
                if self
                    .stylesheet_lifecycle
                    .owner_states
                    .accept_completion(&operation, 0, true)
                {
                    self.note_connected_style_import_completion(
                        &operation,
                        owner_install_successful,
                    );
                }
            }
        }
    }
}
