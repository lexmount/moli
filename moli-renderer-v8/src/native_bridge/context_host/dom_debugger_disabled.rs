//! Ordinary DOM/event hooks when external DevTools instrumentation is absent.
use super::PageScriptHost;
use crate::document_runtime::{DevToolsDomPrepublishedRemoval, DomHandle, EventTargetHandle};
use crate::event_type::EventTypeKey;
use crate::script_vm::{RendererDomDebuggerPauseScheduler, RendererDomDebuggerScheduledPause};
#[cfg(feature = "devtools")]
use moli_page_types::{
    RendererDomDebuggerEventListenerBreakpoint, RendererDomDebuggerXhrBreakpoint,
};

pub(super) struct DomDebuggerState;
impl DomDebuggerState {
    pub(super) fn new(_scheduler: RendererDomDebuggerPauseScheduler) -> Self {
        Self
    }
    pub(super) fn rebind_pause_scheduler(&mut self, _scheduler: RendererDomDebuggerPauseScheduler) {
    }
    pub(super) fn debugger_pauses_enabled(&self) -> bool {
        false
    }
}

impl PageScriptHost {
    pub(crate) fn has_dom_debugger_dom_breakpoints(&self) -> bool {
        false
    }
    #[cfg(feature = "devtools")]
    pub(crate) fn configure_dom_debugger_event_listener_breakpoint(
        &mut self,
        _inspector_session_id: Option<&str>,
        _breakpoint: RendererDomDebuggerEventListenerBreakpoint,
        _enabled: bool,
    ) {
    }
    pub(crate) fn schedule_dom_debugger_event_listener_pause_for_target(
        &self,
        _event_name: &(impl EventTypeKey + ?Sized),
        _target: EventTargetHandle,
    ) -> Option<RendererDomDebuggerScheduledPause> {
        None
    }
    pub(crate) fn schedule_dom_debugger_event_listener_pause_for_interface(
        &self,
        _event_name: &(impl EventTypeKey + ?Sized),
        _target_name: &str,
    ) -> Option<RendererDomDebuggerScheduledPause> {
        None
    }
    #[cfg(feature = "devtools")]
    pub(crate) fn configure_dom_debugger_xhr_breakpoint(
        &mut self,
        _inspector_session_id: Option<&str>,
        _breakpoint: RendererDomDebuggerXhrBreakpoint,
        _enabled: bool,
    ) {
    }

    pub(crate) fn break_on_dom_debugger_will_insert_dom_node(&mut self, _parent: DomHandle) {}
    pub(crate) fn break_on_dom_debugger_character_data_modified(&mut self, _target: DomHandle) {}
    pub(crate) fn break_on_dom_debugger_will_modify_dom_attribute(&mut self, _target: DomHandle) {}
    pub(crate) fn break_on_dom_debugger_will_remove_dom_node(
        &mut self,
        _target: DomHandle,
    ) -> Vec<DevToolsDomPrepublishedRemoval> {
        Vec::new()
    }

    pub(crate) fn break_on_dom_debugger_xhr_or_fetch_network_request(&self, _request_url: &str) {}
    pub(crate) fn has_dom_debugger_event_listener_breakpoints(&self) -> bool {
        false
    }
}
