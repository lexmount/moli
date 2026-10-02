use std::{fmt, sync::Arc};

use parking_lot::Mutex;

/// Mutable state owned by one top-level browsing context and shared by each
/// Document committed into that context.
#[derive(Clone, Default)]
pub struct RendererTopLevelBrowsingContextState {
    window_name_owner: Arc<Mutex<Arc<Mutex<String>>>>,
}

impl RendererTopLevelBrowsingContextState {
    pub fn window_name(&self) -> String {
        self.window_name_owner.lock().clone().lock().clone()
    }

    pub fn set_window_name(&self, value: String) {
        *self.window_name_owner.lock().clone().lock() = value;
    }

    pub fn shares_identity_with(&self, other: &Self) -> bool {
        if Arc::ptr_eq(&self.window_name_owner, &other.window_name_owner) {
            return true;
        }
        let left = self.window_name_owner.lock().clone();
        let right = other.window_name_owner.lock().clone();
        Arc::ptr_eq(&left, &right)
    }

    pub fn bind_to(&self, owner: &Self) {
        if Arc::ptr_eq(&self.window_name_owner, &owner.window_name_owner) {
            return;
        }
        let owner = owner.window_name_owner.lock().clone();
        *self.window_name_owner.lock() = owner;
    }
}

impl fmt::Debug for RendererTopLevelBrowsingContextState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RendererTopLevelBrowsingContextState")
            .field(
                "owner_strong_count",
                &Arc::strong_count(&self.window_name_owner),
            )
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::RendererTopLevelBrowsingContextState;

    #[test]
    fn binding_redirects_both_handles_to_one_window_name_owner() {
        let owner = RendererTopLevelBrowsingContextState::default();
        owner.set_window_name("target".to_owned());
        let proxy = RendererTopLevelBrowsingContextState::default();
        proxy.set_window_name("proxy-before-bind".to_owned());

        proxy.bind_to(&owner);
        assert!(proxy.shares_identity_with(&owner));
        assert_eq!(proxy.window_name(), "target");

        proxy.set_window_name("from-proxy".to_owned());
        assert_eq!(owner.window_name(), "from-proxy");
        owner.set_window_name("from-target".to_owned());
        assert_eq!(proxy.window_name(), "from-target");
    }
}
