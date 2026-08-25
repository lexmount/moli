use std::collections::{HashMap, HashSet};

use anyhow::{Result, bail};
use serde_json::Value;

use crate::cdp_writer::CdpSocketSink;

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum CdpSessionFrontendKind {
    Browser,
    Page,
    ForegroundTab,
}

pub(super) struct CdpSessionFrontendRoute {
    kind: CdpSessionFrontendKind,
    target_id: Option<String>,
    browser_context_id: Option<String>,
    base_session_id: String,
    replayable_target_commands: Vec<(String, Value)>,
    sink: CdpSocketSink,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ForegroundTabFrontendRoute {
    pub(crate) frontend_id: u64,
    pub(crate) page_target_id: String,
    pub(crate) base_session_id: String,
    pub(crate) replayable_target_commands: Vec<(String, Value)>,
}

#[derive(Clone)]
pub(super) struct FrontendSessionRoute {
    pub(super) frontend_id: u64,
    pub(super) kind: FrontendSessionKind,
}

#[derive(Clone)]
pub(super) enum FrontendSessionKind {
    /// One hidden browser-target or page-target session owned by exactly one
    /// WebSocket frontend. Commands without a public sessionId dispatch here.
    Base,
    /// A client-visible session created underneath the base session (or one
    /// of its descendants). Parent and target identity enforce Chromium's
    /// per-TargetHandler session lookup boundary.
    Child {
        parent_session_id: Option<String>,
        target_id: Option<String>,
    },
}

#[derive(Clone)]
enum SessionBinding {
    InternalControl,
    Frontend(FrontendSessionRoute),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DirectChildLookupError {
    MissingSession,
    MissingTarget,
    AmbiguousTarget,
}

#[derive(Default)]
pub(super) struct FrontendRegistry {
    frontends: HashMap<u64, CdpSessionFrontendRoute>,
    sessions: HashMap<String, SessionBinding>,
}

impl Drop for FrontendRegistry {
    fn drop(&mut self) {
        for frontend in self.frontends.values() {
            frontend.sink.close_after_flush();
        }
    }
}

impl FrontendRegistry {
    pub(super) fn register_browser_frontend(
        &mut self,
        frontend_id: u64,
        session_id: String,
        sink: CdpSocketSink,
    ) -> Result<()> {
        self.register_session_frontend(
            frontend_id,
            CdpSessionFrontendKind::Browser,
            None,
            None,
            session_id,
            sink,
            true,
        )
    }

    pub(super) fn register_page_frontend(
        &mut self,
        frontend_id: u64,
        target_id: String,
        session_id: String,
        sink: CdpSocketSink,
    ) -> Result<()> {
        self.register_session_frontend(
            frontend_id,
            CdpSessionFrontendKind::Page,
            Some(target_id),
            None,
            session_id,
            sink,
            false,
        )
    }

    pub(super) fn register_foreground_tab_frontend(
        &mut self,
        frontend_id: u64,
        browser_context_id: Option<String>,
        page_target_id: String,
        session_id: String,
        sink: CdpSocketSink,
    ) -> Result<()> {
        self.register_session_frontend(
            frontend_id,
            CdpSessionFrontendKind::ForegroundTab,
            Some(page_target_id),
            browser_context_id,
            session_id,
            sink,
            false,
        )
    }

    fn register_session_frontend(
        &mut self,
        frontend_id: u64,
        kind: CdpSessionFrontendKind,
        target_id: Option<String>,
        browser_context_id: Option<String>,
        session_id: String,
        sink: CdpSocketSink,
        may_claim_control_session: bool,
    ) -> Result<()> {
        if self.frontends.contains_key(&frontend_id) {
            bail!("CDP frontend id is already registered");
        }
        match self.sessions.get(&session_id) {
            None => {}
            Some(SessionBinding::InternalControl) if may_claim_control_session => {}
            Some(SessionBinding::InternalControl) => {
                bail!("CDP frontend session is reserved for internal control");
            }
            Some(SessionBinding::Frontend(_)) => {
                bail!("CDP frontend session is already registered");
            }
        }
        self.sessions.insert(
            session_id.clone(),
            SessionBinding::Frontend(FrontendSessionRoute {
                frontend_id,
                kind: FrontendSessionKind::Base,
            }),
        );
        self.frontends.insert(
            frontend_id,
            CdpSessionFrontendRoute {
                kind,
                target_id,
                browser_context_id,
                base_session_id: session_id,
                replayable_target_commands: Vec::new(),
                sink,
            },
        );
        Ok(())
    }

    pub(super) fn register_internal_control_session(&mut self, session_id: String) -> Result<()> {
        match self.sessions.get(&session_id) {
            None => {
                self.sessions
                    .insert(session_id, SessionBinding::InternalControl);
                Ok(())
            }
            Some(SessionBinding::InternalControl) => Ok(()),
            Some(SessionBinding::Frontend(_)) => {
                bail!("CDP internal control session is already owned by a frontend");
            }
        }
    }

    pub(super) fn unregister_browser_frontend(&mut self, frontend_id: u64) -> Option<String> {
        self.unregister_session_frontend(frontend_id, CdpSessionFrontendKind::Browser)
            .map(|route| route.base_session_id)
    }

    pub(super) fn unregister_page_frontend(&mut self, frontend_id: u64) -> Option<String> {
        self.unregister_session_frontend(frontend_id, CdpSessionFrontendKind::Page)
            .map(|route| route.base_session_id)
    }

    pub(super) fn unregister_foreground_tab_frontend(
        &mut self,
        frontend_id: u64,
    ) -> Option<String> {
        self.unregister_session_frontend(frontend_id, CdpSessionFrontendKind::ForegroundTab)
            .map(|route| route.base_session_id)
    }

    fn unregister_session_frontend(
        &mut self,
        frontend_id: u64,
        expected_kind: CdpSessionFrontendKind,
    ) -> Option<CdpSessionFrontendRoute> {
        if self.frontends.get(&frontend_id)?.kind != expected_kind {
            return None;
        }
        let route = self.frontends.remove(&frontend_id)?;
        self.sessions.retain(|_, binding| {
            !matches!(
                binding,
                SessionBinding::Frontend(session) if session.frontend_id == frontend_id
            )
        });
        route.sink.close_after_flush();
        Some(route)
    }

    pub(super) fn unregister_page_frontends_for_target(&mut self, target_id: &str) -> Vec<u64> {
        let frontend_ids = self
            .frontends
            .iter()
            .filter_map(|(frontend_id, route)| {
                (matches!(
                    route.kind,
                    CdpSessionFrontendKind::Page | CdpSessionFrontendKind::ForegroundTab
                ) && route.target_id.as_deref() == Some(target_id))
                .then_some(*frontend_id)
            })
            .collect::<Vec<_>>();
        for frontend_id in &frontend_ids {
            match self.frontends.get(frontend_id).map(|route| route.kind) {
                Some(CdpSessionFrontendKind::Page) => {
                    self.unregister_page_frontend(*frontend_id);
                }
                Some(CdpSessionFrontendKind::ForegroundTab) => {
                    self.unregister_foreground_tab_frontend(*frontend_id);
                }
                Some(CdpSessionFrontendKind::Browser) | None => {}
            }
        }
        frontend_ids
    }

    pub(super) fn foreground_tab_frontends_for_browser_context(
        &self,
        browser_context_id: Option<&str>,
    ) -> Vec<ForegroundTabFrontendRoute> {
        self.frontends
            .iter()
            .filter_map(|(frontend_id, route)| {
                if route.kind != CdpSessionFrontendKind::ForegroundTab
                    || route.browser_context_id.as_deref() != browser_context_id
                {
                    return None;
                }
                Some(ForegroundTabFrontendRoute {
                    frontend_id: *frontend_id,
                    page_target_id: route.target_id.clone()?,
                    base_session_id: route.base_session_id.clone(),
                    replayable_target_commands: route.replayable_target_commands.clone(),
                })
            })
            .collect()
    }

    pub(super) fn replace_foreground_tab_frontend_base(
        &mut self,
        frontend_id: u64,
        browser_context_id: Option<String>,
        page_target_id: String,
        session_id: String,
    ) -> Result<String> {
        if self.sessions.contains_key(&session_id) {
            bail!("CDP foreground tab replacement session is already registered");
        }
        let previous_session_id = {
            let Some(route) = self.frontends.get_mut(&frontend_id) else {
                bail!("CDP foreground tab frontend is not registered");
            };
            if route.kind != CdpSessionFrontendKind::ForegroundTab {
                bail!("CDP frontend is not a foreground tab");
            }
            let previous_session_id =
                std::mem::replace(&mut route.base_session_id, session_id.clone());
            route.target_id = Some(page_target_id);
            route.browser_context_id = browser_context_id;
            previous_session_id
        };
        self.remove_session_descendants(&previous_session_id);
        self.sessions.remove(&previous_session_id);
        self.sessions.insert(
            session_id,
            SessionBinding::Frontend(FrontendSessionRoute {
                frontend_id,
                kind: FrontendSessionKind::Base,
            }),
        );
        Ok(previous_session_id)
    }

    pub(super) fn is_foreground_tab_frontend(&self, frontend_id: u64) -> bool {
        self.frontends
            .get(&frontend_id)
            .is_some_and(|route| route.kind == CdpSessionFrontendKind::ForegroundTab)
    }

    pub(super) fn remember_foreground_tab_target_command(
        &mut self,
        frontend_id: u64,
        method: String,
        params: Value,
    ) {
        let Some(route) = self.frontends.get_mut(&frontend_id) else {
            return;
        };
        if route.kind != CdpSessionFrontendKind::ForegroundTab {
            return;
        }
        if let Some((_, previous_params)) = route
            .replayable_target_commands
            .iter_mut()
            .find(|(previous_method, _)| previous_method == &method)
        {
            *previous_params = params;
            return;
        }
        route.replayable_target_commands.push((method, params));
    }

    pub(super) fn base_session_id(&self, frontend_id: u64) -> Option<&str> {
        self.frontends
            .get(&frontend_id)
            .map(|route| route.base_session_id.as_str())
    }

    pub(super) fn frontend_sink(&self, frontend_id: u64) -> Option<CdpSocketSink> {
        self.frontends
            .get(&frontend_id)
            .map(|route| route.sink.clone())
    }

    pub(super) fn session(&self, session_id: &str) -> Option<&FrontendSessionRoute> {
        match self.sessions.get(session_id) {
            Some(SessionBinding::Frontend(session)) => Some(session),
            Some(SessionBinding::InternalControl) | None => None,
        }
    }

    pub(super) fn is_internal_control_session(&self, session_id: &str) -> bool {
        matches!(
            self.sessions.get(session_id),
            Some(SessionBinding::InternalControl)
        )
    }

    pub(super) fn remove_internal_control_session(&mut self, session_id: &str) -> bool {
        if !self.is_internal_control_session(session_id) {
            return false;
        }
        self.sessions.remove(session_id);
        true
    }

    pub(super) fn owns_client_session(&self, frontend_id: u64, session_id: &str) -> bool {
        self.session(session_id).is_some_and(|session| {
            session.frontend_id == frontend_id
                && matches!(session.kind, FrontendSessionKind::Child { .. })
        })
    }

    pub(super) fn owns_direct_child(
        &self,
        frontend_id: u64,
        parent_session_id: Option<&str>,
        child_session_id: &str,
    ) -> bool {
        self.session(child_session_id).is_some_and(|session| {
            session.frontend_id == frontend_id
                && matches!(
                    &session.kind,
                    FrontendSessionKind::Child {
                        parent_session_id: parent,
                        ..
                    } if parent.as_deref() == parent_session_id
                )
        })
    }

    pub(super) fn direct_child_for_target(
        &self,
        frontend_id: u64,
        parent_session_id: Option<&str>,
        target_id: &str,
    ) -> std::result::Result<String, DirectChildLookupError> {
        let mut matching_sessions = self.sessions.iter().filter_map(|(session_id, binding)| {
            let SessionBinding::Frontend(session) = binding else {
                return None;
            };
            (session.frontend_id == frontend_id
                && matches!(
                    &session.kind,
                    FrontendSessionKind::Child {
                        parent_session_id: parent,
                        target_id: Some(session_target_id),
                    } if parent.as_deref() == parent_session_id
                        && session_target_id == target_id
                ))
            .then_some(session_id.as_str())
        });
        let Some(session_id) = matching_sessions.next() else {
            return Err(DirectChildLookupError::MissingTarget);
        };
        if matching_sessions.next().is_some() {
            return Err(DirectChildLookupError::AmbiguousTarget);
        }
        Ok(session_id.to_owned())
    }

    pub(super) fn register_child_session(
        &mut self,
        frontend_id: u64,
        parent_session_id: Option<&str>,
        child_session_id: &str,
        target_id: Option<&str>,
    ) {
        let frontend = self.frontends.get(&frontend_id);
        if frontend.is_none() {
            return;
        }
        if let Some(parent_session_id) = parent_session_id {
            if self
                .session(parent_session_id)
                .is_none_or(|parent| parent.frontend_id != frontend_id)
            {
                return;
            }
        } else if frontend.is_none_or(|route| route.kind != CdpSessionFrontendKind::Browser) {
            return;
        }
        if let Some(existing) = self.sessions.get(child_session_id)
            && match existing {
                SessionBinding::InternalControl => true,
                SessionBinding::Frontend(session) => {
                    session.frontend_id != frontend_id
                        || matches!(session.kind, FrontendSessionKind::Base)
                }
            }
        {
            return;
        }
        self.sessions.insert(
            child_session_id.to_owned(),
            SessionBinding::Frontend(FrontendSessionRoute {
                frontend_id,
                kind: FrontendSessionKind::Child {
                    parent_session_id: parent_session_id.map(str::to_owned),
                    target_id: target_id.map(str::to_owned),
                },
            }),
        );
    }

    pub(super) fn remove_child_session_cascade(&mut self, session_id: &str) {
        if self
            .session(session_id)
            .is_none_or(|session| matches!(session.kind, FrontendSessionKind::Base))
        {
            return;
        }
        self.remove_session_descendants(session_id);
        self.sessions.remove(session_id);
    }

    pub(super) fn remove_session_descendants(&mut self, session_id: &str) {
        let mut pending = vec![session_id.to_owned()];
        let mut descendants = Vec::new();
        let mut visited = HashSet::new();
        visited.insert(session_id.to_owned());
        while let Some(parent_session_id) = pending.pop() {
            for child_session_id in self
                .sessions
                .iter()
                .filter_map(|(child_session_id, binding)| {
                    let SessionBinding::Frontend(route) = binding else {
                        return None;
                    };
                    match &route.kind {
                        FrontendSessionKind::Child {
                            parent_session_id: parent,
                            ..
                        } if parent.as_deref() == Some(parent_session_id.as_str()) => {
                            Some(child_session_id.clone())
                        }
                        FrontendSessionKind::Base | FrontendSessionKind::Child { .. } => None,
                    }
                })
                .collect::<Vec<_>>()
            {
                if visited.insert(child_session_id.clone()) {
                    pending.push(child_session_id.clone());
                    descendants.push(child_session_id);
                }
            }
        }
        for descendant in descendants {
            self.sessions.remove(&descendant);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_sink() -> CdpSocketSink {
        CdpSocketSink::for_test()
    }

    #[test]
    fn browser_frontend_claims_an_internal_control_session_atomically() {
        let mut registry = FrontendRegistry::default();
        registry
            .register_internal_control_session("SID-browser".to_owned())
            .expect("register control session");
        registry
            .register_browser_frontend(5, "SID-browser".to_owned(), test_sink())
            .expect("claim browser session");

        assert!(!registry.is_internal_control_session("SID-browser"));
        assert!(!registry.owns_client_session(5, "SID-browser-child"));
        assert!(matches!(
            registry.session("SID-browser").map(|route| &route.kind),
            Some(FrontendSessionKind::Base)
        ));
    }

    #[test]
    fn page_frontend_cannot_claim_an_internal_control_session() {
        let mut registry = FrontendRegistry::default();
        registry
            .register_internal_control_session("SID-control".to_owned())
            .expect("register control session");

        let error = registry
            .register_page_frontend(
                10,
                "TID-page".to_owned(),
                "SID-control".to_owned(),
                test_sink(),
            )
            .expect_err("page frontend must not claim control session");
        assert!(error.to_string().contains("internal control"));
        assert!(registry.is_internal_control_session("SID-control"));
    }
}
