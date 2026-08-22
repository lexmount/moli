//! Completion routing for owner-bound stylesheet and connected-load objects.

use super::*;

#[derive(Debug, Clone)]
pub(crate) struct ConnectedLoadCompletion {
    pub(in crate::document_runtime) operation: Arc<ConnectedLoadOperation>,
    pub(in crate::document_runtime) successful: bool,
    pub(in crate::document_runtime) network_results: Vec<ConnectedLoadNetworkResult>,
}

/// Network terminals for imports introduced by a CSSOM mutation after the
/// owning `<style>`/`<link>` load lifecycle has already completed.
///
/// These results remain observable as Networking work, but must not reopen or
/// complete the element's load/error state. The captured native roots provide
/// their own stylesheet/import generation authority when ScriptVm installs the
/// fetched child sheets.
#[derive(Debug, Clone)]
pub(crate) struct LiveStylesheetImportLoadCompletion {
    pub(in crate::document_runtime) network_results: Vec<ConnectedLoadNetworkResult>,
}

impl DocumentRuntime {
    pub(crate) fn apply_live_stylesheet_import_load_completion(
        &mut self,
        mut completion: LiveStylesheetImportLoadCompletion,
        install_authority: bool,
    ) {
        if !install_authority {
            for result in &mut completion.network_results {
                result.import_roots.clear();
                result.source_owners.clear();
            }
        }
        self.stylesheet_lifecycle
            .ready_connected_load_network_results
            .extend(completion.network_results);
    }

    pub(super) fn install_stylesheet_link_state(
        &mut self,
        owner: DomHandle,
        state: LinkStyleState,
    ) {
        let load = Arc::clone(state.active_load());
        if let Some(replaced) = self
            .stylesheet_lifecycle
            .owner_states
            .replace_link_state(owner, state)
        {
            self.stylesheet_lifecycle
                .link_client_index
                .unregister(replaced.active_load());
            self.unregister_linked_stylesheet_import_consumer(replaced.active_load());
        }
        if load.fetch().terminal().is_none() {
            self.stylesheet_lifecycle.link_client_index.register(load);
        }
    }

    pub(super) fn invalidate_stylesheet_owner_operations(&mut self, owner: DomHandle) {
        if let Some(load) = self
            .stylesheet_lifecycle
            .owner_states
            .link_state(owner)
            .map(|state| Arc::clone(state.active_load()))
        {
            self.stylesheet_lifecycle
                .link_client_index
                .unregister(&load);
            self.unregister_linked_stylesheet_import_consumer(&load);
        }
        self.stylesheet_lifecycle
            .owner_states
            .invalidate_owner_operations(owner);
    }

    pub(super) fn promote_stylesheet_link_client_if_ready(
        &mut self,
        load: Arc<StylesheetLinkClient>,
    ) -> Option<StylesheetLinkClientTerminal> {
        let active_link_clients = self.stylesheet_lifecycle.link_client_index.len();
        self.promote_stylesheet_link_client_candidates(vec![load], active_link_clients)
            .into_iter()
            .next()
    }

    pub(super) fn promote_stylesheet_link_clients_for_fetch(
        &mut self,
        fetch: &crate::stylesheet_blocking::StylesheetFetch,
    ) {
        let active_link_clients = self.stylesheet_lifecycle.link_client_index.len();
        let clients = self
            .stylesheet_lifecycle
            .link_client_index
            .take_for_fetch(fetch);
        let terminals =
            self.promote_stylesheet_link_client_candidates(clients, active_link_clients);
        self.stylesheet_lifecycle
            .ready_stylesheet_link_client_terminals
            .extend(terminals);
    }

    fn promote_stylesheet_link_client_candidates(
        &mut self,
        candidates: Vec<Arc<StylesheetLinkClient>>,
        active_link_clients: usize,
    ) -> Vec<StylesheetLinkClientTerminal> {
        #[cfg(test)]
        let trace_enabled = true;
        #[cfg(not(test))]
        let trace_enabled = moli_trace::cdp_nav_timing_enabled();
        let trace_started = trace_enabled.then(std::time::Instant::now);
        let mut inspected_link_states = 0_u64;
        let mut promoted_clients = 0_u64;
        let mut terminals = Vec::new();
        for load in candidates {
            if trace_enabled {
                inspected_link_states += 1;
            }
            let Some(terminal) = load.fetch().terminal() else {
                continue;
            };
            let handle = load.owner();
            let successful = terminal.is_ready();
            let (accepted, ready) = self
                .stylesheet_lifecycle
                .owner_states
                .link_state_mut(handle)
                .map(|state| {
                    let accepted = state.accept_resource_completion(&load, successful);
                    let ready = accepted.then(|| state.take_ready_event()).flatten();
                    (accepted, ready)
                })
                .unwrap_or((false, None));
            if accepted {
                promoted_clients += 1;
                terminals.push(StylesheetLinkClientTerminal::new(
                    Arc::clone(&load),
                    terminal,
                ));
            }
            if let Some((load, successful)) = ready {
                self.push_ready_connected_style_load(ReadyConnectedStyleLoad::for_stylesheet_link(
                    load, successful,
                ));
            }
        }
        if let Some(started) = trace_started {
            let elapsed_us = started.elapsed().as_micros();
            let trace = &mut self.stylesheet_lifecycle.link_promotion_trace;
            trace.promotion_count += 1;
            trace.inspected_link_states += inspected_link_states;
            trace.promoted_clients += promoted_clients;
            trace.max_indexed_link_clients =
                trace.max_indexed_link_clients.max(active_link_clients);
            trace.total_elapsed_us += elapsed_us;
            tracing::info!(
                target: "moli_cdp_nav_timing",
                inspected_link_states,
                promoted_clients,
                elapsed_us,
                promotion_count = trace.promotion_count,
                cumulative_inspected_link_states = trace.inspected_link_states,
                cumulative_promoted_clients = trace.promoted_clients,
                max_indexed_link_clients = trace.max_indexed_link_clients,
                cumulative_elapsed_us = trace.total_elapsed_us,
                stage = "stylesheet_link_client_promotion_done",
            );
        }
        terminals
    }

    pub(super) fn note_connected_style_import_completion(
        &mut self,
        operation: &Arc<ConnectedLoadOperation>,
        successful: bool,
    ) {
        self.push_ready_connected_style_load(ReadyConnectedStyleLoad::for_operation(
            Arc::clone(operation),
            successful,
        ));
    }

    pub(super) fn note_stylesheet_link_import_completion(
        &mut self,
        load: &Arc<StylesheetLinkClient>,
        successful: bool,
    ) {
        let ready = self
            .stylesheet_lifecycle
            .owner_states
            .link_state_mut(load.owner())
            .and_then(|state| {
                StylesheetLinkClient::ptr_eq(state.active_load(), load)
                    .then(|| state.accept_import_completion(successful))
                    .filter(|accepted| *accepted)
                    .and_then(|_| state.take_ready_event())
            });
        if let Some((load, successful)) = ready {
            self.push_ready_connected_style_load(ReadyConnectedStyleLoad::for_stylesheet_link(
                load, successful,
            ));
        }
    }

    pub(in crate::document_runtime) fn apply_network_result_install_authority(
        &mut self,
        mut result: ConnectedLoadNetworkResult,
    ) -> ConnectedLoadNetworkResult {
        if result.resource_type != SubresourceResourceType::Stylesheet {
            result.source_owners.clear();
            result.import_roots.clear();
            if let Some(operation) = &result.source_operation {
                let _ = self
                    .stylesheet_lifecycle
                    .owner_states
                    .consume_source_result(operation);
            }
            return result;
        }
        if !result.import_roots.is_empty() {
            result.source_owners = result.import_roots.iter().map(|root| root.owner).collect();
            return result;
        }
        if result.stylesheet_fetch.is_some() {
            result.source_owners.clear();
            result.import_roots.clear();
            return result;
        }
        if result.blocking_operation.is_some() {
            // Parser-blocking imports publish their physical terminals here,
            // but their live install authority may not exist until the parser
            // admits the owning CSSStyleSheet. The retained one-shot graph is
            // consumed through ReadyBlockingStyleImportGraph instead of
            // attaching an opportunistic owner/root to these network records.
            result.source_owners.clear();
            result.import_roots.clear();
            return result;
        }
        let Some(operation) = result.source_operation.as_ref() else {
            result.source_owners.clear();
            result.import_roots.clear();
            return result;
        };
        let accepted = self.dom_host.is_connected(operation.owner)
            && self
                .stylesheet_lifecycle
                .owner_states
                .accepts_source_result(operation);
        result.source_owners = accepted
            .then_some(vec![operation.owner])
            .unwrap_or_default();
        result.import_roots = if accepted {
            match &operation.parameters {
                ConnectedLoadParameters::StyleImports { roots, .. } => roots
                    .iter()
                    .filter(|root| root.owner == operation.owner)
                    .cloned()
                    .collect(),
                _ => Vec::new(),
            }
        } else {
            Vec::new()
        };
        if accepted {
            let _ = self
                .stylesheet_lifecycle
                .owner_states
                .consume_source_result(operation);
        }
        result
    }

    pub(crate) fn apply_connected_style_load_completion(
        &mut self,
        completion: ConnectedLoadCompletion,
    ) {
        let ConnectedLoadCompletion {
            operation,
            successful,
            mut network_results,
        } = completion;
        let (source_result_count, event_pending) = match &operation.parameters {
            ConnectedLoadParameters::ImmediateOwnerProcessing
            | ConnectedLoadParameters::PreloadLikeLink { .. } => (0, true),
            ConnectedLoadParameters::StyleImports { .. } => (network_results.len(), true),
        };
        if !self.stylesheet_lifecycle.owner_states.accept_completion(
            &operation,
            source_result_count,
            event_pending,
        ) {
            for result in &mut network_results {
                result.source_owners.clear();
                result.source_operation = None;
            }
            self.stylesheet_lifecycle
                .ready_connected_load_network_results
                .extend(network_results);
            return;
        }
        match &operation.parameters {
            ConnectedLoadParameters::ImmediateOwnerProcessing
            | ConnectedLoadParameters::PreloadLikeLink { .. } => {
                if let Some(result) = network_results.into_iter().next() {
                    self.stylesheet_lifecycle
                        .ready_connected_load_network_results
                        .push_back(result);
                }
                self.push_ready_connected_style_load(ReadyConnectedStyleLoad::for_operation(
                    Arc::clone(&operation),
                    successful,
                ));
            }
            ConnectedLoadParameters::StyleImports { .. } => {
                for mut result in network_results {
                    result.source_operation = Some(Arc::clone(&operation));
                    self.stylesheet_lifecycle
                        .ready_connected_load_network_results
                        .push_back(result);
                }
                self.note_connected_style_import_completion(&operation, successful);
            }
        }
    }

    #[cfg(test)]
    pub(in crate::document_runtime) fn drain_ready_connected_style_load_completions(&mut self) {
        self.apply_ready_stylesheet_networking_tasks_for_test();
        let completed_fetches = self.stylesheet_lifecycle.fetches.drain_ready_completions();
        for fetch in completed_fetches {
            self.promote_stylesheet_link_clients_for_fetch(&fetch);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{network::ResourceRequestClient, parser::HtmlParser};
    use anyhow::Result;
    use moli_fetch::FetchConfig;
    use url::Url;

    async fn stylesheet_link_promotion_probe(
        client_count: usize,
    ) -> Result<(StylesheetLinkPromotionTrace, StylesheetLinkPromotionTrace)> {
        let links = std::iter::repeat_n(
            "<link rel=preload as=style href='data:text/css,.shared%7Bcolor%3Agreen%7D'>",
            client_count,
        )
        .collect::<String>();
        let document = HtmlParser.parse(
            Url::parse("https://example.test/page").unwrap(),
            format!("<!doctype html><html><head>{links}</head><body></body></html>"),
        );
        let loader = ResourceRequestClient::new(&FetchConfig::default())?;
        let mut runtime = DocumentRuntime::new_networked(&document, &loader);

        runtime.queue_initial_connected_style_loads();
        runtime.prime_pending_connected_style_loads();
        assert_eq!(
            runtime
                .stylesheet_lifecycle
                .owner_states
                .link_states()
                .count(),
            client_count
        );
        let admission = std::mem::take(&mut runtime.stylesheet_lifecycle.link_promotion_trace);
        assert_eq!(admission.inspected_link_states, client_count as u64);
        assert_eq!(
            runtime.stylesheet_lifecycle.link_client_index.len(),
            client_count
        );

        assert!(
            runtime.wait_for_stylesheet_networking_task_for_test().await,
            "shared stylesheet fetch should publish one terminal"
        );
        assert!(runtime.apply_next_stylesheet_networking_task_for_test());
        let terminal = runtime.stylesheet_lifecycle.link_promotion_trace;
        assert_eq!(terminal.inspected_link_states, client_count as u64);
        assert_eq!(terminal.promoted_clients, client_count as u64);
        assert_eq!(runtime.stylesheet_lifecycle.link_client_index.len(), 0);

        Ok((admission, terminal))
    }

    #[tokio::test]
    async fn stylesheet_link_promotion_probe_reports_fanout_work() -> Result<()> {
        for client_count in [1, 32, 128, 512] {
            let (admission, terminal) = stylesheet_link_promotion_probe(client_count).await?;
            eprintln!(
                "clients={client_count} admission_calls={} admission_inspected={} \
                 admission_us={} terminal_us={}",
                admission.promotion_count,
                admission.inspected_link_states,
                admission.total_elapsed_us,
                terminal.total_elapsed_us,
            );
            assert_eq!(admission.max_indexed_link_clients, client_count);
        }
        Ok(())
    }
}
