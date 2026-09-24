use super::ScriptVm;
use crate::{context_bootstrap::NavigationHistoryPrunePlan, document_runtime::DomHandle};
use anyhow::{Result, anyhow};

#[derive(Clone, Copy, Debug)]
enum NavigationHistoryRealm {
    TopDefault,
    ChildDefault {
        execution_context_id: i64,
        child_handle: DomHandle,
    },
    PrebootstrappedChildDefault {
        child_handle: DomHandle,
    },
}

impl NavigationHistoryRealm {
    fn order_key(self) -> (usize, i64) {
        match self {
            Self::TopDefault => (0, 0),
            Self::ChildDefault {
                execution_context_id,
                child_handle,
            } => (child_handle.index().saturating_add(1), execution_context_id),
            Self::PrebootstrappedChildDefault { child_handle } => {
                (child_handle.index().saturating_add(1), i64::MIN)
            }
        }
    }
}

impl ScriptVm {
    pub(crate) fn reset_navigation_history(&mut self) -> Result<bool> {
        let realms = self.live_navigation_history_realms();
        let mut plans: Vec<(NavigationHistoryRealm, NavigationHistoryPrunePlan)> =
            Vec::with_capacity(realms.len());

        for realm in realms.iter().copied() {
            let Some(context_ptr) = self.navigation_history_realm_context_ptr(realm) else {
                return Ok(false);
            };
            let plan = self.with_context_scope_by_ptr(context_ptr, |scope, _host_ptr| {
                Ok(crate::context_bootstrap::plan_navigation_history_prune(
                    scope,
                ))
            })?;
            let Some(plan) = plan else {
                return Ok(false);
            };
            plans.push((realm, plan));
        }

        self.with_default_context_scope(|scope, _host_ptr| {
            crate::context_bootstrap::prune_joint_session_history(scope);
            Ok(())
        })?;

        for (realm, plan) in plans {
            let Some(context_ptr) = self.navigation_history_realm_context_ptr(realm) else {
                continue;
            };
            let pruned = self.with_context_scope_by_ptr(context_ptr, |scope, _host_ptr| {
                Ok(crate::context_bootstrap::apply_navigation_history_prune_plan(scope, &plan))
            })?;
            if !pruned {
                return Err(anyhow!(
                    "navigation history realm {realm:?} changed after reset preflight"
                ));
            }
        }

        // Dispose handlers can create a new child realm. Check its view without
        // pruning again: callbacks may have appended valid new joint steps.
        for realm in self.live_navigation_history_realms() {
            let Some(context_ptr) = self.navigation_history_realm_context_ptr(realm) else {
                continue;
            };
            let finalized = self.with_context_scope_by_ptr(context_ptr, |scope, _host_ptr| {
                Ok(crate::context_bootstrap::finalize_navigation_history_prune(
                    scope,
                ))
            })?;
            if !finalized {
                return Err(anyhow!(
                    "navigation history realm {realm:?} lost its History state during reset"
                ));
            }
        }

        // Page.resetNavigationHistory is one renderer task. Promise reactions
        // queued by dispose listeners run only after every live realm has been
        // updated.
        self.with_default_context_scope(|_scope, _host_ptr| Ok(()))?;
        Ok(true)
    }

    fn live_navigation_history_realms(&mut self) -> Vec<NavigationHistoryRealm> {
        self.prune_stale_child_default_execution_contexts();

        // Isolated worlds share these histories; prune each Window only once.
        let mut realms = vec![NavigationHistoryRealm::TopDefault];
        {
            let host = self._context_host.borrow();
            realms.extend(
                self.child_frame_realm_store
                    .iter_by_execution_context_id()
                    .map(
                        |(execution_context_id, realm)| NavigationHistoryRealm::ChildDefault {
                            execution_context_id,
                            child_handle: realm.child_handle,
                        },
                    ),
            );
            let prebootstrapped = self.prebootstrapped_child_default_contexts.borrow();
            realms.extend(
                prebootstrapped
                    .iter()
                    .filter_map(|(child_handle, context)| {
                        (host
                            .current_child_document_task_owner(*child_handle)
                            .map(|owner| owner.local_window_id)
                            == Some(context.local_window_id))
                        .then_some(NavigationHistoryRealm::PrebootstrappedChildDefault {
                            child_handle: *child_handle,
                        })
                    }),
            );
        }
        realms.sort_by_key(|realm| realm.order_key());
        realms
    }

    fn navigation_history_realm_context_ptr(
        &self,
        realm: NavigationHistoryRealm,
    ) -> Option<*const v8::Global<v8::Context>> {
        match realm {
            NavigationHistoryRealm::TopDefault => Some(&self.page_default_context as *const _),
            NavigationHistoryRealm::ChildDefault {
                execution_context_id,
                child_handle,
            } => {
                let owner_realm_id = self
                    .child_frame_owner_realm_id_for_execution_context_id(execution_context_id)
                    .ok()?;
                let realm = self
                    .child_frame_realm_store
                    .context_for_owner_realm_id(owner_realm_id)?;
                (realm.child_handle == child_handle).then_some(&realm.context as *const _)
            }
            NavigationHistoryRealm::PrebootstrappedChildDefault { child_handle } => {
                let current_local_window_id = self
                    ._context_host
                    .borrow()
                    .current_child_document_task_owner(child_handle)?
                    .local_window_id;
                let contexts = self.prebootstrapped_child_default_contexts.borrow();
                let context = contexts.get(&child_handle)?;
                (context.local_window_id == current_local_window_id)
                    .then_some(&context.context as *const _)
            }
        }
    }
}
