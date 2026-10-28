use anyhow::Result;

use crate::conn::{
    BackgroundProtocolEvent, CdpConnection, CommandOwnerScope, ProtocolWorkflowDriver,
    SessionDisposalPlan, SessionDisposalTarget,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DevToolsSessionDomainHandler {
    Tracing,
    Browser,
    Target,
    Fetch,
    Runtime,
    Page,
    Emulation,
    Network,
    PrimaryPageTargetState,
}

const CONNECTION_HANDLERS: &[DevToolsSessionDomainHandler] = &[
    DevToolsSessionDomainHandler::Tracing,
    DevToolsSessionDomainHandler::Browser,
    DevToolsSessionDomainHandler::Target,
];

const PAGE_HANDLERS: &[DevToolsSessionDomainHandler] = &[
    DevToolsSessionDomainHandler::Tracing,
    DevToolsSessionDomainHandler::Browser,
    DevToolsSessionDomainHandler::Target,
    DevToolsSessionDomainHandler::Fetch,
    DevToolsSessionDomainHandler::Runtime,
    DevToolsSessionDomainHandler::Page,
    DevToolsSessionDomainHandler::Emulation,
    DevToolsSessionDomainHandler::Network,
    DevToolsSessionDomainHandler::PrimaryPageTargetState,
];

const WORKER_HANDLERS: &[DevToolsSessionDomainHandler] = &[
    DevToolsSessionDomainHandler::Tracing,
    DevToolsSessionDomainHandler::Browser,
    DevToolsSessionDomainHandler::Target,
    DevToolsSessionDomainHandler::Runtime,
];

/// Browser-side handlers installed for one DevTools session.
///
/// Target teardown iterates this collection without knowing any domain's
/// state or disable operation. That mirrors Chromium's DevToolsSession, where
/// handlers own Disable() and the session lifecycle only invokes them before
/// renderer Inspector detachment.
struct DevToolsSessionHandlers {
    handlers: &'static [DevToolsSessionDomainHandler],
}

impl DevToolsSessionHandlers {
    fn for_target(target: &SessionDisposalTarget) -> Self {
        let handlers = match target {
            SessionDisposalTarget::PageTarget { .. } => PAGE_HANDLERS,
            SessionDisposalTarget::SharedWorkerTarget { .. }
            | SessionDisposalTarget::DedicatedWorkerTarget { .. }
            | SessionDisposalTarget::ServiceWorkerTarget { .. } => WORKER_HANDLERS,
            SessionDisposalTarget::Browser | SessionDisposalTarget::TabTarget { .. } => {
                CONNECTION_HANDLERS
            }
        };
        Self { handlers }
    }
}

pub(crate) struct DevToolsSessionHandlerDisposal {
    first_error: Option<anyhow::Error>,
    renderer_output_predecessor: Option<moli_core::RendererOutputFence>,
}

impl DevToolsSessionHandlerDisposal {
    pub(crate) fn into_parts(
        self,
    ) -> (
        Option<anyhow::Error>,
        Option<moli_core::RendererOutputFence>,
    ) {
        (self.first_error, self.renderer_output_predecessor)
    }
}

/// Invokes every installed browser-side handler while the session route is
/// still authoritative. One failure never prevents the remaining handlers
/// from receiving their disposal callback.
pub(crate) struct PreparedSessionHandlers {
    identity: crate::domains::target::session_disposal::SessionDisposalIdentity,
    handlers: Vec<(DevToolsSessionDomainHandler, PreparedSessionHandlerDisposal)>,
}

impl PreparedSessionHandlers {
    #[cfg(test)]
    pub(crate) fn prepare(conn: &mut CdpConnection, plan: &SessionDisposalPlan) -> Self {
        let identity = crate::domains::target::session_disposal::SessionDisposalIdentity::capture(
            conn,
            plan.clone(),
        );
        Self::prepare_for_identity(conn, identity)
    }

    pub(crate) fn prepare_for_identity(
        conn: &mut CdpConnection,
        identity: crate::domains::target::session_disposal::SessionDisposalIdentity,
    ) -> Self {
        let plan = identity.plan();
        let handlers = DevToolsSessionHandlers::for_target(plan.target())
            .handlers
            .iter()
            .map(|&handler| {
                (
                    handler,
                    PreparedSessionHandlerDisposal::prepare(conn, handler, plan),
                )
            })
            .collect();
        Self { identity, handlers }
    }

    pub(crate) fn identity(
        &self,
    ) -> &crate::domains::target::session_disposal::SessionDisposalIdentity {
        &self.identity
    }

    pub(crate) async fn run(
        self,
        driver: &mut ProtocolWorkflowDriver,
        background_events: &mut Vec<BackgroundProtocolEvent>,
        protocol_events: &mut Vec<BackgroundProtocolEvent>,
    ) -> DevToolsSessionHandlerDisposal {
        let mut disposal = DevToolsSessionHandlerDisposal {
            first_error: None,
            renderer_output_predecessor: None,
        };
        let plan = self.identity.plan();
        for (handler, prepared) in self.handlers {
            let result = prepared
                .run(driver, background_events, protocol_events, &self.identity)
                .await;
            match result {
                Ok(update) => {
                    // Policy/script ACKs may refer to an earlier observation from
                    // another Document; they do not publish a command response.
                    // Fetch cleanup owns concrete replay/failure output, so keep
                    // its dependency at the original disposal publication boundary.
                    if handler == DevToolsSessionDomainHandler::Fetch
                        && let Some(predecessor) = update.predecessor
                    {
                        predecessor
                            .merge_into_same_stream_tail(&mut disposal.renderer_output_predecessor);
                    }
                    if let Some(error) = update.first_error {
                        disposal.record_error(handler, plan, error);
                    }
                }
                Err(error) => disposal.record_error(handler, plan, error),
            }
        }
        disposal
    }
}

#[cfg(test)]
async fn dispose_live_handlers_using(
    driver: &mut ProtocolWorkflowDriver,
    background_events: &mut Vec<BackgroundProtocolEvent>,
    protocol_events: &mut Vec<BackgroundProtocolEvent>,
    plan: &SessionDisposalPlan,
) -> DevToolsSessionHandlerDisposal {
    let preparation_plan = plan.clone();
    driver
        .with_state(move |state| state.prepare_session_handlers(&preparation_plan))
        .unwrap()
        .run(driver, background_events, protocol_events)
        .await
}

impl DevToolsSessionHandlerDisposal {
    fn record_error(
        &mut self,
        handler: DevToolsSessionDomainHandler,
        plan: &SessionDisposalPlan,
        error: anyhow::Error,
    ) {
        tracing::warn!(
            session_id = plan.session_id(),
            domain = handler.name(),
            %error,
            "failed to dispose DevTools session domain"
        );
        self.first_error.get_or_insert_with(|| {
            error.context(format!("failed to dispose {} domain", handler.name()))
        });
    }
}

pub(crate) fn prepare_closed_handlers(
    conn: &mut CdpConnection,
    session_id: &str,
) -> Option<super::tracing::PendingTraceCancel> {
    let pending = conn.start_cancel_tracing_for_session_owner(Some(session_id));
    super::browser::dispose_session_handler(conn, session_id);
    super::target::dispose_session_handler(conn, session_id);
    pending
}

pub(crate) fn dispose_closed_handlers_sync(conn: &mut CdpConnection, session_id: &str) {
    conn.cancel_tracing_for_session_owner(Some(session_id));
    super::browser::dispose_session_handler(conn, session_id);
    super::target::dispose_session_handler(conn, session_id);
}

pub(crate) struct PreparedClosedTargetSessions {
    tracing: Vec<super::tracing::PendingTraceCancel>,
    completion: CompletedClosedTargetSessions,
}

#[derive(Clone)]
pub(crate) struct CompletedClosedTargetSessions {
    plan: crate::conn::TargetClosureCleanupPlan,
    bindings: Vec<(String, Option<crate::conn::SessionDisposalWait>)>,
    parent_session_id: Option<String>,
}

impl PreparedClosedTargetSessions {
    pub(crate) fn prepare(
        conn: &mut CdpConnection,
        plan: crate::conn::TargetClosureCleanupPlan,
        parent_session_id: Option<String>,
    ) -> Self {
        let mut tracing = Vec::new();
        let mut bindings = Vec::new();
        for session_id in plan.session_ids() {
            let binding = conn.session_route(Some(session_id)).and_then(|route| {
                let disposal = SessionDisposalPlan::for_session_route(session_id, &route)?;
                conn.mark_session_closing(&disposal);
                tracing.extend(prepare_closed_handlers(conn, session_id));
                conn.session_disposal_wait(session_id)
            });
            // Legacy automation may own target-side session metadata without
            // a registered CDP binding. Keep its typed detach publication;
            // completion still must not claim a subsequently registered SID.
            bindings.push((session_id.to_owned(), binding));
        }
        Self {
            tracing,
            completion: CompletedClosedTargetSessions {
                plan,
                bindings,
                parent_session_id,
            },
        }
    }

    pub(crate) async fn wait(self) -> CompletedClosedTargetSessions {
        for pending in self.tracing {
            pending.wait().await;
        }
        self.completion
    }
}

impl CompletedClosedTargetSessions {
    pub(crate) fn open_session_ids<'a>(
        &'a self,
        conn: &'a CdpConnection,
    ) -> impl Iterator<Item = &'a str> {
        self.bindings
            .iter()
            .filter(move |(session, binding)| match binding {
                Some(binding) => !binding.is_closed(),
                None => !conn.has_attached_devtools_session(session),
            })
            .map(|(session, _)| session.as_str())
    }

    pub(crate) fn finish(self, conn: &mut CdpConnection) -> crate::conn::TargetEventPlan {
        // Another admitted detachment may finish while native close waits.
        // The original binding's watch remains closed even if its wire id is
        // reused; a late completion must neither detach that successor nor
        // publish the old detachment twice.
        let plan = crate::conn::TargetClosureCleanupPlan::new(
            self.plan.target_id(),
            self.plan.reason(),
            self.open_session_ids(conn).map(str::to_owned),
        );
        conn.commit_target_closure_session_detachment_events(
            plan,
            self.parent_session_id.as_deref(),
        )
    }
}

impl DevToolsSessionDomainHandler {
    fn name(self) -> &'static str {
        match self {
            Self::Tracing => "Tracing",
            Self::Browser => "Browser",
            Self::Target => "Target",
            Self::Fetch => "Fetch",
            Self::Runtime => "Runtime",
            Self::Page => "Page",
            Self::Emulation => "Emulation",
            Self::Network => "Network",
            Self::PrimaryPageTargetState => "primary Page target",
        }
    }
}

/// A handler's admitted native work. There is one ordinary await sequence per
/// handler; native completion values never carry a connection borrow.
pub(crate) enum PreparedSessionHandlerDisposal {
    Complete,
    Tracing(Option<super::tracing::PendingTraceCancel>),
    Fetch(Option<Box<super::fetch::PreparedFetchCleanup>>),
    Runtime(Option<crate::conn::SessionWorkerCleanup>),
    Scripts(crate::conn::SessionScriptCleanup),
    PageUpdates {
        owner: CommandOwnerScope,
        updates: crate::conn::SessionPageUpdates,
    },
}

impl PreparedSessionHandlerDisposal {
    pub(crate) fn prepare(
        conn: &mut CdpConnection,
        handler: DevToolsSessionDomainHandler,
        plan: &SessionDisposalPlan,
    ) -> Self {
        let session_id = plan.session_id();
        match handler {
            DevToolsSessionDomainHandler::Tracing => {
                Self::Tracing(conn.start_cancel_tracing_for_session_owner(Some(session_id)))
            }
            DevToolsSessionDomainHandler::Browser => {
                super::browser::dispose_session_handler(conn, session_id);
                Self::Complete
            }
            DevToolsSessionDomainHandler::Target => {
                super::target::dispose_session_handler(conn, session_id);
                Self::Complete
            }
            DevToolsSessionDomainHandler::Fetch => Self::Fetch(
                super::fetch::prepare_owner_disposal(conn, Some(session_id)).map(Box::new),
            ),
            DevToolsSessionDomainHandler::Runtime => Self::Runtime(
                (!matches!(plan.target(), SessionDisposalTarget::PageTarget { .. }))
                    .then(|| conn.prepare_session_worker_cleanup(session_id)),
            ),
            DevToolsSessionDomainHandler::Page => {
                Self::Scripts(conn.prepare_session_script_cleanup(session_id))
            }
            DevToolsSessionDomainHandler::Emulation => Self::PageUpdates {
                owner: CommandOwnerScope::capture(conn, Some(session_id)),
                updates: super::emulation::prepare_page_session_disposal(conn, session_id),
            },
            DevToolsSessionDomainHandler::Network => Self::PageUpdates {
                owner: CommandOwnerScope::capture(conn, Some(session_id)),
                updates: conn.start_clear_devtools_network_session_policy(session_id),
            },
            DevToolsSessionDomainHandler::PrimaryPageTargetState => {
                let owner = CommandOwnerScope::capture(conn, Some(session_id));
                let mut updates = crate::conn::SessionPageUpdates::default();
                if let SessionDisposalTarget::PageTarget {
                    browser_context_id,
                    target_id,
                    session_key: moli_page_types::DevToolsSessionKey::Primary,
                } = plan.target()
                {
                    let found = if let Some(context) =
                        conn.browser_context_by_id_mut(browser_context_id)
                    {
                        let (found, prepared) = context
                            .start_reset_primary_page_session_target_state(target_id, session_id);
                        updates = prepared;
                        found
                    } else {
                        false
                    };
                    if !found {
                        updates.record_error(
                            anyhow::Error::new(crate::conn::NoDocumentLoaded)
                                .context("primary Page target disappeared during session disposal"),
                        );
                    }
                }
                Self::PageUpdates { owner, updates }
            }
        }
    }

    async fn run(
        self,
        driver: &mut ProtocolWorkflowDriver,
        background_events: &mut Vec<BackgroundProtocolEvent>,
        protocol_events: &mut Vec<BackgroundProtocolEvent>,
        identity: &crate::domains::target::session_disposal::SessionDisposalIdentity,
    ) -> Result<crate::conn::SessionPageUpdateResult> {
        match self {
            Self::Complete => Ok(Default::default()),
            Self::Tracing(pending) => {
                if let Some(pending) = pending {
                    pending.wait().await;
                }
                Ok(Default::default())
            }
            Self::Fetch(pending) => match pending {
                Some(pending) => Ok(pending
                    .run_using(driver)
                    .await
                    .into_disposal_output(background_events)),
                None => Ok(Default::default()),
            },
            Self::Runtime(pending) => {
                let completed = match pending {
                    Some(pending) => Some(pending.wait().await),
                    None => None,
                };
                let identity = identity.clone();
                let (background, protocol) = driver.with_state(move |state| {
                    state.finish_session_runtime_cleanup(&identity, completed)
                })?;
                background_events.extend(background);
                protocol_events.extend(protocol);
                Ok(Default::default())
            }
            Self::Scripts(pending) => {
                let completed = pending.wait().await;
                let identity = identity.clone();
                driver.with_state(move |state| {
                    state.finish_session_script_cleanup(&identity, completed)
                })
            }
            Self::PageUpdates { owner, updates } => {
                let completed = updates.wait().await;
                let identity = identity.clone();
                driver.with_state(move |state| {
                    state.finish_session_page_updates(&identity, &owner, completed)
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn::{BrowserContext, CommandDispatchContext, ParsedCdpCommand};
    use crate::testing::TestContext;
    use moli_page_types::DevToolsSessionKey;
    use serde_json::json;

    async fn fixture() -> (TestContext, SessionDisposalPlan) {
        let mut context = TestContext::new();
        let mut browser = BrowserContext::new("BID-handler-disposal".into());
        browser.set_active_target_id("TID-handler-disposal");
        browser.attach_active_session("SID-handler-disposal");
        context.with_fixture(|conn| conn.install_browser_context_fixture_for_test(browser));
        context
            .install_navigation_fixture_for_session_owner(
                "about:blank",
                Some("SID-handler-disposal"),
            )
            .await;
        for (id, method, params) in [
            (
                1,
                "Page.addScriptToEvaluateOnNewDocument",
                json!({"source": "globalThis.sessionPreload = true;"}),
            ),
            (2, "Network.enable", json!({})),
            (
                3,
                "Emulation.setScriptExecutionDisabled",
                json!({"value": true}),
            ),
        ] {
            context.process_async(json!({
                "id": id, "sessionId": "SID-handler-disposal", "method": method, "params": params,
            })).await;
            assert!(
                context
                    .sent
                    .iter()
                    .any(|message| message["id"] == id && message.get("result").is_some())
            );
        }
        let plan = SessionDisposalPlan::for_session_route(
            "SID-handler-disposal",
            &context.with_fixture(|conn| conn.session_route(Some("SID-handler-disposal")).unwrap()),
        )
        .unwrap();
        (context, plan)
    }

    #[tokio::test]
    async fn owned_handler_chain_releases_state_at_every_wait_and_keeps_the_binding() {
        use futures_util::FutureExt as _;
        struct ReleaseRenderer(Option<std::sync::mpsc::Sender<()>>);
        impl Drop for ReleaseRenderer {
            fn drop(&mut self) {
                if let Some(release) = self.0.take() {
                    let _ = release.send(());
                }
            }
        }
        let (mut context, plan) = fixture().await;
        let (entered, waiting) = tokio::sync::oneshot::channel();
        let (release, held) = std::sync::mpsc::channel();
        let (gate, guard) = context.with_fixture(|conn| {
            conn.loaded_page_mut_for_protocol_access(Some("SID-handler-disposal"))
                .unwrap()
                .start_cdp_call(
                    90000001,
                    None,
                    moli_core::RendererNativeOperation::then(
                        moli_core::RendererPageCommand::ChildFrameTreeSnapshot,
                        move |_| {
                            let _ = entered.send(());
                            let _ = held.recv();
                            moli_core::RendererNativeOperationStep::Complete(
                                moli_core::RendererNativeProtocolResponse::success(json!({})),
                            )
                        },
                    ),
                )
                .unwrap()
        });
        let release = ReleaseRenderer(Some(release));
        tokio::time::timeout(std::time::Duration::from_secs(30), waiting)
            .await
            .unwrap()
            .unwrap();
        let access = context.handle();
        let mut workflow = Box::pin(async move {
            let mut background = Vec::new();
            let mut protocol = Vec::new();
            let result = dispose_live_handlers_using(
                &mut ProtocolWorkflowDriver::new(access),
                &mut background,
                &mut protocol,
                &plan,
            )
            .await;
            (result, background, protocol)
        });
        assert!(
            workflow.as_mut().now_or_never().is_none(),
            "the real renderer cleanup remains held"
        );
        let command = ParsedCdpCommand::parse_str(
            json!({"id":1000,"method":"Browser.getVersion"}).to_string(),
        )
        .unwrap();
        context.with_fixture(|conn| {
            let crate::conn::CdpCommandTaskStep::Complete(outcome) = conn
                .start_parsed_command_dispatch_with_context(
                    &command,
                    &mut CommandDispatchContext::default(),
                )
            else {
                panic!("independent command must complete during renderer cleanup");
            };
            assert!(
                outcome
                    .into_parts()
                    .0
                    .iter()
                    .any(|message| message["id"] == 1000 && message.get("result").is_some())
            );
            assert!(conn.has_attached_devtools_session("SID-handler-disposal"));
        });
        drop(release);
        let (result, _, _) = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            context.drive_owned_wait(workflow),
        )
        .await
        .expect("all admitted handler cleanup must terminate");
        drop(gate);
        drop(guard);
        let (error, _) = result.into_parts();
        assert!(error.is_none(), "{error:#?}");
        context.with_fixture(|conn| {
            assert!(conn.has_attached_devtools_session("SID-handler-disposal"));
            assert!(
                conn.target_owner_state_for_session(Some("SID-handler-disposal"))
                    .unwrap()
                    .document_start_scripts
                    .is_empty()
            );
        });
    }

    #[tokio::test]
    async fn renderer_cleanup_failure_keeps_script_authority_and_still_clears_later_domains() {
        let (context, plan) = fixture().await;
        let owner = context
            .with_fixture(|conn| CommandOwnerScope::capture(conn, Some("SID-handler-disposal")));
        context.with_fixture(|conn| {
            conn.loaded_page_mut_for_owner(&owner)
                .unwrap()
                .crash_devtools_target_from_io()
        });
        let mut background = Vec::new();
        let mut protocol = Vec::new();
        let state_owner = context.state;
        let result = dispose_live_handlers_using(
            &mut ProtocolWorkflowDriver::new(state_owner.handle()),
            &mut background,
            &mut protocol,
            &plan,
        )
        .await;
        let (error, _) = result.into_parts();
        let error = error.expect("the dead renderer must reject script removal");
        assert!(
            error.chain().count() > 1,
            "the handler context must retain the native cause"
        );
        state_owner
            .with_state(|state| {
                state.fixture(|connection| {
                    assert!(connection.has_attached_devtools_session("SID-handler-disposal"));
                    assert!(
                        !connection
                            .target_owner_state_for_session(Some("SID-handler-disposal"))
                            .unwrap()
                            .document_start_scripts
                            .is_empty()
                    );
                    connection
                        .with_target_devtools_session_state_for_owner_mut(&owner, |state| {
                            assert!(!state.network_session_state.network_enabled);
                            assert_eq!(state.emulation_session_state, Default::default());
                        })
                        .expect("every handler ran before the binding was removed");
                })
            })
            .unwrap();
    }

    #[test]
    fn handler_error_context_retains_the_first_typed_cause() {
        let plan =
            SessionDisposalPlan::for_session_route("SID", &crate::conn::CdpSessionRoute::Browser)
                .unwrap();
        let mut result = DevToolsSessionHandlerDisposal {
            first_error: None,
            renderer_output_predecessor: None,
        };
        result.record_error(
            DevToolsSessionDomainHandler::Page,
            &plan,
            std::io::Error::from(std::io::ErrorKind::PermissionDenied).into(),
        );
        result.record_error(
            DevToolsSessionDomainHandler::Network,
            &plan,
            std::io::Error::from(std::io::ErrorKind::ConnectionReset).into(),
        );
        let error = result.into_parts().0.unwrap();
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::PermissionDenied
        );
    }

    #[test]
    fn handler_sets_are_owned_by_session_target_kind() {
        let page = SessionDisposalTarget::PageTarget {
            browser_context_id: "BID".to_owned(),
            target_id: "TID".to_owned(),
            session_key: DevToolsSessionKey::Attached("SID".to_owned()),
        };
        assert_eq!(
            DevToolsSessionHandlers::for_target(&page).handlers,
            PAGE_HANDLERS
        );

        let worker = SessionDisposalTarget::DedicatedWorkerTarget {
            browser_context_id: "BID".to_owned(),
            target_id: "WID".to_owned(),
        };
        assert_eq!(
            DevToolsSessionHandlers::for_target(&worker).handlers,
            WORKER_HANDLERS
        );
        assert_eq!(
            DevToolsSessionHandlers::for_target(&SessionDisposalTarget::Browser).handlers,
            CONNECTION_HANDLERS
        );
    }
}
