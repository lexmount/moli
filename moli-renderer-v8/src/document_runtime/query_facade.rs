use std::sync::atomic::{AtomicU32, Ordering};

use tracing::debug;

use super::*;

static HOT_SELECTOR_API_TRACE_COUNT: AtomicU32 = AtomicU32::new(0);

// This slice holds the selector/query facade.
//
// It is intentionally separate from the low-level DOM facade and the mutation commands because it
// is the one place where `DocumentRuntime` combines:
// - selector-engine entrypoints
// - selector-specific debug counters / hot-call tracing
//
// Keeping that combination together makes it easier to tighten or replace query behavior later
// without threading selector concerns back through the rest of `DocumentRuntime`.
impl DocumentRuntime {
    pub(crate) fn query_selector(
        &self,
        root: Option<DomHandle>,
        selector: &str,
    ) -> Result<Option<DomHandle>, SelectorError> {
        self.query_selector_with_optional_validity_states(root, selector, None)
    }

    pub(crate) fn query_selector_with_validity_states(
        &self,
        root: Option<DomHandle>,
        selector: &str,
        validity_states: &HashMap<DomHandle, bool>,
    ) -> Result<Option<DomHandle>, SelectorError> {
        self.query_selector_with_optional_validity_states(root, selector, Some(validity_states))
    }

    fn query_selector_with_optional_validity_states(
        &self,
        root: Option<DomHandle>,
        selector: &str,
        validity_states: Option<&HashMap<DomHandle, bool>>,
    ) -> Result<Option<DomHandle>, SelectorError> {
        self.selector_debug.record_query_selector();
        trace_hot_selector_api("querySelector", root, selector);
        match root {
            Some(root) => {
                if self.dom_host.node(root).is_some_and(Node::is_document) {
                    let handles = match validity_states {
                        Some(states) => self
                            .selector_engine
                            .query_selector_all_in_host_with_validity_states(
                                &self.dom_host,
                                root,
                                selector,
                                states,
                            )?,
                        None => self.selector_engine.query_selector_all_in_host(
                            &self.dom_host,
                            root,
                            selector,
                        )?,
                    };
                    Ok(handles.into_iter().next())
                } else {
                    match validity_states {
                        Some(states) => self
                            .selector_engine
                            .query_selector_in_host_with_validity_states(
                                &self.dom_host,
                                root,
                                selector,
                                states,
                            ),
                        None => self.selector_engine.query_selector_in_host(
                            &self.dom_host,
                            root,
                            selector,
                        ),
                    }
                }
            }
            None => match validity_states {
                Some(states) => self
                    .selector_engine
                    .query_selector_host_with_validity_states(&self.dom_host, selector, states),
                None => self
                    .selector_engine
                    .query_selector_host(&self.dom_host, selector),
            },
        }
    }

    pub(crate) fn query_selector_all(
        &self,
        root: Option<DomHandle>,
        selector: &str,
    ) -> Result<Vec<DomHandle>, SelectorError> {
        self.query_selector_all_with_optional_validity_states(root, selector, None)
    }

    pub(crate) fn query_selector_all_with_validity_states(
        &self,
        root: Option<DomHandle>,
        selector: &str,
        validity_states: &HashMap<DomHandle, bool>,
    ) -> Result<Vec<DomHandle>, SelectorError> {
        self.query_selector_all_with_optional_validity_states(root, selector, Some(validity_states))
    }

    fn query_selector_all_with_optional_validity_states(
        &self,
        root: Option<DomHandle>,
        selector: &str,
        validity_states: Option<&HashMap<DomHandle, bool>>,
    ) -> Result<Vec<DomHandle>, SelectorError> {
        self.selector_debug.record_query_selector_all();
        trace_hot_selector_api("querySelectorAll", root, selector);
        match root {
            Some(root) => match validity_states {
                Some(states) => self
                    .selector_engine
                    .query_selector_all_in_host_with_validity_states(
                        &self.dom_host,
                        root,
                        selector,
                        states,
                    ),
                None => {
                    self.selector_engine
                        .query_selector_all_in_host(&self.dom_host, root, selector)
                }
            },
            None => match validity_states {
                Some(states) => self
                    .selector_engine
                    .query_selector_all_host_with_validity_states(&self.dom_host, selector, states),
                None => self
                    .selector_engine
                    .query_selector_all_host(&self.dom_host, selector),
            },
        }
    }

    pub(crate) fn matches(&self, node: DomHandle, selector: &str) -> Result<bool, SelectorError> {
        self.matches_with_optional_validity_states(node, selector, None)
    }

    pub(crate) fn matches_with_validity_states(
        &self,
        node: DomHandle,
        selector: &str,
        validity_states: &HashMap<DomHandle, bool>,
    ) -> Result<bool, SelectorError> {
        self.matches_with_optional_validity_states(node, selector, Some(validity_states))
    }

    fn matches_with_optional_validity_states(
        &self,
        node: DomHandle,
        selector: &str,
        validity_states: Option<&HashMap<DomHandle, bool>>,
    ) -> Result<bool, SelectorError> {
        self.selector_debug.record_matches();
        trace_hot_selector_api("matches", Some(node), selector);
        match validity_states {
            Some(states) => self.selector_engine.matches_host_with_validity_states(
                &self.dom_host,
                node,
                selector,
                states,
            ),
            None => self
                .selector_engine
                .matches_host(&self.dom_host, node, selector),
        }
    }

    pub(crate) fn closest(
        &self,
        node: DomHandle,
        selector: &str,
    ) -> Result<Option<DomHandle>, SelectorError> {
        self.closest_with_optional_validity_states(node, selector, None)
    }

    pub(crate) fn closest_with_validity_states(
        &self,
        node: DomHandle,
        selector: &str,
        validity_states: &HashMap<DomHandle, bool>,
    ) -> Result<Option<DomHandle>, SelectorError> {
        self.closest_with_optional_validity_states(node, selector, Some(validity_states))
    }

    fn closest_with_optional_validity_states(
        &self,
        node: DomHandle,
        selector: &str,
        validity_states: Option<&HashMap<DomHandle, bool>>,
    ) -> Result<Option<DomHandle>, SelectorError> {
        self.selector_debug.record_closest();
        trace_hot_selector_api("closest", Some(node), selector);
        match validity_states {
            Some(states) => self.selector_engine.closest_host_with_validity_states(
                &self.dom_host,
                node,
                selector,
                states,
            ),
            None => self
                .selector_engine
                .closest_host(&self.dom_host, node, selector),
        }
    }

    pub(crate) fn selector_debug_snapshot(&self) -> SelectorDebugSnapshot {
        self.selector_debug.snapshot()
    }
}

fn trace_hot_selector_api(api: &str, root: Option<DomHandle>, selector: &str) {
    if !should_trace_hot_selector(selector) {
        return;
    }
    let count = HOT_SELECTOR_API_TRACE_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
    if count <= 60 || count.is_multiple_of(500) {
        let root = root
            .map(|handle| format!("{handle:?}"))
            .unwrap_or_else(|| "document".to_owned());
        debug!(%api, %root, %selector, count, "hot selector API call");
    }
}

fn should_trace_hot_selector(selector: &str) -> bool {
    let selector = selector.trim();
    selector.contains("script[")
        || selector.contains("link[")
        || selector.contains("[src")
        || selector.contains("[href")
}
