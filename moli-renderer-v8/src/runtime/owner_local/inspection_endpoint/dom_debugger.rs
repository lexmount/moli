use super::super::*;

/// A borrowed, session-scoped DOMDebugger capability, without Page access.
pub struct RendererDomDebuggerInspection<'a> {
    endpoint: &'a RendererInspectionEndpoint,
    attachment: RendererAgentAttachmentId,
    inspector_session_id: Option<String>,
}

impl RendererInspectionEndpoint {
    pub fn dom_debugger_inspection(
        &self,
        attachment: RendererAgentAttachmentId,
        inspector_session_id: Option<String>,
    ) -> RendererDomDebuggerInspection<'_> {
        RendererDomDebuggerInspection {
            endpoint: self,
            attachment,
            inspector_session_id,
        }
    }
}

impl RendererDomDebuggerInspection<'_> {
    fn start_page_command(
        &self,
        command: RendererPageCommand,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.endpoint.enqueue_typed_inspection_command(
            self.attachment,
            self.inspector_session_id.clone(),
            command,
        )
    }
    pub fn start_dom_debugger_get_event_listeners(
        &self,
        object_id: String,
        depth: i32,
        pierce: bool,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::dom_debugger_get_event_listeners(
            self.inspector_session_id.clone(),
            object_id,
            depth,
            pierce,
        ))
    }

    pub fn start_dom_debugger_configure_event_listener_breakpoint(
        &self,
        breakpoint: RendererDomDebuggerEventListenerBreakpoint,
        enabled: bool,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(
            RendererPageCommand::DomDebuggerConfigureEventListenerBreakpoint {
                inspector_session_id: self.inspector_session_id.clone(),
                breakpoint,
                enabled,
            },
        )
    }

    pub fn start_dom_debugger_configure_xhr_breakpoint(
        &self,
        breakpoint: RendererDomDebuggerXhrBreakpoint,
        enabled: bool,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::DomDebuggerConfigureXhrBreakpoint {
            inspector_session_id: self.inspector_session_id.clone(),
            breakpoint,
            enabled,
        })
    }

    pub fn start_dom_debugger_configure_dom_breakpoint(
        &self,
        frontend_node_id: u32,
        breakpoint_type: String,
        enabled: bool,
    ) -> Result<RendererRuntimeInspectorMainCommandRoute> {
        self.start_page_command(RendererPageCommand::DomDebuggerConfigureDomBreakpoint {
            inspector_session_id: self.inspector_session_id.clone(),
            frontend_node_id,
            breakpoint_type,
            enabled,
        })
    }
}
