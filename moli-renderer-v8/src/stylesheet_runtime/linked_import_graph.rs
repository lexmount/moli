use super::*;

#[derive(Debug)]
pub(crate) struct LinkedStylesheetImportGraphCompletion {
    pub(in crate::document_runtime) fetch: crate::stylesheet_blocking::StylesheetFetch,
    pub(in crate::document_runtime) graph:
        Arc<crate::stylesheet_blocking::StylesheetImportGraphFetchResult>,
    pub(in crate::document_runtime) network_results: Vec<ConnectedLoadNetworkResult>,
}

/// Per-Document lifecycle for the import graph shared by one exact stylesheet
/// fetch.
///
/// The physical stylesheet response is shared, but every linked owner keeps a
/// distinct CSSOM root. Keeping the phase and roots in one entry prevents a
/// one-shot network decision from accidentally suppressing per-owner install
/// work.
#[derive(Debug, Default)]
pub(in crate::document_runtime) struct LinkedStylesheetImportGraphs {
    entries:
        HashMap<crate::stylesheet_blocking::StylesheetFetchIdentity, LinkedStylesheetImportGraph>,
}

#[derive(Debug)]
struct LinkedStylesheetImportGraph {
    /// Retains the allocation backing the pointer-derived map identity.
    fetch: crate::stylesheet_blocking::StylesheetFetch,
    roots: Vec<ConnectedStyleImportRoot>,
    phase: LinkedStylesheetImportGraphPhase,
}

#[derive(Debug)]
enum LinkedStylesheetImportGraphPhase {
    NotStarted,
    InFlight {
        blocking: Option<LinkedStylesheetBlockingImportGraph>,
    },
    Completed(Arc<crate::stylesheet_blocking::StylesheetImportGraphFetchResult>),
}

#[derive(Debug)]
struct LinkedStylesheetBlockingImportGraph {
    operation: crate::stylesheet_blocking::StylesheetBlockingOperation,
    urls: Vec<Url>,
}

pub(in crate::document_runtime) enum LinkedStylesheetImportGraphAdmission {
    /// This admission owns the only dependent-resource start for the fetch.
    Start,
    /// Another owner already started the shared graph.
    InFlight,
    /// The graph is terminal and can be installed into the new owner now.
    Completed(Arc<crate::stylesheet_blocking::StylesheetImportGraphFetchResult>),
}

impl LinkedStylesheetImportGraphs {
    pub(super) fn admit(
        &mut self,
        fetch: &crate::stylesheet_blocking::StylesheetFetch,
        root: Option<ConnectedStyleImportRoot>,
    ) -> LinkedStylesheetImportGraphAdmission {
        let entry =
            self.entries
                .entry(fetch.identity())
                .or_insert_with(|| LinkedStylesheetImportGraph {
                    fetch: fetch.clone(),
                    roots: Vec::new(),
                    phase: LinkedStylesheetImportGraphPhase::NotStarted,
                });
        debug_assert!(entry.fetch.ptr_eq(fetch));
        if let Some(root) = root {
            entry
                .roots
                .retain(|candidate| candidate.owner != root.owner);
            entry.roots.push(root);
        }
        match &entry.phase {
            LinkedStylesheetImportGraphPhase::NotStarted => {
                entry.phase = LinkedStylesheetImportGraphPhase::InFlight { blocking: None };
                LinkedStylesheetImportGraphAdmission::Start
            }
            LinkedStylesheetImportGraphPhase::InFlight { .. } => {
                LinkedStylesheetImportGraphAdmission::InFlight
            }
            LinkedStylesheetImportGraphPhase::Completed(graph) => {
                LinkedStylesheetImportGraphAdmission::Completed(Arc::clone(graph))
            }
        }
    }

    pub(super) fn complete(
        &mut self,
        fetch: &crate::stylesheet_blocking::StylesheetFetch,
        graph: Arc<crate::stylesheet_blocking::StylesheetImportGraphFetchResult>,
    ) -> Arc<crate::stylesheet_blocking::StylesheetImportGraphFetchResult> {
        let entry =
            self.entries
                .entry(fetch.identity())
                .or_insert_with(|| LinkedStylesheetImportGraph {
                    fetch: fetch.clone(),
                    roots: Vec::new(),
                    phase: LinkedStylesheetImportGraphPhase::InFlight { blocking: None },
                });
        debug_assert!(entry.fetch.ptr_eq(fetch));
        if !matches!(entry.phase, LinkedStylesheetImportGraphPhase::Completed(_)) {
            entry.phase = LinkedStylesheetImportGraphPhase::Completed(graph);
        }
        match &entry.phase {
            LinkedStylesheetImportGraphPhase::Completed(graph) => Arc::clone(graph),
            LinkedStylesheetImportGraphPhase::NotStarted
            | LinkedStylesheetImportGraphPhase::InFlight { .. } => {
                unreachable!("completing an import graph must retain its terminal")
            }
        }
    }

    pub(super) fn completion_state(
        &self,
        fetch: &crate::stylesheet_blocking::StylesheetFetch,
    ) -> StylesheetCompletionState {
        let Some(entry) = self.entry(fetch) else {
            return StylesheetCompletionState::Pending;
        };
        match &entry.phase {
            LinkedStylesheetImportGraphPhase::Completed(graph) => {
                StylesheetCompletionState::from_successful(graph.successful())
            }
            LinkedStylesheetImportGraphPhase::NotStarted
            | LinkedStylesheetImportGraphPhase::InFlight { .. } => {
                StylesheetCompletionState::Pending
            }
        }
    }

    pub(super) fn bind_blocking_operation(
        &mut self,
        fetch: &crate::stylesheet_blocking::StylesheetFetch,
        operation: crate::stylesheet_blocking::StylesheetBlockingOperation,
        urls: Vec<Url>,
    ) {
        let entry = self
            .entries
            .get_mut(&fetch.identity())
            .filter(|entry| entry.fetch.ptr_eq(fetch))
            .expect("a linked import graph must be admitted before its work is started");
        let LinkedStylesheetImportGraphPhase::InFlight { blocking } = &mut entry.phase else {
            unreachable!("only an in-flight linked import graph can bind blocking work")
        };
        debug_assert!(
            blocking
                .as_ref()
                .is_none_or(|current| current.operation.ptr_eq(&operation))
        );
        *blocking = Some(LinkedStylesheetBlockingImportGraph { operation, urls });
    }

    pub(super) fn blocking_operations(
        &self,
    ) -> Vec<(
        crate::stylesheet_blocking::StylesheetFetch,
        crate::stylesheet_blocking::StylesheetBlockingOperation,
        Vec<Url>,
    )> {
        self.entries
            .values()
            .filter_map(|entry| {
                let LinkedStylesheetImportGraphPhase::InFlight {
                    blocking: Some(blocking),
                } = &entry.phase
                else {
                    return None;
                };
                Some((
                    entry.fetch.clone(),
                    blocking.operation.clone(),
                    blocking.urls.clone(),
                ))
            })
            .collect()
    }

    pub(super) fn replace_blocking_with_async(
        &mut self,
        fetch: &crate::stylesheet_blocking::StylesheetFetch,
        operation: &crate::stylesheet_blocking::StylesheetBlockingOperation,
    ) -> bool {
        let Some(entry) = self
            .entries
            .get_mut(&fetch.identity())
            .filter(|entry| entry.fetch.ptr_eq(fetch))
        else {
            return false;
        };
        let LinkedStylesheetImportGraphPhase::InFlight { blocking } = &mut entry.phase else {
            return false;
        };
        if blocking
            .as_ref()
            .is_none_or(|current| !current.operation.ptr_eq(operation))
        {
            return false;
        }
        *blocking = None;
        true
    }

    pub(super) fn roots(
        &self,
        fetch: &crate::stylesheet_blocking::StylesheetFetch,
    ) -> Vec<ConnectedStyleImportRoot> {
        self.entry(fetch)
            .map(|entry| entry.roots.clone())
            .unwrap_or_default()
    }

    pub(super) fn unregister_owner(
        &mut self,
        fetch: &crate::stylesheet_blocking::StylesheetFetch,
        owner: DomHandle,
    ) {
        let Some(entry) = self.entries.get_mut(&fetch.identity()) else {
            return;
        };
        debug_assert!(entry.fetch.ptr_eq(fetch));
        entry.roots.retain(|root| root.owner != owner);
    }

    fn entry(
        &self,
        fetch: &crate::stylesheet_blocking::StylesheetFetch,
    ) -> Option<&LinkedStylesheetImportGraph> {
        self.entries
            .get(&fetch.identity())
            .filter(|entry| entry.fetch.ptr_eq(fetch))
    }
}

impl DocumentRuntime {
    pub(super) fn admit_linked_stylesheet_import_graph(
        &mut self,
        fetch: &crate::stylesheet_blocking::StylesheetFetch,
        root: Option<ConnectedStyleImportRoot>,
    ) -> LinkedStylesheetImportGraphAdmission {
        self.stylesheet_lifecycle
            .linked_stylesheet_import_graphs
            .admit(fetch, root)
    }

    pub(super) fn linked_stylesheet_import_roots(
        &self,
        fetch: &crate::stylesheet_blocking::StylesheetFetch,
    ) -> Vec<ConnectedStyleImportRoot> {
        self.stylesheet_lifecycle
            .linked_stylesheet_import_graphs
            .roots(fetch)
    }

    pub(super) fn bind_linked_stylesheet_import_blocking_operation(
        &mut self,
        fetch: &crate::stylesheet_blocking::StylesheetFetch,
        operation: crate::stylesheet_blocking::StylesheetBlockingOperation,
        urls: Vec<Url>,
    ) {
        self.stylesheet_lifecycle
            .linked_stylesheet_import_graphs
            .bind_blocking_operation(fetch, operation, urls);
    }

    pub(super) fn linked_stylesheet_import_blocking_operations(
        &self,
    ) -> Vec<(
        crate::stylesheet_blocking::StylesheetFetch,
        crate::stylesheet_blocking::StylesheetBlockingOperation,
        Vec<Url>,
    )> {
        self.stylesheet_lifecycle
            .linked_stylesheet_import_graphs
            .blocking_operations()
    }

    pub(super) fn restart_invalidated_linked_stylesheet_import_blocking_operations(&mut self) {
        let operations = self.linked_stylesheet_import_blocking_operations();
        for (fetch, operation, urls) in operations {
            if self
                .stylesheet_lifecycle
                .fetches
                .status_for_blocking_operation(&operation)
                .is_some()
            {
                continue;
            }
            if !self
                .stylesheet_lifecycle
                .linked_stylesheet_import_graphs
                .replace_blocking_with_async(&fetch, &operation)
            {
                continue;
            }
            self.spawn_linked_stylesheet_import_graph(fetch, urls);
        }
    }

    pub(super) fn unregister_linked_stylesheet_import_consumer(
        &mut self,
        load: &Arc<StylesheetLinkClient>,
    ) {
        self.stylesheet_lifecycle
            .linked_stylesheet_import_graphs
            .unregister_owner(load.fetch(), load.owner());
    }

    pub(super) fn initial_stylesheet_import_completion(
        &self,
        stylesheet_url: &Url,
        fetch: &crate::stylesheet_blocking::StylesheetFetch,
    ) -> StylesheetCompletionState {
        let graph_completion = self
            .stylesheet_lifecycle
            .linked_stylesheet_import_graphs
            .completion_state(fetch);
        if graph_completion != StylesheetCompletionState::Pending {
            return graph_completion;
        }
        if stylesheet_url.scheme() != "data" {
            return StylesheetCompletionState::Pending;
        }
        match super::import_graph::data_stylesheet_import_readiness(stylesheet_url) {
            super::import_graph::DataStylesheetImportReadiness::NoImports => {
                StylesheetCompletionState::Succeeded
            }
            super::import_graph::DataStylesheetImportReadiness::Imports(_) => {
                StylesheetCompletionState::Pending
            }
            super::import_graph::DataStylesheetImportReadiness::Failed => {
                StylesheetCompletionState::Failed
            }
        }
    }

    pub(crate) fn prime_network_stylesheet_import_loads(
        &mut self,
        load: Arc<StylesheetLinkClient>,
        urls: Vec<Url>,
        host_ptr: *mut JsContextHost,
    ) {
        let fetch = load.fetch().clone();
        let roots = self.linked_stylesheet_import_roots(&fetch);
        let urls = match super::import_graph::connected_style_import_readiness(urls.clone()) {
            super::import_graph::ConnectedStyleImportReadiness::Ready(graph_successful) => {
                if !host_ptr.is_null() {
                    for root in &roots {
                        if unsafe { &*host_ptr }
                            .install_live_stylesheet_import_graph(root.clone(), &[])
                            .is_some()
                        {
                            let _ = unsafe { &mut *host_ptr }
                                .refresh_live_stylesheet_after_import_graph(
                                    root.owner,
                                    root.stylesheet_id,
                                );
                        }
                    }
                }
                self.note_stylesheet_import_graph_completion(
                    &fetch,
                    Arc::new(
                        crate::stylesheet_blocking::StylesheetImportGraphFetchResult::new(
                            graph_successful,
                            Vec::new(),
                        ),
                    ),
                );
                return;
            }
            super::import_graph::ConnectedStyleImportReadiness::Pending(urls) => urls,
        };
        let blocking_signature =
            DocumentBlockingStylesheetSignature::ParserCreatedStyleImport { urls: urls.clone() };
        let blocking_operation = self
            .stylesheet_lifecycle
            .fetches
            .blocking_operation(NodeId::new(load.owner().index()), &blocking_signature);
        if let Some(blocking_operation) = blocking_operation {
            self.bind_linked_stylesheet_import_blocking_operation(&fetch, blocking_operation, urls);
            return;
        }
        self.spawn_linked_stylesheet_import_graph(fetch, urls);
    }

    fn spawn_linked_stylesheet_import_graph(
        &mut self,
        fetch: crate::stylesheet_blocking::StylesheetFetch,
        urls: Vec<Url>,
    ) {
        let task_producer = self
            .stylesheet_lifecycle
            .task_producer
            .clone()
            .expect("linked stylesheet import requires a bound Page task producer");
        let stylesheet_fetcher = self.stylesheet_fetcher();
        let document_url = self
            .dom_host
            .node(self.dom_host.document_handle())
            .and_then(Node::as_document)
            .map(|document| document.url().clone())
            .expect("live dom host must retain a document url")
            .clone();
        let resource_loader = self
            .current_document_resource_loader()
            .expect("linked stylesheet import requires its Document authority");
        resource_loader.spawn_resource_task(async move {
            let (graph, network_results) =
                super::import_graph_projection::fetch_observed_stylesheet_import_graph(
                    stylesheet_fetcher,
                    document_url,
                    urls,
                    Vec::new(),
                )
                .await;
            let completion = LinkedStylesheetImportGraphCompletion {
                fetch,
                graph,
                network_results,
            };
            let _ = task_producer.send_linked_import_completion(completion);
        });
    }

    pub(crate) fn note_stylesheet_import_graph_completion(
        &mut self,
        fetch: &crate::stylesheet_blocking::StylesheetFetch,
        graph: Arc<crate::stylesheet_blocking::StylesheetImportGraphFetchResult>,
    ) {
        let graph = self
            .stylesheet_lifecycle
            .linked_stylesheet_import_graphs
            .complete(fetch, graph);
        let successful = graph.successful();
        let clients = self
            .stylesheet_lifecycle
            .owner_states
            .link_states()
            .filter(|(_, state)| state.active_load().fetch().ptr_eq(fetch))
            .map(|(_, state)| Arc::clone(state.active_load()))
            .collect::<Vec<_>>();
        for client in clients {
            self.note_stylesheet_link_import_completion(&client, successful);
        }
    }

    pub(crate) fn apply_linked_stylesheet_import_graph_completion(
        &mut self,
        completion: LinkedStylesheetImportGraphCompletion,
    ) {
        let LinkedStylesheetImportGraphCompletion {
            fetch,
            graph,
            mut network_results,
        } = completion;
        let roots = self
            .linked_stylesheet_import_roots(&fetch)
            .into_iter()
            .filter(|root| {
                self.dom_host.is_connected(root.owner)
                    && self
                        .stylesheet_lifecycle
                        .owner_states
                        .link_state(root.owner)
                        .is_some_and(|state| state.active_load().fetch().ptr_eq(&fetch))
            })
            .collect::<Vec<_>>();
        let source_owners = roots.iter().map(|root| root.owner).collect::<Vec<_>>();
        for result in &mut network_results {
            result.import_roots = roots.clone();
            result.source_owners = source_owners.clone();
        }
        self.stylesheet_lifecycle
            .ready_connected_load_network_results
            .extend(network_results);
        self.note_stylesheet_import_graph_completion(&fetch, graph);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{network::ResourceRequestClient, parser::HtmlParser};
    use moli_fetch::FetchConfig;

    #[tokio::test]
    async fn invalidated_blocking_transport_can_release_the_shared_graph() {
        let document_url = Url::parse("https://example.test/page").unwrap();
        let parser = HtmlParser;
        let first_document = parser.parse(
            document_url.clone(),
            "<!doctype html><style>@import url('data:text/css,.first%7Bcolor:red%7D');</style>"
                .to_owned(),
        );
        let first_input = crate::stylesheet_blocking::collect_document_owned_blocking_stylesheets(
            &first_document,
        )
        .into_iter()
        .next()
        .map(|blocker| {
            crate::stylesheet_blocking::DocumentOwnedBlockingStylesheetDiscoveryInput::from(
                &blocker,
            )
        })
        .expect("first parser-created import blocker");
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut runtime = DocumentRuntime::new_networked(&first_document, &loader);
        runtime.note_discovered_document_owned_blocking_stylesheet_inputs([&first_input]);
        let first_operation = runtime
            .stylesheet_lifecycle
            .fetches
            .blocking_operation(first_input.node_id(), first_input.signature())
            .expect("first blocking operation");
        let fetch = runtime
            .preload_stylesheet(
                Url::parse("data:text/css,.root%7Bcolor:blue%7D").unwrap(),
                crate::stylesheet_blocking::StylesheetFetchOptions::default(),
            )
            .expect("ownerless stylesheet fetch");
        assert!(matches!(
            runtime.admit_linked_stylesheet_import_graph(&fetch, None),
            LinkedStylesheetImportGraphAdmission::Start
        ));
        runtime.bind_linked_stylesheet_import_blocking_operation(
            &fetch,
            first_operation.clone(),
            vec![Url::parse("data:text/css,.first%7Bcolor:red%7D").unwrap()],
        );

        let second_document = parser.parse(
            document_url,
            "<!doctype html><style>@import url('data:text/css,.second%7Bcolor:green%7D');</style>"
                .to_owned(),
        );
        let second_input = crate::stylesheet_blocking::collect_document_owned_blocking_stylesheets(
            &second_document,
        )
        .into_iter()
        .next()
        .map(|blocker| {
            crate::stylesheet_blocking::DocumentOwnedBlockingStylesheetDiscoveryInput::from(
                &blocker,
            )
            .with_node_id(first_input.node_id())
        })
        .expect("replacement parser-created import blocker");
        runtime.note_discovered_document_owned_blocking_stylesheet_inputs([&second_input]);
        assert!(
            runtime
                .stylesheet_lifecycle
                .fetches
                .status_for_blocking_operation(&first_operation)
                .is_none(),
            "the old parser transport must lose canonical authority"
        );
        assert!(
            runtime
                .stylesheet_lifecycle
                .linked_stylesheet_import_graphs
                .replace_blocking_with_async(&fetch, &first_operation),
            "the shared graph must be able to continue independently"
        );
        assert!(
            runtime
                .linked_stylesheet_import_blocking_operations()
                .is_empty()
        );
    }
}
