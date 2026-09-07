use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
};

use moli_core::browser::BrowserContextId;

use super::{CdpConnection, CdpSessionRoute};
use crate::devtools_runtime::{
    DevToolsBrowserContextId, DevToolsCommand, DevToolsCommandContext, DevToolsCommandResult,
    DevToolsError, DevToolsErrorKind, DevToolsTargetId,
};

/// Session ownership is a set of exact native Context identities. It is not a
/// second Browser/AgentHost directory and cannot adopt a replacement by name.
pub(super) struct WebDriverSessionScope {
    default_context: String,
    contexts: HashMap<String, BrowserContextId>,
    dialog_handler_enabled: bool,
    downloads: RefCell<HashSet<String>>,
    // Last successful session-wide settings, reused for newly created contexts.
    global_settings: Vec<DevToolsCommand>,
}

impl CdpConnection {
    pub(super) fn webdriver_global_setting(
        &self,
        command: &DevToolsCommand,
    ) -> Option<DevToolsCommand> {
        self.webdriver_scope(command.context())?;
        let global = match command {
            DevToolsCommand::SetUserAgentOverride(c) => {
                c.target_ids.is_empty() && c.browser_context_ids.is_empty()
            }
            DevToolsCommand::SetGeolocationOverride(c) => {
                c.target_ids.is_empty() && c.browser_context_ids.is_empty()
            }
            DevToolsCommand::SetNetworkConditions(c) => {
                c.target_ids.is_empty() && c.browser_context_ids.is_empty()
            }
            DevToolsCommand::SetExtraHeaders(c) => {
                c.target_ids.is_empty() && c.browser_context_ids.is_empty()
            }
            DevToolsCommand::SetCacheBehavior(c) => c.target_ids.is_empty(),
            DevToolsCommand::SetDownloadBehavior(c) => c.user_contexts.is_none(),
            _ => false,
        };
        global.then(|| command.clone())
    }

    pub(super) fn remember_webdriver_global_setting(&mut self, command: DevToolsCommand) {
        let session = command.context().session_id.as_ref().unwrap();
        let scope = self.webdriver_sessions.get_mut(session.as_str()).unwrap();
        scope.global_settings.retain(|previous| {
            std::mem::discriminant(previous) != std::mem::discriminant(&command)
        });
        scope.global_settings.push(command);
    }

    pub(super) fn webdriver_context_initial_settings(
        &self,
        context: &DevToolsCommandContext,
        id: &DevToolsBrowserContextId,
    ) -> Vec<DevToolsCommand> {
        let Some(scope) = self.webdriver_scope(context) else {
            return Vec::new();
        };
        scope
            .global_settings
            .iter()
            .filter_map(|setting| {
                let mut setting = setting.clone();
                setting.context_mut().target_id = None;
                setting.context_mut().browser_context_id = Some(id.clone());
                match &mut setting {
                    DevToolsCommand::SetUserAgentOverride(c) => {
                        c.browser_context_ids = vec![id.clone()]
                    }
                    DevToolsCommand::SetGeolocationOverride(c) => {
                        c.browser_context_ids = vec![id.clone()]
                    }
                    DevToolsCommand::SetNetworkConditions(c) => {
                        c.browser_context_ids = vec![id.clone()]
                    }
                    DevToolsCommand::SetExtraHeaders(c) => c.browser_context_ids = vec![id.clone()],
                    DevToolsCommand::SetDownloadBehavior(c) => {
                        c.user_contexts = Some(vec![id.clone()])
                    }
                    // Cache is read by target creation; an empty Context has no renderer policy yet.
                    _ => return None,
                }
                Some(setting)
            })
            .collect()
    }

    pub(crate) fn cache_disabled_for_browser_context(&self, id: &str) -> bool {
        let native = self
            .browser_context_by_id(id)
            .map(|context| context.browser_context_id());
        self.webdriver_sessions
            .values()
            .find(|scope| scope.contexts.get(id).copied() == native && native.is_some())
            .and_then(|scope| {
                scope
                    .global_settings
                    .iter()
                    .find_map(|setting| match setting {
                        DevToolsCommand::SetCacheBehavior(c) => Some(c.cache_disabled),
                        _ => None,
                    })
            })
            .unwrap_or(self.browser_global_overrides.cache_disabled)
    }

    pub(super) fn webdriver_command_owner_scope(
        &self,
        context: &DevToolsCommandContext,
    ) -> Option<super::CommandOwnerScope> {
        if let Some(target) = &context.target_id {
            if !self.webdriver_target_is_visible(context, target.as_str()) {
                return None;
            }
            return self
                .target_session_route_for_target_id(target.as_str())
                .or_else(|| self.target_session_route_for_child_frame_id(target.as_str()))
                .map(super::CommandOwnerScope::for_route);
        }
        let scope = self.webdriver_scope(context)?;
        let id = context
            .browser_context_id
            .as_ref()
            .map(|id| id.as_str())
            .filter(|id| !matches!(*id, "default" | "BID-default"))
            .unwrap_or(&scope.default_context);
        if !self.webdriver_context_is_visible(context, id) {
            return None;
        }
        let browser_context = self.browser_context_by_id(id)?;
        let route = match browser_context.active_target_id() {
            Some(target_id) => CdpSessionRoute::PageTarget {
                browser_context_id: id.to_owned(),
                target_id: target_id.to_owned(),
                session_key: moli_page_types::DevToolsSessionKey::Primary,
            },
            None => CdpSessionRoute::BrowserContext {
                browser_context_id: id.to_owned(),
            },
        };
        Some(super::CommandOwnerScope::for_route(route))
    }

    pub fn attach_webdriver_session(&mut self, session_id: &str) -> Result<(), DevToolsError> {
        if self.webdriver_sessions.contains_key(session_id) {
            return Err(DevToolsError::new(
                DevToolsErrorKind::InvalidArgument,
                "WebDriver session already attached",
            ));
        }
        let id = self.gen_bc_id();
        let context = self.new_browser_context(id.clone());
        let native_id = context.browser_context_id();
        self.insert_browser_context(context);
        self.webdriver_sessions.insert(
            session_id.to_owned(),
            WebDriverSessionScope {
                default_context: id.clone(),
                contexts: HashMap::from([(id, native_id)]),
                dialog_handler_enabled: false,
                downloads: RefCell::default(),
                global_settings: Vec::new(),
            },
        );
        Ok(())
    }

    pub fn close_webdriver_session(&mut self, session_id: &str) -> Result<(), String> {
        let Some(scope) = self.webdriver_sessions.remove(session_id) else {
            return Ok(());
        };
        for native_id in scope.contexts.into_values() {
            self.browser.remove_context(native_id)?;
        }
        Ok(())
    }

    pub fn set_webdriver_session_dialog_handler_enabled(
        &mut self,
        session_id: &str,
        enabled: bool,
    ) -> bool {
        let Some(scope) = self.webdriver_sessions.get_mut(session_id) else {
            return false;
        };
        scope.dialog_handler_enabled = enabled;
        let contexts = scope.contexts.clone();
        for (id, native) in contexts {
            if let Some(context) = self
                .browser_context_by_id(&id)
                .filter(|context| context.browser_context_id() == native)
            {
                context.set_javascript_dialog_handler_enabled(enabled);
            }
        }
        true
    }

    fn webdriver_scope(&self, context: &DevToolsCommandContext) -> Option<&WebDriverSessionScope> {
        self.webdriver_sessions
            .get(context.session_id.as_ref()?.as_str())
    }

    pub fn webdriver_context_is_visible(
        &self,
        context: &DevToolsCommandContext,
        browser_context_id: &str,
    ) -> bool {
        self.webdriver_scope(context).is_none_or(|scope| {
            scope
                .contexts
                .get(browser_context_id)
                .is_some_and(|native_id| {
                    self.browser_context_by_id(browser_context_id)
                        .is_some_and(|current| current.browser_context_id() == *native_id)
                })
        })
    }

    pub fn webdriver_target_is_visible(
        &self,
        context: &DevToolsCommandContext,
        target_id: &str,
    ) -> bool {
        if self.webdriver_scope(context).is_none() {
            return true;
        }
        self.target_session_route_for_target_id(target_id)
            .or_else(|| self.target_session_route_for_child_frame_id(target_id))
            .and_then(|route| match route {
                CdpSessionRoute::PageTarget {
                    browser_context_id, ..
                }
                | CdpSessionRoute::DedicatedWorkerTarget {
                    browser_context_id, ..
                }
                | CdpSessionRoute::SharedWorkerTarget {
                    browser_context_id, ..
                }
                | CdpSessionRoute::ServiceWorkerTarget {
                    browser_context_id, ..
                } => Some(browser_context_id),
                _ => None,
            })
            .is_some_and(|id| self.webdriver_context_is_visible(context, &id))
    }

    pub fn webdriver_event_is_visible(
        &self,
        session: Option<&str>,
        event: &super::BackgroundProtocolEvent,
    ) -> bool {
        let (_, automation) = event.clone().into_parts();
        self.webdriver_automation_event_is_visible(
            session,
            automation.as_ref(),
            event.navigation_gate_target_id(),
        )
    }

    pub fn webdriver_automation_event_is_visible(
        &self,
        session: Option<&str>,
        automation: Option<&crate::devtools_runtime::AutomationEvent>,
        fallback_target: Option<&str>,
    ) -> bool {
        use crate::devtools_runtime::AutomationEvent as Event;
        let Some(session) = session.filter(|id| self.webdriver_sessions.contains_key(*id)) else {
            return true;
        };
        let context = DevToolsCommandContext {
            protocol: crate::devtools_runtime::DevToolsProtocol::WebDriverBidi,
            session_id: Some(crate::devtools_runtime::DevToolsSessionId::from(session)),
            target_id: None,
            browser_context_id: None,
        };
        let target = match automation {
            Some(Event::TargetCreated(event) | Event::TargetDestroyed(event)) => {
                return event.browser_context_id.as_ref().is_some_and(|id| {
                    self.webdriver_sessions[session]
                        .contexts
                        .contains_key(id.as_str())
                });
            }
            Some(Event::TargetAttached(event)) => Some(event.target_id.as_str()),
            Some(Event::TargetDetached(event)) => Some(event.target_id.as_str()),
            Some(Event::NavigationFrame(event)) => Some(event.target_id.as_str()),
            Some(
                Event::NavigationStarted(event)
                | Event::DomContentLoaded(event)
                | Event::Load(event),
            ) => Some(event.target_id.as_str()),
            Some(Event::PageLifecycle(event)) => Some(event.target_id.as_str()),
            Some(Event::SameDocumentNavigation(event)) => Some(event.target_id.as_str()),
            Some(
                Event::NetworkBeforeRequestSent(event)
                | Event::NetworkResponseStarted(event)
                | Event::NetworkResponseCompleted(event)
                | Event::NetworkFetchError(event)
                | Event::NetworkAuthRequired(event)
                | Event::RequestPaused(event),
            ) => Some(event.target_id.as_str()),
            Some(Event::RuntimeConsoleApiCalled(event)) => {
                event.target_id.as_ref().map(|id| id.as_str())
            }
            Some(
                Event::RuntimeExecutionContextCreated(event)
                | Event::RuntimeExecutionContextDestroyed(event),
            ) => event.target_id.as_ref().map(|id| id.as_str()),
            Some(Event::RuntimeExecutionContextsCleared(event)) => {
                event.target_id.as_ref().map(|id| id.as_str())
            }
            Some(Event::LogEntryAdded(event)) => event.target_id.as_ref().map(|id| id.as_str()),
            Some(Event::ScriptMessage(event)) => event.target_id.as_ref().map(|id| id.as_str()),
            Some(Event::ScriptException(event)) => event.target_id.as_ref().map(|id| id.as_str()),
            Some(Event::UserPromptClosed(event)) => Some(event.frame_id.as_str()),
            Some(Event::PageJavaScriptDialogOpening(event)) => {
                event.frame_id.as_ref().map(|id| id.as_str())
            }
            Some(Event::PageFileChooserOpened(event)) => Some(event.frame_id.as_str()),
            Some(Event::BrowserDownloadWillBegin(event)) => {
                let visible = self.webdriver_target_is_visible(&context, event.frame_id.as_str());
                if visible {
                    self.webdriver_sessions
                        .get(session)
                        .unwrap()
                        .downloads
                        .borrow_mut()
                        .insert(event.guid.clone());
                }
                return visible;
            }
            Some(Event::BrowserDownloadProgress(event)) => {
                return self.webdriver_sessions[session]
                    .downloads
                    .borrow()
                    .contains(&event.guid);
            }
            _ => None,
        };
        target
            .or(fallback_target)
            .is_some_and(|target| self.webdriver_target_is_visible(&context, target))
    }

    pub(crate) fn set_webdriver_cache_disabled(
        &mut self,
        context: &DevToolsCommandContext,
        disabled: bool,
    ) -> bool {
        let Some(scope) = self.webdriver_scope(context) else {
            return false;
        };
        let ids = scope
            .contexts
            .keys()
            .filter(|id| self.webdriver_context_is_visible(context, id))
            .cloned()
            .collect::<Vec<_>>();
        for id in ids {
            if let Some(context) = self.browser_context_by_id_mut(&id) {
                context.apply_browser_cache_disabled(disabled);
            }
        }
        true
    }

    pub fn prepare_webdriver_command(
        &self,
        command: &mut DevToolsCommand,
    ) -> Result<(), DevToolsError> {
        let Some(scope) = self.webdriver_scope(command.context()) else {
            return Ok(());
        };
        // Directory queries remain valid after the current window has closed.
        if matches!(
            command,
            DevToolsCommand::GetTargets(_) | DevToolsCommand::GetBrowserContexts(_)
        ) {
            command.context_mut().target_id = None;
        }
        if command.context().target_id.as_ref().is_some_and(|target| {
            !self.webdriver_target_is_visible(command.context(), target.as_str())
        }) {
            return Err(DevToolsError::new(
                DevToolsErrorKind::NoSuchTarget,
                "Target is outside the WebDriver session",
            ));
        }
        let requested = command
            .context()
            .browser_context_id
            .as_ref()
            .map(|id| id.as_str());
        let selected = match requested {
            None => command
                .context()
                .target_id
                .as_ref()
                .and_then(|target| self.browser_context_id_for_target(target.as_str()))
                .unwrap_or(&scope.default_context)
                .to_owned(),
            Some("default" | "BID-default") => scope.default_context.clone(),
            Some(id) if self.webdriver_context_is_visible(command.context(), id) => id.to_owned(),
            _ => {
                return Err(DevToolsError::new(
                    DevToolsErrorKind::NoSuchTarget,
                    "Context is outside the WebDriver session",
                ));
            }
        };
        command.context_mut().browser_context_id =
            Some(DevToolsBrowserContextId::from(selected.clone()));
        let context = command.context().clone();
        let outside = || {
            DevToolsError::new(
                DevToolsErrorKind::NoSuchTarget,
                "Object is outside the WebDriver session",
            )
        };
        let target = |id: &DevToolsTargetId| {
            self.webdriver_target_is_visible(&context, id.as_str())
                .then_some(())
                .ok_or_else(outside)
        };
        let context_id = |id: &mut DevToolsBrowserContextId| {
            if matches!(id.as_str(), "default" | "BID-default") {
                *id = DevToolsBrowserContextId::from(scope.default_context.clone());
            }
            self.webdriver_context_is_visible(&context, id.as_str())
                .then_some(())
                .ok_or_else(outside)
        };
        let optional_context = |id: &mut Option<DevToolsBrowserContextId>| {
            context_id(id.get_or_insert_with(|| DevToolsBrowserContextId::from(selected.clone())))
        };
        let contexts = |ids: &mut Vec<DevToolsBrowserContextId>, global: bool| {
            if global && ids.is_empty() {
                *ids = scope
                    .contexts
                    .keys()
                    .filter(|id| self.webdriver_context_is_visible(&context, id))
                    .cloned()
                    .map(DevToolsBrowserContextId::from)
                    .collect();
                ids.sort();
                if ids.is_empty() {
                    return Err(outside());
                }
            }
            ids.iter_mut().try_for_each(context_id)
        };
        macro_rules! scoped_targets_and_contexts {
            ($command:expr) => {{
                $command.target_ids.iter().try_for_each(target)?;
                contexts(
                    &mut $command.browser_context_ids,
                    $command.target_ids.is_empty(),
                )?;
            }};
        }
        match command {
            DevToolsCommand::CreateTarget(c) => optional_context(&mut c.browser_context_id)?,
            DevToolsCommand::RemoveBrowserContext(c) => {
                context_id(&mut c.browser_context_id)?;
            }
            DevToolsCommand::CloseTarget(c) => target(&c.target_id)?,
            DevToolsCommand::ActivateTarget(c) => target(&c.target_id)?,
            DevToolsCommand::GetTargetInfo(c) => {
                if let Some(id) = &c.target_id {
                    target(id)?;
                }
            }
            DevToolsCommand::GetTargets(c) => {
                if let Some(id) = &c.root {
                    target(id)?;
                }
            }
            DevToolsCommand::GetServiceWorkerLogs(c) => {
                if let Some(id) = &c.target_id {
                    target(id)?;
                }
            }
            DevToolsCommand::SetClientWindowState(c) => target(&c.client_window)?,
            DevToolsCommand::SetPermission(c) => optional_context(&mut c.browser_context_id)?,
            DevToolsCommand::GetCookies(c) => optional_context(&mut c.browser_context_id)?,
            DevToolsCommand::SetCookies(c) => optional_context(&mut c.browser_context_id)?,
            DevToolsCommand::DeleteCookies(c) => optional_context(&mut c.browser_context_id)?,
            DevToolsCommand::SetDownloadBehavior(c) => {
                contexts(c.user_contexts.get_or_insert_with(Vec::new), true)?
            }
            DevToolsCommand::SetViewport(c) => {
                contexts(&mut c.browser_context_ids, c.context.target_id.is_none())?
            }
            DevToolsCommand::SetUserAgentOverride(c) => scoped_targets_and_contexts!(c),
            DevToolsCommand::SetLocaleOverride(c) => scoped_targets_and_contexts!(c),
            DevToolsCommand::SetTimezoneOverride(c) => scoped_targets_and_contexts!(c),
            DevToolsCommand::SetGeolocationOverride(c) => scoped_targets_and_contexts!(c),
            DevToolsCommand::SetNetworkConditions(c) => scoped_targets_and_contexts!(c),
            DevToolsCommand::SetExtraHeaders(c) => scoped_targets_and_contexts!(c),
            DevToolsCommand::AddNetworkDataCollector(c) => scoped_targets_and_contexts!(c),
            DevToolsCommand::AddPreloadScript(c) => {
                if let Some(ids) = &c.target_ids {
                    ids.iter().try_for_each(target)?;
                }
                contexts(&mut c.browser_context_ids, c.target_ids.is_none())?;
            }
            DevToolsCommand::SetCacheBehavior(c) => {
                c.target_ids.iter().try_for_each(target)?;
            }
            DevToolsCommand::ContinueInterceptedRequest(c) => {
                self.validate_webdriver_fetch_request(&context, c.request_id.as_str())?
            }
            DevToolsCommand::ContinueInterceptedResponse(c) => {
                self.validate_webdriver_fetch_request(&context, c.request_id.as_str())?
            }
            DevToolsCommand::ContinueWithAuth(c) => {
                self.validate_webdriver_fetch_request(&context, c.request_id.as_str())?
            }
            DevToolsCommand::FailInterceptedRequest(c) => {
                self.validate_webdriver_fetch_request(&context, c.request_id.as_str())?
            }
            DevToolsCommand::FulfillInterceptedRequest(c) => {
                self.validate_webdriver_fetch_request(&context, c.request_id.as_str())?
            }
            _ => {}
        }
        Ok(())
    }

    fn validate_webdriver_fetch_request(
        &self,
        context: &DevToolsCommandContext,
        request_id: &str,
    ) -> Result<(), DevToolsError> {
        if let Some(route) = self.pending_fetch_request_session_route(request_id) {
            let id = match route {
                CdpSessionRoute::PageTarget {
                    browser_context_id, ..
                }
                | CdpSessionRoute::DedicatedWorkerTarget {
                    browser_context_id, ..
                }
                | CdpSessionRoute::SharedWorkerTarget {
                    browser_context_id, ..
                }
                | CdpSessionRoute::ServiceWorkerTarget {
                    browser_context_id, ..
                }
                | CdpSessionRoute::BrowserContext { browser_context_id } => {
                    Some(browser_context_id)
                }
                _ => None,
            };
            if !id.is_some_and(|id| self.webdriver_context_is_visible(context, &id)) {
                return Err(DevToolsError::new(
                    DevToolsErrorKind::NoSuchTarget,
                    "Request is outside the WebDriver session",
                ));
            }
        }
        Ok(())
    }

    pub fn finish_webdriver_command(
        &mut self,
        context: &DevToolsCommandContext,
        result: &mut Result<DevToolsCommandResult, DevToolsError>,
    ) {
        let Some(session) = context
            .session_id
            .as_ref()
            .filter(|session| self.webdriver_sessions.contains_key(session.as_str()))
        else {
            return;
        };
        match result {
            Ok(DevToolsCommandResult::CreateBrowserContext(created)) => {
                if let Some(native_id) = self
                    .browser_context_by_id(created.browser_context_id.as_str())
                    .map(|context| context.browser_context_id())
                {
                    let scope = self.webdriver_sessions.get_mut(session.as_str()).unwrap();
                    scope
                        .contexts
                        .insert(created.browser_context_id.as_str().to_owned(), native_id);
                    let enabled = scope.dialog_handler_enabled;
                    self.browser_context_by_id(created.browser_context_id.as_str())
                        .unwrap()
                        .set_javascript_dialog_handler_enabled(enabled);
                }
            }
            Ok(DevToolsCommandResult::ClientWindows(windows)) => {
                windows.client_windows.retain(|window| {
                    self.webdriver_target_is_visible(context, window.client_window.as_str())
                })
            }
            Ok(DevToolsCommandResult::GetTargets(targets)) => targets.targets.retain(|target| {
                target
                    .browser_context_id
                    .as_ref()
                    .is_some_and(|id| self.webdriver_context_is_visible(context, id.as_str()))
            }),
            Ok(DevToolsCommandResult::GetFrameTrees(trees)) => trees.frame_trees.retain(|tree| {
                tree.target_info
                    .as_ref()
                    .and_then(|target| target.browser_context_id.as_ref())
                    .is_some_and(|id| self.webdriver_context_is_visible(context, id.as_str()))
            }),
            Ok(DevToolsCommandResult::GetBrowserContexts(contexts)) => contexts
                .browser_context_ids
                .retain(|id| self.webdriver_context_is_visible(context, id.as_str())),
            Ok(DevToolsCommandResult::Realms(realms)) => realms.realms.retain(|realm| {
                realm
                    .target_id
                    .as_ref()
                    .is_some_and(|id| self.webdriver_target_is_visible(context, id.as_str()))
            }),
            _ => {}
        }
    }
}
