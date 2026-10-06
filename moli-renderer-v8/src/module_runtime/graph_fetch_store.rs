use std::collections::HashMap;
use std::fmt;

use super::ModuleScriptGraphFetchContinuation;

#[derive(Default)]
pub(crate) struct NativeModuleGraphFetchStore {
    inflight_module_script_fetches: HashMap<u64, ModuleScriptGraphFetchContinuation>,
    next_module_graph_fetch_load_id: u64,
}

impl fmt::Debug for NativeModuleGraphFetchStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeModuleGraphFetchStore")
            .field(
                "inflight_module_script_fetch_count",
                &self.inflight_module_script_fetches.len(),
            )
            .field(
                "next_module_graph_fetch_load_id",
                &self.next_module_graph_fetch_load_id,
            )
            .finish()
    }
}

impl NativeModuleGraphFetchStore {
    pub(crate) fn clear_for_context_teardown(&mut self) {
        self.inflight_module_script_fetches.clear();
    }

    pub(crate) fn suspend_fetch(
        &mut self,
        continuation: ModuleScriptGraphFetchContinuation,
    ) -> u64 {
        let load_id = self.reserve_load_id();
        self.inflight_module_script_fetches
            .insert(load_id, continuation);
        load_id
    }

    pub(crate) fn take_inflight_fetch(
        &mut self,
        load_id: u64,
    ) -> Option<ModuleScriptGraphFetchContinuation> {
        self.inflight_module_script_fetches.remove(&load_id)
    }

    pub(crate) fn has_inflight_fetch(&self, load_id: u64) -> bool {
        self.inflight_module_script_fetches.contains_key(&load_id)
    }

    pub(crate) fn reserve_load_id(&mut self) -> u64 {
        let load_id = self.next_module_graph_fetch_load_id;
        self.next_module_graph_fetch_load_id = self.next_module_graph_fetch_load_id.wrapping_add(1);
        load_id
    }
}
