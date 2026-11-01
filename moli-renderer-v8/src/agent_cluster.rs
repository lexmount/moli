use crate::{resource_owner::ResourceOwnerId, runtime::RendererStoragePartitionIdentity};
use std::sync::Arc;

/// Native identity shared by Window access checks and shared-memory cloning.
/// Permissions Policy controls API exposure separately from this identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AgentClusterIdentity(Arc<AgentClusterKey>);

#[derive(Debug, Eq, PartialEq)]
enum AgentClusterKey {
    Window {
        partition: RendererStoragePartitionIdentity,
        origin: String,
        isolation: WindowAgentClusterIsolation,
    },
    Worker(ResourceOwnerId),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum WindowAgentClusterIsolation {
    CoopCoep { isolation_origin: String },
    DocumentIsolation,
}

impl AgentClusterIdentity {
    pub(crate) fn window(
        partition: RendererStoragePartitionIdentity,
        origin: String,
        isolation: WindowAgentClusterIsolation,
    ) -> Self {
        Self(Arc::new(AgentClusterKey::Window {
            partition,
            origin,
            isolation,
        }))
    }

    pub(crate) fn worker(owner: ResourceOwnerId) -> Self {
        Self(Arc::new(AgentClusterKey::Worker(owner)))
    }

    pub(crate) fn window_security_token_key(&self, prefix: &str) -> Option<String> {
        let AgentClusterKey::Window {
            partition,
            origin,
            isolation,
        } = self.0.as_ref()
        else {
            return None;
        };
        let (kind, common_origin) = match isolation {
            WindowAgentClusterIsolation::CoopCoep { isolation_origin } => {
                ("coop-coep", isolation_origin.as_str())
            }
            WindowAgentClusterIsolation::DocumentIsolation => ("document", origin.as_str()),
        };
        let browser = partition.browser_context_id();
        let profile = partition.profile_partition_id();
        // Lengths make the representation injective even for arbitrary native
        // partition identifiers. The caller supplies a distinct world namespace.
        Some(format!(
            "{prefix}{}:{browser}{}:{profile}{}:{origin}{}:{kind}{}:{common_origin}",
            browser.len(),
            profile.len(),
            origin.len(),
            kind.len(),
            common_origin.len(),
        ))
    }
}
