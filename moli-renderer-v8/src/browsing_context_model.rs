//! Browsing-context primitives shared by top-level and nested contexts.
//!
//! This module deliberately does not know about iframe owner elements, Page
//! targets, or popup activation carriers. Those are adapters around the
//! browser concepts represented here: a stable browsing-context identity, a
//! stable WindowProxy, replaceable LocalWindow/Document generations, and realm
//! materialization state.

use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_BROWSING_CONTEXT_GROUP_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) enum BrowsingContextKind {
    PrimaryTopLevel,
    AuxiliaryTopLevel,
    Nested,
}

/// Renderer owner identity for one browsing-context group.
///
/// This is deliberately distinct from a script-agent id: related Pages may
/// share an agent today, while a future remote group can contain multiple
/// agents. A COOP commit allocates a new group before its replacement realm is
/// made observable.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct BrowsingContextGroupId(u64);

impl BrowsingContextGroupId {
    pub(crate) fn allocate() -> Self {
        let value = NEXT_BROWSING_CONTEXT_GROUP_ID
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .expect("browsing-context-group id allocator overflow");
        Self(value)
    }

    pub(crate) const fn value(self) -> u64 {
        self.0
    }
}

/// Group-qualified identity of one top-level WindowProxy routing endpoint.
///
/// A normal Document replacement preserves this identity. A browsing-context
/// group switch allocates a new group and therefore a new endpoint even when
/// the protocol Page residence is reused. The generation is allocated by the
/// group owner and prevents a stale V8 projection from addressing a later
/// top-level target that happens to reuse the same Page residence.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct TopLevelWindowProxyEndpointId {
    browsing_context_group_id: BrowsingContextGroupId,
    generation: u64,
}

impl TopLevelWindowProxyEndpointId {
    pub(crate) const fn new(
        browsing_context_group_id: BrowsingContextGroupId,
        generation: u64,
    ) -> Self {
        assert!(
            generation != 0,
            "WindowProxy endpoint generation must be non-zero"
        );
        Self {
            browsing_context_group_id,
            generation,
        }
    }

    pub(crate) const fn from_wire_parts(
        browsing_context_group_id: u64,
        generation: u64,
    ) -> Option<Self> {
        if browsing_context_group_id == 0 || generation == 0 {
            return None;
        }
        Some(Self {
            browsing_context_group_id: BrowsingContextGroupId(browsing_context_group_id),
            generation,
        })
    }

    pub(crate) const fn browsing_context_group_id(self) -> BrowsingContextGroupId {
        self.browsing_context_group_id
    }

    pub(crate) const fn generation(self) -> u64 {
        self.generation
    }
}

/// Owner-runtime identity of one browsing context.
///
/// The numeric namespace is intentionally qualified by kind so nested and
/// auxiliary ids cannot alias while allocator ownership is being migrated.
/// A browsing-context group must ultimately allocate all related top-level ids
/// from one authority; neither DOM owner handles nor protocol target ids are
/// valid substitutes for that browser identity.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct BrowsingContextId {
    kind: BrowsingContextKind,
    value: u64,
}

/// Stable identity of the V8/script execution agent hosting one or more realms.
///
/// Fresh top-level Pages receive a new agent, while nested realms and
/// same-Page navigation generations reuse their Page's agent. An explicitly
/// related auxiliary Page may join its opener's agent; unrelated and
/// opener-suppressed Pages stay isolated.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct ScriptAgentId(u64);

impl ScriptAgentId {
    pub(crate) const fn new(value: u64) -> Self {
        assert!(value != 0, "script-agent id must be non-zero");
        Self(value)
    }

    pub(crate) const fn value(self) -> u64 {
        self.0
    }
}

/// Admission scope currently retaining a script agent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScriptAgentScope {
    PageScriptEnvironment,
    RelatedPageGroup,
}

impl ScriptAgentScope {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::PageScriptEnvironment => "page-script-environment",
            Self::RelatedPageGroup => "related-page-group",
        }
    }
}

impl BrowsingContextId {
    pub(crate) const fn primary_top_level() -> Self {
        Self {
            kind: BrowsingContextKind::PrimaryTopLevel,
            value: 0,
        }
    }

    pub(crate) const fn nested(value: u64) -> Self {
        assert!(value != 0, "nested browsing-context id must be non-zero");
        Self {
            kind: BrowsingContextKind::Nested,
            value,
        }
    }

    pub(crate) const fn auxiliary_top_level(value: u64) -> Self {
        assert!(value != 0, "auxiliary browsing-context id must be non-zero");
        Self {
            kind: BrowsingContextKind::AuxiliaryTopLevel,
            value,
        }
    }

    pub(crate) const fn kind(self) -> BrowsingContextKind {
        self.kind
    }

    pub(crate) const fn value(self) -> u64 {
        self.value
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct WindowProxyId(pub(crate) u64);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct LocalWindowId(pub(crate) u64);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct DocumentId(pub(crate) u64);

/// Owner-model state for one stable WindowProxy.
///
/// The current inner LocalWindow may change or disappear while this record and
/// its browsing-context identity remain stable.
#[derive(Clone, Debug)]
pub(crate) struct StableWindowProxyRecord {
    pub(crate) id: WindowProxyId,
    pub(crate) browsing_context_id: BrowsingContextId,
    pub(crate) current_local_window_id: Option<LocalWindowId>,
    pub(crate) reachability: WindowProxyReachability,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WindowProxyReachability {
    Live,
    DetachedReachable,
}

/// Origin projection used by WindowProxy access checks.
///
/// The opaque identity is generic so the comparison primitive does not choose
/// its owner. Window access supplies a browser-context-qualified origin nonce;
/// tuple-origin comparison remains shared by nested and top-level contexts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum BrowsingContextAccessOrigin<OpaqueIdentity> {
    Opaque {
        identity: Option<OpaqueIdentity>,
    },
    Tuple {
        serialized_origin: String,
        scheme: String,
        document_domain: Option<String>,
    },
}

impl<OpaqueIdentity> BrowsingContextAccessOrigin<OpaqueIdentity> {
    #[cfg(test)]
    pub(crate) fn opaque(identity: OpaqueIdentity) -> Self {
        Self::Opaque {
            identity: Some(identity),
        }
    }

    pub(crate) fn from_serialized_origin(
        serialized_origin: String,
        document_domain: Option<String>,
    ) -> Option<Self> {
        if serialized_origin == "null" {
            return Some(Self::Opaque { identity: None });
        }
        let scheme = url::Url::parse(&serialized_origin)
            .ok()?
            .scheme()
            .to_owned();
        Some(Self::Tuple {
            serialized_origin,
            scheme,
            document_domain,
        })
    }
}

impl<OpaqueIdentity> BrowsingContextAccessOrigin<OpaqueIdentity>
where
    OpaqueIdentity: Eq,
{
    pub(crate) fn can_access(&self, target: &Self) -> bool {
        if let (
            Self::Opaque {
                identity: Some(accessing_identity),
            },
            Self::Opaque {
                identity: Some(target_identity),
            },
        ) = (self, target)
        {
            return accessing_identity == target_identity;
        }
        let (
            Self::Tuple {
                serialized_origin: accessing_origin,
                scheme: accessing_scheme,
                document_domain: accessing_domain,
            },
            Self::Tuple {
                serialized_origin: target_origin,
                scheme: target_scheme,
                document_domain: target_domain,
            },
        ) = (self, target)
        else {
            return false;
        };
        match (accessing_domain, target_domain) {
            (None, None) => accessing_origin == target_origin,
            (Some(accessing_domain), Some(target_domain)) => {
                accessing_scheme == target_scheme && accessing_domain == target_domain
            }
            _ => false,
        }
    }
}

/// The LocalWindow identity change committed with a Document-owner change.
///
/// This is generic over the generation id so a nested frame and an auxiliary
/// top-level Page can report the same transition without sharing their owner
/// stores prematurely.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LocalWindowOwnerTransition<Id> {
    Installed { current: Id },
    Preserved { current: Id },
    Replaced { retired: Id, current: Id },
    Retired { retired: Id },
}

impl<Id> LocalWindowOwnerTransition<Id>
where
    Id: Copy + Eq,
{
    pub(crate) fn between(retired: Option<Id>, current: Option<Id>) -> Self {
        match (retired, current) {
            (None, Some(current)) => Self::Installed { current },
            (Some(retired), Some(current)) if retired == current => Self::Preserved { current },
            (Some(retired), Some(current)) => Self::Replaced { retired, current },
            (Some(retired), None) => Self::Retired { retired },
            (None, None) => {
                unreachable!("a document owner transition must install or retire an owner")
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DocumentLocalWindowTransition {
    ReplaceLocalWindow,
    ReuseInitialEmptyLocalWindow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DocumentCreationKind {
    InitialEmpty,
    Navigation,
    Srcdoc,
    JavascriptUrl,
    DocumentOpen,
}

impl DocumentCreationKind {
    pub(crate) fn is_initial_empty(self) -> bool {
        matches!(self, Self::InitialEmpty)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RealmMaterializationRequest<RealmId> {
    NewlyQueued { realm_id: RealmId },
    AlreadyQueued { realm_id: RealmId },
    AlreadyMaterialized { realm_id: RealmId },
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) enum RealmAccessPolicy {
    #[default]
    EnforceWebOrigin,
    Universal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RealmWorldKind<AccessPolicy> {
    Default,
    Isolated { access_policy: AccessPolicy },
}

impl<AccessPolicy> RealmWorldKind<AccessPolicy>
where
    AccessPolicy: Copy + Default,
{
    pub(crate) fn access_policy(self) -> AccessPolicy {
        match self {
            Self::Default => AccessPolicy::default(),
            Self::Isolated { access_policy } => access_policy,
        }
    }

    pub(crate) fn is_default(self) -> bool {
        matches!(self, Self::Default)
    }
}

/// Exact realm projection requested from a script-agent host.
///
/// Embedding-specific data such as an iframe owner handle or a Page residence
/// stays in the adapter around this value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RealmHostProjection<ContextId, Owner, RealmToken, World> {
    browsing_context_id: ContextId,
    owner: Owner,
    realm_token: RealmToken,
    world: World,
}

impl<ContextId, Owner, RealmToken, World> RealmHostProjection<ContextId, Owner, RealmToken, World>
where
    ContextId: Copy,
    Owner: Copy,
    RealmToken: Copy,
    World: Copy,
{
    pub(crate) fn new(
        browsing_context_id: ContextId,
        owner: Owner,
        realm_token: RealmToken,
        world: World,
    ) -> Self {
        Self {
            browsing_context_id,
            owner,
            realm_token,
            world,
        }
    }

    pub(crate) const fn browsing_context_id(self) -> ContextId {
        self.browsing_context_id
    }

    pub(crate) const fn owner(self) -> Owner {
        self.owner
    }

    pub(crate) const fn realm_token(self) -> RealmToken {
        self.realm_token
    }

    pub(crate) const fn world(self) -> World {
        self.world
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RealmLifecycleState {
    Reserved,
    MaterializationQueued,
    Materialized,
    DetachedReachable,
    Disposed,
}

impl RealmLifecycleState {
    pub(crate) const fn belongs_to_current_local_window(self) -> bool {
        matches!(
            self,
            Self::Reserved | Self::MaterializationQueued | Self::Materialized
        )
    }
}

/// Currentness of an exact Document/LocalWindow/realm tuple.
///
/// Owner stores choose their own typed owner and realm identities; consumers
/// can share the stale/pending/current decision without depending on a frame or
/// auxiliary-Page adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DocumentRealmCurrentness<Owner, RealmId> {
    Current {
        owner: Owner,
        realm_id: RealmId,
    },
    StaleOwner,
    MissingRealm {
        owner: Owner,
    },
    PendingRealm {
        owner: Owner,
        realm_id: RealmId,
    },
    StaleRealm {
        owner: Owner,
        current_realm_id: RealmId,
    },
}

impl<Owner, RealmId> DocumentRealmCurrentness<Owner, RealmId> {
    /// Whether the exact Document/LocalWindow/realm identity is still current,
    /// independently of whether its runtime context has materialized.
    pub(crate) fn names_current_document_realm(self) -> bool {
        matches!(self, Self::Current { .. } | Self::PendingRealm { .. })
    }
}

/// One atomic current-Document owner transition for a browsing context.
///
/// The adapter may carry additional embedding metadata (for example an iframe
/// owner handle), but retirement consumers can always key browser-owned state
/// by this context identity and exact owner generation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DocumentOwnerTransition<ContextId, Owner> {
    browsing_context_id: ContextId,
    retired_owner: Option<Owner>,
    current_owner: Option<Owner>,
}

/// Exact browser-owned state that must be retired with an old Document.
///
/// The document token is generic so DOM-backed adapters can use a wrapper
/// handle while a top-level Page owner can use its native Document identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DocumentExternalStateRetirement<ContextId, Owner, DocumentToken> {
    browsing_context_id: ContextId,
    retired_owner: Owner,
    document_token: DocumentToken,
}

impl<ContextId, Owner, DocumentToken>
    DocumentExternalStateRetirement<ContextId, Owner, DocumentToken>
where
    ContextId: Copy,
    Owner: Copy,
    DocumentToken: Copy,
{
    pub(crate) fn new(
        browsing_context_id: ContextId,
        retired_owner: Owner,
        document_token: DocumentToken,
    ) -> Self {
        Self {
            browsing_context_id,
            retired_owner,
            document_token,
        }
    }

    pub(crate) const fn browsing_context_id(self) -> ContextId {
        self.browsing_context_id
    }

    pub(crate) const fn retired_owner(self) -> Owner {
        self.retired_owner
    }

    pub(crate) const fn document_token(self) -> DocumentToken {
        self.document_token
    }
}

impl<ContextId, Owner> DocumentOwnerTransition<ContextId, Owner>
where
    ContextId: Copy,
    Owner: Copy,
{
    pub(crate) fn new(
        browsing_context_id: ContextId,
        retired_owner: Option<Owner>,
        current_owner: Option<Owner>,
    ) -> Self {
        assert!(
            retired_owner.is_some() || current_owner.is_some(),
            "a document owner transition must install or retire an owner"
        );
        Self {
            browsing_context_id,
            retired_owner,
            current_owner,
        }
    }

    pub(crate) const fn browsing_context_id(self) -> ContextId {
        self.browsing_context_id
    }

    pub(crate) const fn retired_owner(self) -> Option<Owner> {
        self.retired_owner
    }

    pub(crate) const fn current_owner(self) -> Option<Owner> {
        self.current_owner
    }

    pub(crate) fn external_state_retirement<DocumentToken>(
        self,
        document_token: DocumentToken,
    ) -> Option<DocumentExternalStateRetirement<ContextId, Owner, DocumentToken>>
    where
        DocumentToken: Copy,
    {
        self.retired_owner.map(|retired_owner| {
            DocumentExternalStateRetirement::new(
                self.browsing_context_id,
                retired_owner,
                document_token,
            )
        })
    }
}

impl<RealmId> RealmMaterializationRequest<RealmId>
where
    RealmId: Copy,
{
    pub(crate) const fn realm_id(self) -> RealmId {
        match self {
            Self::NewlyQueued { realm_id }
            | Self::AlreadyQueued { realm_id }
            | Self::AlreadyMaterialized { realm_id } => realm_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browsing_context_identity_is_kind_qualified() {
        let nested = BrowsingContextId::nested(7);
        let auxiliary = BrowsingContextId::auxiliary_top_level(7);

        assert_ne!(nested, auxiliary);
        assert_eq!(nested.kind(), BrowsingContextKind::Nested);
        assert_eq!(nested.value(), 7);
        assert_eq!(
            BrowsingContextId::primary_top_level().kind(),
            BrowsingContextKind::PrimaryTopLevel
        );
    }

    #[test]
    fn local_window_transition_classifies_all_owner_changes() {
        let first = LocalWindowId(1);
        let second = LocalWindowId(2);

        assert_eq!(
            LocalWindowOwnerTransition::between(None, Some(first)),
            LocalWindowOwnerTransition::Installed { current: first }
        );
        assert_eq!(
            LocalWindowOwnerTransition::between(Some(first), Some(first)),
            LocalWindowOwnerTransition::Preserved { current: first }
        );
        assert_eq!(
            LocalWindowOwnerTransition::between(Some(first), Some(second)),
            LocalWindowOwnerTransition::Replaced {
                retired: first,
                current: second,
            }
        );
        assert_eq!(
            LocalWindowOwnerTransition::between(Some(second), None),
            LocalWindowOwnerTransition::Retired { retired: second }
        );
    }

    #[test]
    fn document_owner_transition_preserves_context_and_exact_generations() {
        let context_id = BrowsingContextId::nested(11);
        let transition = DocumentOwnerTransition::new(context_id, Some(3_u64), Some(4_u64));

        assert_eq!(transition.browsing_context_id(), context_id);
        assert_eq!(transition.retired_owner(), Some(3));
        assert_eq!(transition.current_owner(), Some(4));

        let retirement = transition
            .external_state_retirement(17_u64)
            .expect("transition with a retired owner must expose a retirement hook");
        assert_eq!(retirement.browsing_context_id(), context_id);
        assert_eq!(retirement.retired_owner(), 3);
        assert_eq!(retirement.document_token(), 17);
    }

    #[test]
    fn document_realm_currentness_treats_pending_materialization_as_current_identity() {
        let pending = DocumentRealmCurrentness::PendingRealm {
            owner: 3_u64,
            realm_id: 9_u64,
        };
        let stale = DocumentRealmCurrentness::<u64, u64>::StaleOwner;

        assert!(pending.names_current_document_realm());
        assert!(!stale.names_current_document_realm());
    }

    #[test]
    fn realm_projection_keeps_browser_identity_outside_embedding_adapter() {
        let context_id = BrowsingContextId::nested(13);
        let projection = RealmHostProjection::new(
            context_id,
            5_u64,
            19_u64,
            RealmWorldKind::Isolated {
                access_policy: RealmAccessPolicy::Universal,
            },
        );

        assert_eq!(projection.browsing_context_id(), context_id);
        assert_eq!(projection.owner(), 5);
        assert_eq!(projection.realm_token(), 19);
        assert_eq!(
            projection.world().access_policy(),
            RealmAccessPolicy::Universal
        );
        assert!(!projection.world().is_default());
    }

    #[test]
    fn access_origin_requires_matching_domain_mode_or_opaque_identity() {
        let tuple = BrowsingContextAccessOrigin::<u64>::from_serialized_origin(
            "https://example.test".to_owned(),
            None,
        )
        .expect("tuple origin");
        let domain_relaxed = BrowsingContextAccessOrigin::<u64>::from_serialized_origin(
            "https://sub.example.test".to_owned(),
            Some("example.test".to_owned()),
        )
        .expect("domain-relaxed tuple origin");

        assert!(tuple.can_access(&tuple));
        assert!(!tuple.can_access(&domain_relaxed));
        assert!(
            BrowsingContextAccessOrigin::opaque(7_u64)
                .can_access(&BrowsingContextAccessOrigin::opaque(7_u64))
        );
        assert!(
            !BrowsingContextAccessOrigin::opaque(7_u64)
                .can_access(&BrowsingContextAccessOrigin::opaque(8_u64))
        );
    }
}
