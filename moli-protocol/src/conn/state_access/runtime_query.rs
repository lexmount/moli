use anyhow::Result;
use moli_core::page::{
    CompletedPageCommand, DocumentNodeObjectSnapshot, DocumentNodeRuntimeObjectResolution, Page,
    RendererDomBidiNodeBindingResolution,
};
use serde_json::Value;

use super::ProtocolWorkflowState;
use crate::conn::{CommandOwnerScope, PreparedRuntimePageQuery};

#[derive(Debug, thiserror::Error)]
#[error("missing execution context")]
struct RuntimeNodeContextUnavailable;

impl ProtocolWorkflowState<'_> {
    pub(crate) fn with_runtime_scope<T>(
        self,
        scope: Option<&crate::conn::RuntimePageScope>,
        operation: impl FnOnce(Self) -> T,
    ) -> Result<T> {
        if let Some(scope) = scope {
            anyhow::ensure!(
                scope.can_start_operation(self.connection),
                crate::conn::NoDocumentLoaded
            );
        }
        Ok(operation(self))
    }

    pub(crate) fn runtime_scope_accepts_work(&self, scope: &crate::conn::RuntimePageScope) -> bool {
        scope.can_start_operation(self.connection)
    }

    pub(crate) fn capture_runtime_scope(
        &self,
        owner: &CommandOwnerScope,
    ) -> Result<crate::conn::RuntimePageScope> {
        crate::conn::RuntimePageScope::capture(self.connection, owner)
    }
    pub(crate) fn finish_runtime_automation_output(
        self,
        command: &crate::automation::AutomationContext,
        scope: Option<&crate::conn::RuntimePageScope>,
        context: &mut crate::conn::CommandDispatchContext,
    ) {
        let host = scope.map(crate::conn::RuntimePageScope::host_scope);
        self.finish_automation_output(command, host.as_ref(), context);
    }
    pub(crate) fn runtime_dom_query_owner(
        &self,
        context: &crate::automation::AutomationContext,
    ) -> Option<CommandOwnerScope> {
        self.connection
            .command_owner_scope_for_devtools_context(context)
    }

    pub(crate) fn prepare_runtime_dom_query(
        self,
        owner: CommandOwnerScope,
        command: crate::automation::AutomationCommand,
    ) -> Result<crate::conn::PreparedRuntimeDomQuery, crate::automation::DevToolsError> {
        crate::conn::PreparedRuntimeDomQuery::prepare(self.connection, owner, command)
    }

    pub(crate) fn finish_runtime_dom_query(
        self,
        query: crate::conn::PreparedRuntimeDomQuery,
        completed: crate::domains::dom::CompletedDomCommandDispatch,
    ) -> Result<
        (
            crate::conn::PreparedRuntimeDomQuery,
            crate::domains::dom::DevToolsDomCommandTaskStep,
        ),
        crate::automation::DevToolsError,
    > {
        query.finish(self.connection, completed)
    }
    pub(crate) fn runtime_bidi_channel_owner(
        self,
        route: crate::conn::CdpSessionRoute,
    ) -> Option<crate::conn::BidiChannelPageOwner> {
        crate::conn::BidiChannelPageOwner::capture_for_owner(
            self.connection,
            CommandOwnerScope::for_route(route),
        )
    }

    pub(crate) fn release_runtime_object_group(
        self,
        mut lease: crate::conn::BidiChannelObjectGroupLease,
    ) {
        lease.release(self.connection);
    }

    pub(crate) fn allocate_runtime_object_group(
        self,
        context: &crate::automation::AutomationContext,
    ) -> Result<crate::conn::BidiChannelObjectGroupLease, crate::automation::DevToolsError> {
        let owner = self
            .connection
            .command_owner_scope_for_devtools_context(context)
            .and_then(|owner| {
                crate::conn::BidiChannelPageOwner::capture_for_owner(self.connection, owner)
            })
            .ok_or_else(|| {
                crate::automation::DevToolsError::new(
                    crate::automation::DevToolsErrorKind::NoSuchTarget,
                    crate::conn::NoDocumentLoaded,
                )
            })?;
        crate::conn::BidiChannelObjectGroupLease::allocate(self.connection, owner).ok_or_else(
            || {
                crate::automation::DevToolsError::new(
                    crate::automation::DevToolsErrorKind::NoSuchTarget,
                    crate::conn::NoDocumentLoaded,
                )
            },
        )
    }

    pub(crate) fn validate_runtime_automation_handles(
        self,
        owner: &CommandOwnerScope,
        command: &crate::automation::AutomationCommand,
        realm: Option<&crate::automation::DevToolsRealmId>,
    ) -> Result<(), crate::automation::DevToolsError> {
        crate::domains::runtime::validate_protocol_neutral_runtime_handle_realms(
            self.connection,
            owner,
            command,
            realm,
        )
    }

    pub(crate) fn register_runtime_script_result(
        self,
        owner: &CommandOwnerScope,
        result: &crate::automation::AutomationResult,
        realm: Option<&crate::automation::DevToolsRealmId>,
    ) {
        crate::domains::runtime::register_devtools_script_result_remote_object(
            self.connection,
            owner,
            result,
        );
        crate::domains::runtime::register_devtools_script_result_remote_object_realm(
            self.connection,
            owner,
            result,
            realm,
        );
    }

    pub(crate) fn register_runtime_node_reference(
        self,
        owner: &CommandOwnerScope,
        object_id: String,
        realm: Option<crate::automation::DevToolsRealmId>,
        shared_id: Option<String>,
    ) {
        let Some(realm) = realm else {
            return;
        };
        self.connection
            .register_runtime_remote_object_ids_for_owner_with_realm(
                owner,
                vec![object_id.clone()],
                realm.as_str(),
            );
        if let Some(shared_id) = shared_id
            && shared_id != object_id
            && !crate::automation::is_webdriver_bidi_node_shared_id(&object_id)
        {
            self.connection
                .register_runtime_remote_object_alias_for_owner_with_realm(
                    owner,
                    shared_id,
                    object_id,
                    realm.as_str(),
                );
        }
    }
    pub(crate) fn runtime_route_candidates(
        &self,
        target_id: &str,
    ) -> (
        Option<crate::conn::CdpSessionRoute>,
        Option<crate::conn::CdpSessionRoute>,
    ) {
        (
            self.connection
                .target_session_route_for_target_id(target_id),
            self.connection
                .target_session_route_for_child_frame_id(target_id),
        )
    }

    pub(crate) fn register_remapped_runtime_node(
        self,
        owner: &CommandOwnerScope,
        remote: Value,
        object_id: String,
        realm: Option<crate::automation::DevToolsRealmId>,
    ) {
        if let Some(realm) = realm {
            self.connection
                .register_runtime_remote_object_ids_for_owner_with_realm(
                    owner,
                    vec![object_id],
                    realm.as_str(),
                );
        } else {
            self.connection
                .register_runtime_remote_object_ids_from_value_for_owner(owner, &remote);
        }
    }

    pub(crate) fn runtime_target_route(
        &self,
        target_id: &str,
        include_child: bool,
    ) -> Option<crate::conn::CdpSessionRoute> {
        self.connection
            .target_session_route_for_target_id(target_id)
            .or_else(|| {
                include_child
                    .then(|| {
                        self.connection
                            .target_session_route_for_child_frame_id(target_id)
                    })
                    .flatten()
            })
    }

    pub(crate) fn runtime_routes(
        &self,
        target_id: Option<&crate::automation::DevToolsTargetId>,
    ) -> Result<Vec<crate::conn::CdpSessionRoute>, crate::automation::DevToolsError> {
        crate::domains::runtime::devtools_get_realms_routes(self.connection, target_id)
    }

    pub(crate) fn runtime_worker_realms(
        &self,
        route: &crate::conn::CdpSessionRoute,
    ) -> Result<
        Option<Vec<crate::automation::RuntimeExecutionContextEvent>>,
        crate::automation::DevToolsError,
    > {
        crate::domains::runtime::runtime_worker_realms(self.connection, route)
    }

    pub(crate) fn runtime_object_matches_realm(
        &self,
        owner: &CommandOwnerScope,
        object_id: &str,
        realm: Option<&crate::automation::DevToolsRealmId>,
    ) -> bool {
        self.connection
            .runtime_remote_object_id_known_for_owner(owner, object_id)
            && realm.is_none_or(|realm| {
                self.connection
                    .runtime_remote_object_realm_for_owner(owner, object_id)
                    .is_none_or(|actual| actual == realm.as_str())
            })
    }

    pub(crate) fn unregister_runtime_objects(
        self,
        owner: &CommandOwnerScope,
        objects: Vec<String>,
    ) {
        self.connection
            .unregister_runtime_remote_object_ids_for_owner(owner, &objects);
    }

    pub(crate) fn finish_runtime_page_query<T: 'static>(
        self,
        query: PreparedRuntimePageQuery<T>,
        completed: CompletedPageCommand,
    ) -> Result<T> {
        query.finish(self.connection, completed)
    }

    pub(crate) fn prepare_runtime_default_context_query(
        self,
        owner: &CommandOwnerScope,
    ) -> Result<PreparedRuntimePageQuery<Option<i64>>> {
        PreparedRuntimePageQuery::prepare_resident(
            self.connection,
            owner,
            "runtime default execution context lookup failed",
            |page| page.start_default_or_initial_execution_context_id(),
            Page::finish_default_or_initial_execution_context_id,
        )
    }

    pub(crate) fn prepare_runtime_child_context_query(
        self,
        owner: &CommandOwnerScope,
        frame_id: String,
    ) -> Result<PreparedRuntimePageQuery<Option<i64>>> {
        PreparedRuntimePageQuery::prepare(
            self.connection,
            owner,
            "runtime child default context lookup failed",
            move |page| page.start_child_default_execution_context_id_for_frame_id(&frame_id),
            Page::finish_child_default_execution_context_id_for_frame_id,
        )
    }

    pub(crate) fn prepare_runtime_world_query(
        self,
        owner: &CommandOwnerScope,
        frame_id: Option<String>,
        world_name: String,
    ) -> Result<PreparedRuntimePageQuery<i64>> {
        let target_id = self
            .connection
            .target_owner_identity_for_owner(owner)
            .and_then(|(_, target)| target);
        let frame_id = frame_id.filter(|frame| target_id.as_deref() != Some(frame.as_str()));
        PreparedRuntimePageQuery::prepare(
            self.connection,
            owner,
            "runtime isolated world creation failed",
            move |page| page.start_create_isolated_world(&world_name, false, frame_id.as_deref()),
            Page::finish_create_isolated_world,
        )
    }

    pub(crate) fn prepare_runtime_bidi_binding_query(
        self,
        owner: &CommandOwnerScope,
        shared_id: String,
    ) -> Result<PreparedRuntimePageQuery<RendererDomBidiNodeBindingResolution>> {
        let session = self
            .connection
            .target_renderer_runtime_inspector_session_id_for_owner(owner);
        PreparedRuntimePageQuery::prepare(
            self.connection,
            owner,
            "resolve BiDi node binding failed",
            move |page| page.start_document_bidi_node_binding(session, shared_id),
            Page::finish_document_bidi_node_binding,
        )
    }

    pub(crate) fn prepare_runtime_bidi_binding_registration(
        self,
        owner: &CommandOwnerScope,
        shared_id: String,
        backend_node_id: u32,
    ) -> Result<PreparedRuntimePageQuery<()>> {
        let session = self
            .connection
            .target_renderer_runtime_inspector_session_id_for_owner(owner);
        PreparedRuntimePageQuery::prepare(
            self.connection,
            owner,
            "register BiDi node binding failed",
            move |page| {
                page.start_register_document_bidi_node_binding(session, shared_id, backend_node_id)
            },
            Page::finish_register_document_bidi_node_binding,
        )
    }

    pub(crate) fn prepare_runtime_backend_node_query(
        self,
        owner: &CommandOwnerScope,
        backend_node_id: u32,
        depth: i32,
        pierce: bool,
    ) -> Result<PreparedRuntimePageQuery<Option<DocumentNodeObjectSnapshot>>> {
        PreparedRuntimePageQuery::prepare(
            self.connection,
            owner,
            "resolve backend node snapshot failed",
            move |page| {
                page.start_document_node_snapshot_for_backend_node_id(
                    backend_node_id,
                    depth,
                    pierce,
                )
            },
            Page::finish_document_node_snapshot_for_backend_node_id,
        )
    }

    pub(crate) fn prepare_runtime_object_node_query(
        self,
        owner: &CommandOwnerScope,
        object_id: String,
        depth: i32,
        pierce: bool,
    ) -> Result<PreparedRuntimePageQuery<Option<DocumentNodeObjectSnapshot>>> {
        let session = self
            .connection
            .target_renderer_runtime_inspector_session_id_for_owner(owner);
        let whitespace =
            crate::domains::dom::dom_agent_includes_whitespace_for_owner(self.connection, owner);
        PreparedRuntimePageQuery::prepare(
            self.connection,
            owner,
            "resolve runtime node snapshot failed",
            move |page| {
                page.start_document_node_snapshot_for_object_id_in_inspector_session(
                    session, whitespace, &object_id, depth, pierce,
                )
            },
            Page::finish_document_node_snapshot_for_object_id,
        )
    }

    pub(crate) fn prepare_runtime_resolve_node_query(
        self,
        owner: &CommandOwnerScope,
        backend_node_id: u32,
        context_id: Option<i64>,
        object_group: Option<String>,
    ) -> Result<PreparedRuntimePageQuery<Option<Value>>> {
        let session = self
            .connection
            .target_renderer_runtime_inspector_session_id_for_owner(owner);
        PreparedRuntimePageQuery::prepare(
            self.connection,
            owner,
            "resolve runtime object for backend node failed",
            move |page| {
                page.start_resolve_runtime_object_for_backend_node_id_in_inspector_session(
                    session,
                    backend_node_id,
                    context_id,
                    object_group.as_deref(),
                )
            },
            |page, completed| match page
                .finish_resolve_runtime_object_for_backend_node_id(completed)?
            {
                DocumentNodeRuntimeObjectResolution::Found(object) => {
                    Ok(Some(object.into_protocol_value()))
                }
                DocumentNodeRuntimeObjectResolution::MissingNode => Ok(None),
                DocumentNodeRuntimeObjectResolution::MissingContext => {
                    Err(RuntimeNodeContextUnavailable.into())
                }
            },
        )
    }
}
