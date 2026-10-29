use std::collections::HashMap;

const FRONTEND_NODE_ID_START: u32 = 1;

#[derive(Debug, Clone)]
pub(super) struct RendererFrontendNodeBindings {
    next_id: u32,
    frontend_to_backend: HashMap<u32, u32>,
    backend_to_frontend: HashMap<u32, u32>,
}

impl Default for RendererFrontendNodeBindings {
    fn default() -> Self {
        Self {
            next_id: FRONTEND_NODE_ID_START,
            frontend_to_backend: HashMap::new(),
            backend_to_frontend: HashMap::new(),
        }
    }
}

impl RendererFrontendNodeBindings {
    pub(super) fn clear(&mut self) {
        self.frontend_to_backend.clear();
        self.backend_to_frontend.clear();
    }

    pub(super) fn id_for_backend_node_id(&mut self, backend_node_id: u32) -> u32 {
        if let Some(frontend_node_id) = self.backend_to_frontend.get(&backend_node_id).copied() {
            return frontend_node_id;
        }

        let frontend_node_id = self.allocate_id();
        self.frontend_to_backend
            .insert(frontend_node_id, backend_node_id);
        self.backend_to_frontend
            .insert(backend_node_id, frontend_node_id);
        frontend_node_id
    }

    pub(super) fn backend_node_id_for_frontend_node_id(
        &self,
        frontend_node_id: u32,
    ) -> Option<u32> {
        self.frontend_to_backend.get(&frontend_node_id).copied()
    }

    pub(super) fn has_backend_node_id(&self, backend_node_id: u32) -> bool {
        self.backend_to_frontend.contains_key(&backend_node_id)
    }

    pub(super) fn frontend_node_id_for_backend_node_id(&self, backend_node_id: u32) -> Option<u32> {
        self.backend_to_frontend.get(&backend_node_id).copied()
    }

    pub(super) fn remove_backend_node_id(&mut self, backend_node_id: u32) {
        let Some(frontend_node_id) = self.backend_to_frontend.remove(&backend_node_id) else {
            return;
        };
        self.frontend_to_backend.remove(&frontend_node_id);
    }

    fn allocate_id(&mut self) -> u32 {
        let frontend_node_id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("renderer frontend node id namespace exhausted");
        frontend_node_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontend_registry_reuses_ids_for_backend_nodes() {
        let mut bindings = RendererFrontendNodeBindings::default();
        let backend_node_id = moli_page_types::RENDERER_BACKEND_NODE_ID_START;
        let first = bindings.id_for_backend_node_id(backend_node_id);
        let second = bindings.id_for_backend_node_id(backend_node_id);

        assert_eq!(first, 1);
        assert_eq!(first, second);
        assert_eq!(
            bindings.backend_node_id_for_frontend_node_id(first),
            Some(backend_node_id)
        );
    }

    #[test]
    fn removing_a_binding_does_not_reuse_its_frontend_id() {
        let mut bindings = RendererFrontendNodeBindings::default();
        let first_backend_node_id = moli_page_types::RENDERER_BACKEND_NODE_ID_START;
        let second_backend_node_id = first_backend_node_id + 1;
        let first = bindings.id_for_backend_node_id(first_backend_node_id);
        bindings.remove_backend_node_id(first_backend_node_id);
        let second = bindings.id_for_backend_node_id(second_backend_node_id);
        let rebound = bindings.id_for_backend_node_id(first_backend_node_id);

        assert_eq!(bindings.backend_node_id_for_frontend_node_id(first), None);
        assert!(second > first);
        assert!(rebound > second);
        assert_eq!(
            bindings.backend_node_id_for_frontend_node_id(second),
            Some(second_backend_node_id)
        );
        assert_eq!(
            bindings.backend_node_id_for_frontend_node_id(rebound),
            Some(first_backend_node_id)
        );
    }

    #[test]
    fn clearing_bindings_preserves_frontend_id_namespace() {
        let mut bindings = RendererFrontendNodeBindings::default();
        let first_backend_node_id = moli_page_types::RENDERER_BACKEND_NODE_ID_START;
        let second_backend_node_id = first_backend_node_id + 1;

        let first_frontend_node_id = bindings.id_for_backend_node_id(first_backend_node_id);
        bindings.clear();
        let second_frontend_node_id = bindings.id_for_backend_node_id(second_backend_node_id);

        assert_eq!(first_frontend_node_id, 1);
        assert_eq!(second_frontend_node_id, 2);
        assert_eq!(
            bindings.backend_node_id_for_frontend_node_id(first_frontend_node_id),
            None
        );
        assert_eq!(
            bindings.backend_node_id_for_frontend_node_id(second_frontend_node_id),
            Some(second_backend_node_id)
        );
    }
}
