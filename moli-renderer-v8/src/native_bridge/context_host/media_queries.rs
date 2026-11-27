use std::{collections::BTreeMap, rc::Rc};

use super::{
    PageScriptHost, RuntimeObservableContextToken, WindowDocumentOwner, WindowDocumentTaskTarget,
    WindowExecutionContextIdentity, WindowExecutionContextOwner,
};

struct MediaQuery {
    target: WindowDocumentTaskTarget,
    media: String,
    matches: bool,
    listening: bool,
    // A deferred environment task still owes this subscription a sample.
    sampling_pending: bool,
    queued: bool,
    // A frozen batch owns this entry until all its callbacks have completed.
    notifying: bool,
    execution_context: WindowExecutionContextIdentity,
    list: Rc<v8::Weak<v8::Object>>,
}

/// Upgrade the weak list only after releasing the registry borrow.
pub(crate) struct MediaQueryNotification {
    pub(crate) id: u64,
    pub(crate) media: String,
    pub(crate) execution_context: WindowExecutionContextIdentity,
    pub(crate) list: Rc<v8::Weak<v8::Object>>,
}

/// Shared by all layout policies. The realm roots subscribed or pending lists;
/// native state holds only weak handles and never roots a Context.
#[derive(Default)]
pub(super) struct MediaQueryState {
    next_query: u64,
    queries: BTreeMap<u64, MediaQuery>,
}

impl MediaQueryState {
    pub(super) fn rebind(
        &mut self,
        retired: WindowDocumentOwner,
        current: WindowDocumentOwner,
    ) -> bool {
        let mut pending = false;
        for query in self.queries.values_mut() {
            if query.target.owner() == retired {
                query.target =
                    WindowDocumentTaskTarget::new(current, query.target.dispatch_scope());
                pending |= query.sampling_pending || query.queued;
            }
        }
        pending
    }
}

impl PageScriptHost {
    pub(super) fn has_media_queries_for_target(&self, target: WindowDocumentTaskTarget) -> bool {
        self.media_queries
            .borrow()
            .queries
            .values()
            .any(|query| query.target == target)
    }

    pub(crate) fn has_media_query_notifications(&self) -> bool {
        self.media_queries
            .borrow()
            .queries
            .values()
            .any(|query| query.queued || query.sampling_pending)
    }

    pub(crate) fn allocate_media_query_id(&self) -> anyhow::Result<u64> {
        let mut state = self.media_queries.borrow_mut();
        state.next_query = state
            .next_query
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("media query identity exhausted"))?;
        Ok(state.next_query)
    }

    pub(crate) fn register_media_query<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        id: u64,
        target: WindowDocumentTaskTarget,
        media: String,
        matches: bool,
        list: v8::Local<'s, v8::Object>,
    ) -> anyhow::Result<()> {
        let context = list
            .get_creation_context(scope)
            .ok_or_else(|| anyhow::anyhow!("media query list has no creation context"))?;
        let execution_context = self
            .window_execution_context_identity_for_v8_context(scope, context)
            .ok_or_else(|| anyhow::anyhow!("media query list has no registered Realm"))?;
        anyhow::ensure!(
            self.window_execution_context_identity_is_current(execution_context)
                && execution_context.dispatch_scope() == target.dispatch_scope()
                && self.current_window_execution_context_owner(target.dispatch_scope())
                    == Some(execution_context.owner())
                && self.window_document_owner_is_current_for_dispatch_scope(
                    target.owner(),
                    target.dispatch_scope(),
                ),
            "media query list belongs to a retired Window document"
        );
        // Creating/upgrading V8 weak handles must not hold a state borrow.
        let list = Rc::new(v8::Weak::new(scope, list));
        self.media_queries
            .borrow_mut()
            .queries
            .entry(id)
            .and_modify(|query| {
                query.listening = true;
                // A deferred driver must compare against its pre-change match.
                if !query.sampling_pending {
                    query.matches = matches;
                }
            })
            .or_insert(MediaQuery {
                target,
                media,
                matches,
                listening: true,
                sampling_pending: false,
                queued: false,
                notifying: false,
                execution_context,
                list,
            });
        Ok(())
    }

    pub(crate) fn media_query_is_current(&self, target: WindowDocumentTaskTarget, id: u64) -> bool {
        self.media_queries
            .borrow()
            .queries
            .get(&id)
            .is_some_and(|query| {
                query.target == target
                    && self.window_document_owner_is_current_for_dispatch_scope(
                        target.owner(),
                        target.dispatch_scope(),
                    )
                    && self.window_execution_context_identity_is_current(query.execution_context)
            })
    }

    pub(crate) fn media_query_is_listening(&self, id: u64) -> bool {
        self.media_queries
            .borrow()
            .queries
            .get(&id)
            .is_some_and(|query| query.listening)
    }

    pub(crate) fn retire_media_query(&self, id: u64) {
        self.media_queries.borrow_mut().queries.remove(&id);
    }

    /// Keep pending sampling, queued events, and frozen batches alive across
    /// zero-listener gaps. The return value controls the realm-local root.
    pub(crate) fn stop_media_query_listening(&self, id: u64) -> bool {
        let mut state = self.media_queries.borrow_mut();
        let Some(query) = state.queries.get_mut(&id) else {
            return false;
        };
        query.listening = false;
        let retained = query.sampling_pending || query.queued || query.notifying;
        if !retained {
            state.queries.remove(&id);
        }
        retained
    }

    pub(crate) fn finish_media_query_notification(&self, id: u64) -> bool {
        let mut state = self.media_queries.borrow_mut();
        let Some(query) = state.queries.get_mut(&id) else {
            return false;
        };
        query.notifying = false;
        let retained = query.listening || query.sampling_pending || query.queued;
        if !retained {
            state.queries.remove(&id);
        }
        retained
    }

    pub(super) fn retire_media_query_owner(&self, owner: WindowExecutionContextOwner) {
        self.media_queries
            .borrow_mut()
            .queries
            .retain(|_, query| query.execution_context.owner() != owner);
    }

    pub(super) fn retire_media_query_realm(&self, token: RuntimeObservableContextToken) {
        self.media_queries
            .borrow_mut()
            .queries
            .retain(|_, query| query.execution_context.realm_token() != token);
    }

    #[cfg(test)]
    pub(crate) fn media_query_count_for_test(&self) -> usize {
        self.media_queries.borrow().queries.len()
    }

    /// The driver has accepted a deferred environment task. Retain existing
    /// subscriptions until it samples them, even if their listeners change.
    pub(super) fn defer_media_query_samples(&self, target: WindowDocumentTaskTarget) {
        for query in self.media_queries.borrow_mut().queries.values_mut() {
            if query.target == target && query.listening {
                query.sampling_pending = true;
            }
        }
    }

    /// Evaluate subscribed lists and pending work without invoking page JS.
    /// Drivers decide whether to sample at an input commit or a deferred turn.
    pub(super) fn sample_media_queries(&self, target: Option<WindowDocumentTaskTarget>) {
        let samples = self
            .media_queries
            .borrow()
            .queries
            .iter()
            .filter(|(_, query)| {
                (query.listening || query.sampling_pending)
                    && target.is_none_or(|target| target == query.target)
            })
            .filter(|(_, query)| {
                self.window_document_owner_is_current_for_dispatch_scope(
                    query.target.owner(),
                    query.target.dispatch_scope(),
                ) && self.window_execution_context_identity_is_current(query.execution_context)
            })
            .map(|(id, query)| {
                let matches = self
                    .window_document_handle_for_dispatch_scope(query.target.dispatch_scope())
                    .is_some_and(|document| {
                        self.media_query_matches_for_document(document, &query.media)
                    });
                (*id, matches)
            })
            .collect::<Vec<_>>();
        let mut state = self.media_queries.borrow_mut();
        for (id, matches) in samples {
            let query = state.queries.get_mut(&id).expect("sampled query");
            query.sampling_pending = false;
            if matches != query.matches {
                query.matches = matches;
                query.queued = true;
            }
        }
    }

    /// A deferred sample may settle without producing a change. Return weak
    /// lists after removing their idle entries so the caller can unroot them.
    pub(crate) fn retire_idle_media_queries(
        &self,
        target: WindowDocumentTaskTarget,
    ) -> Vec<(u64, Rc<v8::Weak<v8::Object>>)> {
        let mut state = self.media_queries.borrow_mut();
        let retired = state
            .queries
            .iter()
            .filter(|(_, query)| {
                query.target == target
                    && !query.listening
                    && !query.sampling_pending
                    && !query.queued
                    && !query.notifying
            })
            .map(|(id, query)| (*id, Rc::clone(&query.list)))
            .collect::<Vec<_>>();
        for (id, _) in &retired {
            state.queries.remove(id);
        }
        retired
    }

    pub(crate) fn restore_media_query_notifications(
        &self,
        target: WindowDocumentTaskTarget,
        ids: &[u64],
    ) {
        let mut state = self.media_queries.borrow_mut();
        for id in ids {
            if let Some(query) = state.queries.get_mut(id)
                && query.target == target
                && self.window_execution_context_identity_is_current(query.execution_context)
                && self.window_document_owner_is_current_for_dispatch_scope(
                    target.owner(),
                    target.dispatch_scope(),
                )
            {
                query.queued = true;
            }
        }
    }

    /// Freeze one Document's batch before callbacks; reentrant changes remain
    /// queued for a later batch, across every Realm of the same Document.
    pub(crate) fn take_media_query_notifications(
        &self,
        target: WindowDocumentTaskTarget,
    ) -> Vec<MediaQueryNotification> {
        self.media_queries
            .borrow_mut()
            .queries
            .iter_mut()
            .filter_map(|(id, query)| {
                if query.target == target && std::mem::take(&mut query.queued) {
                    query.notifying = true;
                    Some(MediaQueryNotification {
                        id: *id,
                        media: query.media.clone(),
                        execution_context: query.execution_context,
                        list: Rc::clone(&query.list),
                    })
                } else {
                    None
                }
            })
            .collect()
    }

    pub(super) fn prune_media_queries(&self) {
        self.media_queries.borrow_mut().queries.retain(|_, query| {
            self.window_document_owner_is_current_for_dispatch_scope(
                query.target.owner(),
                query.target.dispatch_scope(),
            ) && self.window_execution_context_identity_is_current(query.execution_context)
        });
    }
}
