use super::*;
use crate::conn::{
    CommandWorkflowContext, ProtocolPageHostScope, ProtocolWorkflowDriver,
    TargetPageResidenceIdentity, TargetRootDocumentProtocolAttachmentIdentity,
};

/// Real preload work: either a target registration with channel listeners,
/// or the desired registry plus its original loaded Documents.
pub(crate) enum PreparedAutomationPreload {
    TargetAdd {
        projection: PreloadScriptProjection,
        prepared: PreparedPageAddPreload,
    },
    Registry {
        result: AutomationResult,
        operations: Vec<Result<PreparedPreloadRegistryOperation, DevToolsError>>,
    },
}

pub(crate) struct PreloadScriptProjection {
    protocol: FrontendProtocol,
    target: Option<DevToolsTargetId>,
}

impl PreloadScriptProjection {
    fn project(self, identifier: String) -> AutomationResult {
        let identifier = match (self.protocol, self.target) {
            (FrontendProtocol::WebDriverBidi, Some(target)) => {
                format!("{}:{identifier}", target.as_str())
            }
            _ => identifier,
        };
        AutomationResult::AddPreloadScript(DevToolsAddPreloadScriptResult {
            script_id: identifier.into(),
        })
    }
}

pub(crate) struct PreparedPreloadRegistryOperation {
    host: ProtocolPageHostScope,
    document: TargetRootDocumentProtocolAttachmentIdentity,
    mutation: PreloadMutation,
}

enum PreloadMutation {
    Append(DocumentStartScript),
    Remove(String),
}

#[derive(Clone, Copy)]
pub(crate) enum PreloadMutationKind {
    Append,
    Remove,
}

pub(crate) struct PreparedPreloadRuntimeMutation {
    page: TargetPageResidenceIdentity,
    kind: PreloadMutationKind,
    pending: PendingPageCommand,
}

impl PreparedPreloadRegistryOperation {
    fn capture(
        conn: &CdpConnection,
        owner: CommandOwnerScope,
        mutation: PreloadMutation,
    ) -> Result<Option<Self>, DevToolsError> {
        let Some(document) = conn.target_root_document_lifecycle_identity_for_owner(&owner) else {
            // The desired registry is already recorded for a future Document.
            return Ok(None);
        };
        let host = ProtocolPageHostScope::capture_for_operation(conn, &owner)
            .map_err(preload_internal_error)?;
        let document = conn
            .target_root_document_protocol_attachment_identity_for_owner(host.owner(), document)
            .ok_or_else(|| preload_internal_error(crate::conn::NoDocumentLoaded))?;
        Ok(Some(Self {
            host,
            document,
            mutation,
        }))
    }

    pub(crate) fn start(self, conn: &mut CdpConnection) -> Result<PreparedPreloadRuntimeMutation> {
        anyhow::ensure!(
            self.host.can_start_operation(conn)
                && conn
                    .target_root_document_protocol_attachment_identity_is_current(&self.document),
            crate::conn::NoDocumentLoaded,
        );
        let owner = self.host.owner();
        let page = conn
            .target_page_residence_identity_for_owner(owner)
            .ok_or(crate::conn::NoDocumentLoaded)?;
        let inspector = conn.target_renderer_runtime_inspector_session_id_for_owner(owner);
        let loaded = conn
            .loaded_page_mut_for_owner(owner)
            .ok_or(crate::conn::NoDocumentLoaded)?;
        let (kind, pending) = match self.mutation {
            PreloadMutation::Append(script) => (
                PreloadMutationKind::Append,
                loaded.start_add_document_start_script_runtime_activity(
                    inspector.as_deref(),
                    &script,
                    false,
                )?,
            ),
            PreloadMutation::Remove(key) => (
                PreloadMutationKind::Remove,
                loaded.start_remove_document_start_script_by_registry_key(&key)?,
            ),
        };
        Ok(PreparedPreloadRuntimeMutation {
            page,
            kind,
            pending,
        })
    }
}

impl PreparedPreloadRuntimeMutation {
    async fn run(
        self,
        driver: &mut ProtocolWorkflowDriver,
        context: &mut CommandWorkflowContext,
    ) -> Result<()> {
        let Self {
            page,
            kind,
            pending,
        } = self;
        let completed = pending.wait().await?;
        context.with_state(driver, move |state, _| {
            state.finish_preload_registry_mutation(page, kind, completed)
        })??;
        Ok(())
    }
}

impl PreparedAutomationPreload {
    pub(crate) fn prepare(
        conn: &mut CdpConnection,
        command: AutomationCommand,
    ) -> Result<Self, DevToolsError> {
        match command {
            AutomationCommand::AddPreloadScript(command)
                if !command.browser_context_ids.is_empty() =>
            {
                Self::prepare_context_add(conn, command)
            }
            AutomationCommand::RemovePreloadScript(command)
                if command.context.protocol == FrontendProtocol::WebDriverBidi
                    && command.context.target_id.is_none()
                    && split_bidi_preload_script_id(&command.script_id).is_none() =>
            {
                Self::prepare_context_remove(conn, command)
            }
            command => {
                let projection = match &command {
                    AutomationCommand::AddPreloadScript(command) => Some(PreloadScriptProjection {
                        protocol: command.context.protocol,
                        target: add_preload_command_target_id(command)?.cloned(),
                    }),
                    _ => None,
                };
                let (route, command) = devtools_preload_command_target_route(conn, command)?;
                let owner = CommandOwnerScope::for_route(route);
                match command {
                    AutomationCommand::AddPreloadScript(command) => {
                        let projection = projection.expect("an add command retains its projection");
                        if is_bidi_default_preload_command(&command) {
                            Self::prepare_default_add(conn, owner, command, projection)
                        } else {
                            ProtocolPageHostScope::capture_for_operation(conn, &owner)
                                .map_err(preload_internal_error)?;
                            Ok(Self::TargetAdd {
                                projection,
                                prepared: PreparedPageAddPreload::prepare(conn, owner, command)?,
                            })
                        }
                    }
                    AutomationCommand::RemovePreloadScript(command) => {
                        Self::prepare_target_remove(conn, owner, command)
                    }
                    _ => unreachable!("preload routing selects add or remove"),
                }
            }
        }
    }

    fn prepare_context_add(
        conn: &mut CdpConnection,
        command: DevToolsAddPreloadScriptCommand,
    ) -> Result<Self, DevToolsError> {
        let script = document_start_script_from_add_preload_command(&command)?;
        let contexts =
            resolve_bidi_preload_browser_context_ids(conn, &command.browser_context_ids)?;
        let first = contexts.first().ok_or_else(|| {
            DevToolsError::new(
                DevToolsErrorKind::InvalidArgument,
                PreloadCommandError::EmptyUserContexts,
            )
        })?;
        let identifier = conn
            .browser_context_by_id_mut(first)
            .ok_or_else(preload_no_target_error)?
            .reserve_default_document_start_script_id();
        for id in &contexts {
            conn.browser_context_by_id_mut(id)
                .ok_or_else(preload_no_target_error)?
                .record_default_document_start_script_with_identifier(identifier.clone(), &script);
        }
        let script = script.with_registry_key(
            BrowserContext::default_document_start_script_registry_key(&identifier),
        );
        let mut operations = Vec::new();
        for id in contexts {
            if let Some(owner) = active_context_owner(conn, &id)
                && let Some(operation) = PreparedPreloadRegistryOperation::capture(
                    conn,
                    owner,
                    PreloadMutation::Append(script.clone()),
                )
                .transpose()
            {
                operations.push(operation);
            }
        }
        Ok(Self::Registry {
            result: AutomationResult::AddPreloadScript(DevToolsAddPreloadScriptResult {
                script_id: identifier.into(),
            }),
            operations,
        })
    }

    fn prepare_default_add(
        conn: &mut CdpConnection,
        owner: CommandOwnerScope,
        command: DevToolsAddPreloadScriptCommand,
        projection: PreloadScriptProjection,
    ) -> Result<Self, DevToolsError> {
        let script = document_start_script_from_add_preload_command(&command)?;
        let (context_id, _) = conn
            .target_owner_identity_for_owner(&owner)
            .ok_or_else(|| {
                DevToolsError::new(
                    DevToolsErrorKind::NoSuchTarget,
                    PreloadCommandError::BrowserContextNotLoaded,
                )
            })?;
        let identifier = conn
            .browser_context_by_id_mut(&context_id)
            .ok_or_else(preload_no_target_error)?
            .record_default_document_start_script(&script);
        let script = script.with_registry_key(
            BrowserContext::default_document_start_script_registry_key(&identifier),
        );
        let operations =
            PreparedPreloadRegistryOperation::capture(conn, owner, PreloadMutation::Append(script))
                .transpose()
                .into_iter()
                .collect();
        Ok(Self::Registry {
            result: projection.project(identifier),
            operations,
        })
    }

    fn prepare_target_remove(
        conn: &mut CdpConnection,
        owner: CommandOwnerScope,
        command: DevToolsRemovePreloadScriptCommand,
    ) -> Result<Self, DevToolsError> {
        let protocol = command.context.protocol;
        let script_id = command.script_id.into_string();
        let identity = conn.target_owner_identity_for_owner(&owner);
        let fallback = identity.as_ref().map(|(_, target)| {
            BrowserContext::target_document_start_script_registry_key(target.as_deref(), &script_id)
        });
        let (mut removed, mut key) = conn
            .with_target_owner_state_for_owner_mut(&owner, |state| {
                remove_stored_document_start_script_registry_key(state, None, &script_id, fallback)
            })
            .ok_or_else(|| preload_missing_owner_error(conn))?;
        if !removed
            && protocol == FrontendProtocol::WebDriverBidi
            && let Some((id, _)) = identity
            && let Some(context) = conn.browser_context_by_id_mut(&id)
            && let Some(default) = context.remove_default_document_start_script(&script_id)
        {
            removed = true;
            key = Some(default);
        }
        if !removed && protocol == FrontendProtocol::WebDriverBidi {
            return Err(DevToolsError::new(
                DevToolsErrorKind::NoSuchScript,
                PreloadCommandError::NoSuchScript,
            ));
        }
        let operations = match key {
            Some(key) => {
                PreparedPreloadRegistryOperation::capture(conn, owner, PreloadMutation::Remove(key))
                    .transpose()
                    .into_iter()
                    .collect()
            }
            None => Vec::new(),
        };
        Ok(Self::Registry {
            result: AutomationResult::Empty,
            operations,
        })
    }

    fn prepare_context_remove(
        conn: &mut CdpConnection,
        command: DevToolsRemovePreloadScriptCommand,
    ) -> Result<Self, DevToolsError> {
        let script_id = command.script_id.as_str();
        let contexts = conn
            .browser_contexts()
            .filter(|context| context.has_default_document_start_script(script_id))
            .map(|context| context.id.clone())
            .collect::<Vec<_>>();
        if contexts.is_empty() {
            return Err(DevToolsError::new(
                DevToolsErrorKind::NoSuchScript,
                PreloadCommandError::NoSuchScript,
            ));
        }
        let removed = contexts
            .into_iter()
            .filter_map(|id| {
                let key = conn
                    .browser_context_by_id_mut(&id)?
                    .remove_default_document_start_script(script_id)?;
                Some((id, key))
            })
            .collect::<Vec<_>>();
        let mut operations = Vec::new();
        for (id, key) in removed {
            if let Some(owner) = active_context_owner(conn, &id)
                && let Some(operation) = PreparedPreloadRegistryOperation::capture(
                    conn,
                    owner,
                    PreloadMutation::Remove(key),
                )
                .transpose()
            {
                operations.push(operation);
            }
        }
        Ok(Self::Registry {
            result: AutomationResult::Empty,
            operations,
        })
    }

    pub(crate) async fn run(
        self,
        driver: &mut ProtocolWorkflowDriver,
        context: &mut CommandWorkflowContext,
    ) -> Result<AutomationResult, DevToolsError> {
        match self {
            Self::TargetAdd {
                projection,
                prepared,
            } => {
                let identifier = prepared.identifier.clone();
                let plan = prepared
                    .run(driver, context)
                    .await
                    .map_err(preload_internal_error)?;
                context.retain_output(plan.into_composite_command_prefix());
                Ok(projection.project(identifier))
            }
            Self::Registry { result, operations } => {
                for operation in operations {
                    // Preserve short-circuit order even if a later frozen
                    // target was already unavailable at admission.
                    let operation = operation?;
                    let pending = context
                        .with_state(driver, move |state, _| {
                            state.start_preload_registry_mutation(operation)
                        })
                        .map_err(preload_internal_error)?
                        .map_err(preload_internal_error)?;
                    pending
                        .run(driver, context)
                        .await
                        .map_err(preload_internal_error)?;
                }
                Ok(result)
            }
        }
    }
}

fn active_context_owner(conn: &CdpConnection, id: &str) -> Option<CommandOwnerScope> {
    Some(CommandOwnerScope::for_route(CdpSessionRoute::PageTarget {
        browser_context_id: id.to_owned(),
        target_id: conn
            .browser_context_by_id(id)?
            .active_target_id()?
            .to_owned(),
        session_key: DevToolsSessionKey::Primary,
    }))
}

fn preload_no_target_error() -> DevToolsError {
    DevToolsError::new(
        DevToolsErrorKind::NoSuchTarget,
        PreloadCommandError::NoSuchTarget,
    )
}

fn preload_internal_error(error: impl std::fmt::Display) -> DevToolsError {
    DevToolsError::new(DevToolsErrorKind::Internal, error)
}
