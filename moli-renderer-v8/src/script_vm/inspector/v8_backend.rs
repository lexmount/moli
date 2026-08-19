use super::InspectorOutbound;
use super::context_registry::{
    DocumentInspectorContextGroupId, DocumentInspectorContextRegistrationId,
    DocumentInspectorContextRegistry,
};
mod interrupt;

use crate::{
    devtools::{
        ingress::{
            io::{RendererInspectorIoCommand, RendererInspectorIoIngress},
            main::{
                RendererInspectorMainCommand, RendererInspectorMainIngress,
                RendererInspectorMainOwnerDispatch,
            },
        },
        pause::RendererInspectorPauseBridge,
        route::RendererInspectorSessionExecutorRouteId,
        target::RendererDevToolsTargetHandle,
    },
    inspector_microtasks::with_scoped_inspector_microtasks,
    runtime::{
        RendererDevToolsIoCommandKind, RendererDevToolsIoCommandPayload,
        RendererDevToolsMainNestedDispatch, RendererOwnerReply,
        RendererRuntimeInspectorResponseSender, dispatch_nested_main_page_command,
    },
};
use interrupt::{
    allocate_session_executor_route_id, dispatch_inspector_interrupt, register_session_executor,
    unregister_session_executor,
};
pub(crate) use interrupt::{dispatch_inspector_io_owner_wake, dispatch_inspector_main_owner_wake};
use moli_page_types::{DevToolsSessionKey, RendererDevToolsAgentToken, V8InspectorSessionState};
use serde_json::json;
use std::{
    cell::{Cell, RefCell, UnsafeCell},
    collections::HashMap,
    rc::{Rc, Weak},
    sync::atomic::{AtomicI64, Ordering},
};

struct RendererInspectorClient {
    isolate: UnsafeCell<v8::UnsafeRawIsolatePtr>,
    context_registry: DocumentInspectorContextRegistry,
    unique_id_state: Rc<RendererInspectorClientUniqueIdState>,
    session_executor: Rc<RendererInspectorSessionExecutorLocal>,
}

#[derive(Clone)]
struct RendererInspectorSessionRoute {
    session: Weak<v8::inspector::V8InspectorSession>,
    outbound: InspectorOutbound,
    agent_token: RendererDevToolsAgentToken,
    session_key: DevToolsSessionKey,
}

pub(super) struct RendererInspectorSessionExecutorRegistration {
    session_executor: Weak<RendererInspectorSessionExecutorLocal>,
    context_group_id: i32,
    agent_token: RendererDevToolsAgentToken,
    session_key: DevToolsSessionKey,
    session: Weak<v8::inspector::V8InspectorSession>,
}

struct RendererInspectorSessionExecutorLocal {
    isolate: UnsafeCell<v8::UnsafeRawIsolatePtr>,
    target: RendererDevToolsTargetHandle,
    route_id: RendererInspectorSessionExecutorRouteId,
    sessions: RefCell<HashMap<(i32, DevToolsSessionKey), RendererInspectorSessionRoute>>,
    interrupt_sessions: RefCell<
        HashMap<(RendererDevToolsAgentToken, DevToolsSessionKey), RendererInspectorSessionRoute>,
    >,
}

enum RendererInspectorNestedCommand {
    Main(RendererInspectorMainCommand),
    Io(RendererInspectorIoCommand),
}

impl Drop for RendererInspectorSessionExecutorLocal {
    fn drop(&mut self) {
        unregister_session_executor(self.route_id);
        self.target
            .main_ref()
            .close("Inspector session executor was destroyed");
        self.target
            .io_ref()
            .close("Inspector session executor was destroyed");
    }
}

impl Drop for RendererInspectorSessionExecutorRegistration {
    fn drop(&mut self) {
        let Some(session_executor) = self.session_executor.upgrade() else {
            return;
        };
        let key = (self.context_group_id, self.session_key.clone());
        let mut sessions = session_executor.sessions.borrow_mut();
        if sessions
            .get(&key)
            .is_some_and(|entry| Weak::ptr_eq(&entry.session, &self.session))
        {
            sessions.remove(&key);
        }
        let interrupt_key = (self.agent_token, self.session_key.clone());
        let mut interrupt_sessions = session_executor.interrupt_sessions.borrow_mut();
        if interrupt_sessions
            .get(&interrupt_key)
            .is_some_and(|entry| Weak::ptr_eq(&entry.session, &self.session))
        {
            interrupt_sessions.remove(&interrupt_key);
        }
    }
}

impl RendererInspectorSessionExecutorLocal {
    fn new(
        isolate: v8::UnsafeRawIsolatePtr,
        target: RendererDevToolsTargetHandle,
        route_id: RendererInspectorSessionExecutorRouteId,
    ) -> Rc<Self> {
        debug_assert_eq!(target.io_ref().route_id(), Some(route_id));
        let session_executor = Rc::new(Self {
            isolate: UnsafeCell::new(isolate),
            target,
            route_id,
            sessions: RefCell::new(HashMap::new()),
            interrupt_sessions: RefCell::new(HashMap::new()),
        });
        register_session_executor(route_id, &session_executor);
        session_executor
    }

    fn register_session(
        self: &Rc<Self>,
        context_group_id: DocumentInspectorContextGroupId,
        agent_token: RendererDevToolsAgentToken,
        session_key: DevToolsSessionKey,
        session: &Rc<v8::inspector::V8InspectorSession>,
        outbound: InspectorOutbound,
    ) -> RendererInspectorSessionExecutorRegistration {
        let weak_session = Rc::downgrade(session);
        let route = RendererInspectorSessionRoute {
            session: weak_session.clone(),
            outbound,
            agent_token,
            session_key: session_key.clone(),
        };
        self.sessions
            .borrow_mut()
            .insert((context_group_id.get(), session_key.clone()), route.clone());
        // Context groups may overlap briefly during document replacement.
        // Active-JS interrupts carry the stable renderer-agent identity rather
        // than a V8 context-group id, so keep an explicit latest-registration
        // index instead of depending on HashMap iteration order.
        self.interrupt_sessions
            .borrow_mut()
            .insert((agent_token, session_key.clone()), route);
        RendererInspectorSessionExecutorRegistration {
            session_executor: Rc::downgrade(self),
            context_group_id: context_group_id.get(),
            agent_token,
            session_key,
            session: weak_session,
        }
    }

    fn run_message_loop_on_pause(&self, context_group_id: i32) {
        let Some(pause_loop_policy) = self.target.pause_ref().enter_pause() else {
            return;
        };
        let mut prefer_main = true;
        while let Some(command) = self.target.pause_ref().wait_for_pause_work(|| {
            let command = match pause_loop_policy {
                crate::devtools::pause::RendererInspectorPauseLoopPolicy::IoOnly => self
                    .target
                    .io_ref()
                    .claim_for_pause()
                    .map(RendererInspectorNestedCommand::Io),
                crate::devtools::pause::RendererInspectorPauseLoopPolicy::MainAndIo
                    if prefer_main =>
                {
                    self.target
                        .main_ref()
                        .claim_for_pause()
                        .map(RendererInspectorNestedCommand::Main)
                        .or_else(|| {
                            self.target
                                .io_ref()
                                .claim_for_pause()
                                .map(RendererInspectorNestedCommand::Io)
                        })
                }
                crate::devtools::pause::RendererInspectorPauseLoopPolicy::MainAndIo => self
                    .target
                    .io_ref()
                    .claim_for_pause()
                    .map(RendererInspectorNestedCommand::Io)
                    .or_else(|| {
                        self.target
                            .main_ref()
                            .claim_for_pause()
                            .map(RendererInspectorNestedCommand::Main)
                    }),
            };
            if command.is_some()
                && pause_loop_policy
                    == crate::devtools::pause::RendererInspectorPauseLoopPolicy::MainAndIo
            {
                prefer_main = !prefer_main;
            }
            command
        }) {
            match command {
                RendererInspectorNestedCommand::Main(command) => {
                    self.dispatch_main_command(context_group_id, command);
                }
                RendererInspectorNestedCommand::Io(command) => {
                    self.dispatch_io_command(context_group_id, command);
                }
            }
        }
        self.target.pause_ref().leave_pause();
    }

    fn dispatch_io_command(&self, context_group_id: i32, command: RendererInspectorIoCommand) {
        let session = (command.kind() == RendererDevToolsIoCommandKind::Inspector).then(|| {
            let session_key = command.ticket().session().clone();
            self.sessions
                .borrow()
                .get(&(context_group_id, session_key))
                .filter(|session| session.agent_token == command.agent_token)
                .cloned()
        });
        let session = session.flatten();
        self.dispatch_io_command_to_session(command, session);
    }

    fn dispatch_main_command(&self, context_group_id: i32, command: RendererInspectorMainCommand) {
        match command.nested_dispatch() {
            RendererDevToolsMainNestedDispatch::PageAgent => {
                let first_dispatch = self.target.main_ref().first_dispatch_guard(&command);
                let (page_command, reply_tx) = command.into_nested_page_parts();
                let result = dispatch_nested_main_page_command(page_command, first_dispatch)
                    .map(|output| RendererOwnerReply::AsyncPageCommandRan(Box::new(output)));
                let _ = reply_tx.send(result);
                return;
            }
            RendererDevToolsMainNestedDispatch::InspectorSession => {}
            RendererDevToolsMainNestedDispatch::OwnerOnly => {
                unreachable!("an owner-only Main command cannot be claimed by the pause loop")
            }
        }
        let session_key = command.ticket().session().clone();
        let session = self
            .sessions
            .borrow()
            .get(&(context_group_id, session_key))
            .filter(|session| session.agent_token == command.agent_token)
            .cloned();
        self.dispatch_main_command_to_session(command, session);
    }

    fn dispatch_next_io_command_from_interrupt(&self) {
        let Some(command) = self.target.io_ref().claim_for_interrupt() else {
            return;
        };
        let session = (command.kind() == RendererDevToolsIoCommandKind::Inspector).then(|| {
            let session_key = command.ticket().session();
            self.interrupt_sessions
                .borrow()
                .get(&(command.agent_token, session_key.clone()))
                .cloned()
        });
        let session = session.flatten();
        self.dispatch_io_command_to_session(command, session);
    }

    fn dispatch_next_io_command_from_owner(&self) {
        let Some(command) = self.target.io_ref().claim_for_owner() else {
            return;
        };
        let session = (command.kind() == RendererDevToolsIoCommandKind::Inspector).then(|| {
            let session_key = command.ticket().session();
            self.interrupt_sessions
                .borrow()
                .get(&(command.agent_token, session_key.clone()))
                .cloned()
        });
        let session = session.flatten();
        self.dispatch_io_command_to_session(command, session);
    }

    fn claim_next_main_command_from_owner(&self) -> Option<RendererInspectorMainOwnerDispatch> {
        let command = self.target.main_ref().claim_for_owner()?;
        Some(self.target.main_ref().prepare_owner_dispatch(command))
    }

    fn dispatch_main_command_to_session(
        &self,
        command: RendererInspectorMainCommand,
        session: Option<RendererInspectorSessionRoute>,
    ) {
        let mut first_dispatch = self.target.main_ref().first_dispatch_guard(&command);
        let Some(session) = session else {
            let (_, _, response) = command.into_protocol_parts();
            send_inspector_dispatch_error(Some(response), "Inspector session is not available");
            return;
        };
        let Some(v8_session) = session.session.upgrade() else {
            let (_, _, response) = command.into_protocol_parts();
            send_inspector_dispatch_error(Some(response), "Inspector session has been detached");
            return;
        };
        let _command_dispatch = self.target.pause_ref().begin_command_dispatch(
            command.command_id(),
            command.ticket(),
            command.pause_effect(),
            Some(command.response().call_id()),
        );
        let (_, raw_json, response) = command.into_protocol_parts();
        session.outbound.register_response_callback(response);
        let _post_dispatch_wake = first_dispatch.release_for_dispatch();
        v8_session.dispatch_protocol_message(v8::inspector::StringView::from(raw_json.as_bytes()));
        self.target.pause_ref().record_v8_state_update(
            session.agent_token,
            session.session_key,
            V8InspectorSessionState::from_bytes(v8_session.state()),
        );
    }

    fn dispatch_io_command_to_session(
        &self,
        mut command: RendererInspectorIoCommand,
        session: Option<RendererInspectorSessionRoute>,
    ) {
        let mut first_dispatch = self.target.io_ref().first_dispatch_guard(&mut command);
        match command.kind() {
            RendererDevToolsIoCommandKind::Performance => {
                assert!(
                    matches!(
                        command.into_payload(),
                        RendererDevToolsIoCommandPayload::PerformanceGetMetrics
                    ),
                    "Performance IO command kind must carry a Performance payload"
                );
                first_dispatch.release();
                return;
            }
            RendererDevToolsIoCommandKind::Emulation => {
                let RendererDevToolsIoCommandPayload::SetScriptExecutionDisabled {
                    control,
                    disabled,
                } = command.into_payload()
                else {
                    unreachable!("Emulation IO command kind must carry an Emulation payload")
                };
                control.set_disabled(disabled);
                first_dispatch.release();
                return;
            }
            RendererDevToolsIoCommandKind::Inspector => {}
        }
        let Some(session) = session else {
            send_inspector_dispatch_error(
                command.take_response(),
                "Inspector session is not available",
            );
            return;
        };
        let Some(v8_session) = session.session.upgrade() else {
            send_inspector_dispatch_error(
                command.take_response(),
                "Inspector session has been detached",
            );
            return;
        };
        let _command_dispatch = self.target.pause_ref().begin_command_dispatch(
            command.command_id(),
            command.ticket(),
            command.pause_effect(),
            command.response().map(|response| response.call_id()),
        );
        let response_delivery = command.response_delivery();
        if let Some(response) = command.take_response() {
            session
                .outbound
                .register_frontend_response_callback(response, response_delivery);
        }
        let _post_dispatch_wake = first_dispatch.release_for_dispatch();
        v8_session.dispatch_protocol_message(v8::inspector::StringView::from(
            command.raw_json().as_bytes(),
        ));
        self.target.pause_ref().record_v8_state_update(
            session.agent_token,
            session.session_key,
            V8InspectorSessionState::from_bytes(v8_session.state()),
        );
    }

    fn quit_message_loop_on_pause(&self) {
        self.target.pause_ref().request_quit();
    }
}

fn send_inspector_dispatch_error(
    response: Option<RendererRuntimeInspectorResponseSender>,
    message: &str,
) {
    let Some(response) = response else {
        return;
    };
    let call_id = response.call_id();
    let _ = response.send(json!({
        "id": call_id,
        "error": {
            "code": -32000,
            "message": message,
        },
    }));
}

pub(super) struct RendererInspectorClientUniqueIdState {
    capture_depth: Cell<usize>,
    captured_ids: RefCell<Vec<i64>>,
}

struct RendererInspectorUniqueIdCaptureGuard<'a> {
    state: &'a RendererInspectorClientUniqueIdState,
}

impl Drop for RendererInspectorUniqueIdCaptureGuard<'_> {
    fn drop(&mut self) {
        let depth = self.state.capture_depth.get();
        debug_assert!(depth > 0, "V8 inspector unique-id capture underflow");
        self.state.capture_depth.set(depth.saturating_sub(1));
    }
}

impl RendererInspectorClientUniqueIdState {
    pub(super) fn new() -> Self {
        Self {
            capture_depth: Cell::new(0),
            captured_ids: RefCell::new(Vec::new()),
        }
    }

    pub(super) fn generate_unique_id(&self) -> i64 {
        static NEXT_UNIQUE_ID: AtomicI64 = AtomicI64::new(1);

        let id = NEXT_UNIQUE_ID
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .expect("V8 inspector unique id exhausted");
        assert!(id > 0, "V8 inspector unique id exhausted");
        if self.capture_depth.get() > 0 {
            self.captured_ids.borrow_mut().push(id);
        }
        id
    }

    pub(super) fn capture_context_unique_id(&self, op: impl FnOnce()) -> Option<String> {
        debug_assert_eq!(
            self.capture_depth.get(),
            0,
            "nested V8 inspector context unique-id capture"
        );
        self.captured_ids.borrow_mut().clear();
        self.capture_depth.set(self.capture_depth.get() + 1);
        {
            let _capture_guard = RendererInspectorUniqueIdCaptureGuard { state: self };
            op();
        }
        let ids = self.captured_ids.borrow();
        (ids.len() >= 2).then(|| format!("{}.{}", ids[0], ids[1]))
    }

    #[cfg(test)]
    pub(super) fn capture_depth_for_test(&self) -> usize {
        self.capture_depth.get()
    }
}

impl RendererInspectorClient {
    fn new(
        isolate: v8::UnsafeRawIsolatePtr,
        context_registry: DocumentInspectorContextRegistry,
        unique_id_state: Rc<RendererInspectorClientUniqueIdState>,
        session_executor: Rc<RendererInspectorSessionExecutorLocal>,
    ) -> Self {
        Self {
            isolate: UnsafeCell::new(isolate),
            context_registry,
            unique_id_state,
            session_executor,
        }
    }
}

impl v8::inspector::V8InspectorClientImpl for RendererInspectorClient {
    fn run_message_loop_on_pause(&self, context_group_id: i32) {
        // A pause may originate in an ordinary page task, outside the guarded
        // frontend dispatch path. Keep every nested pause-loop command under
        // the same Inspector policy boundary without adding another isolate
        // pointer or changing the pause bridge protocol.
        let isolate = unsafe { &mut *self.isolate.get() };
        let isolate = unsafe { v8::Isolate::ref_from_raw_isolate_ptr_mut(isolate) };
        with_scoped_inspector_microtasks(isolate, || {
            self.session_executor
                .run_message_loop_on_pause(context_group_id);
        });
    }

    fn quit_message_loop_on_pause(&self) {
        self.session_executor.quit_message_loop_on_pause();
    }

    fn generate_unique_id(&self) -> i64 {
        self.unique_id_state.generate_unique_id()
    }

    fn ensure_default_context_in_group(
        &self,
        context_group_id: i32,
    ) -> Option<v8::Local<'_, v8::Context>> {
        let context_group_id = DocumentInspectorContextGroupId::from_raw(context_group_id);

        let isolate = unsafe { &mut *self.isolate.get() };
        let isolate = unsafe { v8::Isolate::ref_from_raw_isolate_ptr_mut(isolate) };
        v8::callback_scope!(unsafe let scope, isolate);
        self.context_registry
            .with_default_context(context_group_id, |default_context| {
                v8::Local::new(scope, default_context)
            })
    }
}

struct RendererInspectorIsolateBackendIdentity;

/// Opaque reference tying a renderer agent to its isolate's Inspector backend.
///
/// The reference deliberately exposes no V8 operations. The document-isolate
/// holder remains the backend owner and controls isolate entry and teardown.
#[derive(Clone)]
pub(crate) struct RendererInspectorIsolateBackendHandle {
    identity: Rc<RendererInspectorIsolateBackendIdentity>,
    target: RendererDevToolsTargetHandle,
}

impl std::fmt::Debug for RendererInspectorIsolateBackendHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RendererInspectorIsolateBackendHandle")
            .field("identity_key", &Rc::as_ptr(&self.identity))
            .finish()
    }
}

pub(in crate::script_vm) struct RendererInspectorIsolateBackend {
    identity: Rc<RendererInspectorIsolateBackendIdentity>,
    inspector: v8::inspector::V8Inspector,
    pub(super) context_registry: DocumentInspectorContextRegistry,
    unique_id_state: Rc<RendererInspectorClientUniqueIdState>,
    target: RendererDevToolsTargetHandle,
    session_executor: Rc<RendererInspectorSessionExecutorLocal>,
}

impl RendererInspectorIsolateBackend {
    pub(in crate::script_vm) fn new(isolate: &mut v8::Isolate) -> Self {
        let isolate_ptr = unsafe { isolate.as_raw_isolate_ptr() };
        let context_registry = DocumentInspectorContextRegistry::default();
        let unique_id_state = Rc::new(RendererInspectorClientUniqueIdState::new());
        let pause_bridge = RendererInspectorPauseBridge::default();
        let route_id =
            RendererInspectorSessionExecutorRouteId::new(allocate_session_executor_route_id());
        let main_ingress =
            RendererInspectorMainIngress::new(route_id, pause_bridge.pause_loop_wake());
        let io_ingress = RendererInspectorIoIngress::new(
            pause_bridge.pause_loop_wake(),
            Some((
                isolate.thread_safe_handle(),
                dispatch_inspector_interrupt,
                route_id,
            )),
        );
        let target = RendererDevToolsTargetHandle::new(pause_bridge, main_ingress, io_ingress);
        let session_executor =
            RendererInspectorSessionExecutorLocal::new(isolate_ptr, target.clone(), route_id);
        let inspector_client =
            v8::inspector::V8InspectorClient::new(Box::new(RendererInspectorClient::new(
                isolate_ptr,
                context_registry.clone(),
                unique_id_state.clone(),
                session_executor.clone(),
            )));
        Self {
            identity: Rc::new(RendererInspectorIsolateBackendIdentity),
            inspector: v8::inspector::V8Inspector::create(isolate, inspector_client),
            context_registry,
            unique_id_state,
            target,
            session_executor,
        }
    }

    pub(crate) fn handle(&self) -> RendererInspectorIsolateBackendHandle {
        RendererInspectorIsolateBackendHandle {
            identity: Rc::clone(&self.identity),
            target: self.target.clone(),
        }
    }

    pub(super) fn devtools_target(&self) -> RendererDevToolsTargetHandle {
        self.target.clone()
    }

    pub(super) fn register_session_executor_route(
        &self,
        context_group_id: DocumentInspectorContextGroupId,
        agent_token: RendererDevToolsAgentToken,
        session_key: DevToolsSessionKey,
        session: &Rc<v8::inspector::V8InspectorSession>,
        outbound: InspectorOutbound,
    ) -> RendererInspectorSessionExecutorRegistration {
        self.session_executor.register_session(
            context_group_id,
            agent_token,
            session_key,
            session,
            outbound,
        )
    }

    pub(super) fn connect_session(
        &mut self,
        context_group_id: DocumentInspectorContextGroupId,
        channel: v8::inspector::Channel,
        state: &[u8],
    ) -> v8::inspector::V8InspectorSession {
        self.inspector.connect(
            context_group_id.get(),
            channel,
            v8::inspector::StringView::from(state),
            v8::inspector::V8InspectorClientTrustLevel::FullyTrusted,
        )
    }

    pub(super) fn context_created_with_unique_id<'s>(
        &self,
        context: v8::Local<'s, v8::Context>,
        context_group_id: DocumentInspectorContextGroupId,
        name: &[u8],
        origin: &[u8],
        aux_data: &[u8],
    ) -> Option<String> {
        self.unique_id_state.capture_context_unique_id(|| {
            self.inspector.context_created(
                context,
                context_group_id.get(),
                v8::inspector::StringView::from(name),
                v8::inspector::StringView::from(origin),
                v8::inspector::StringView::from(aux_data),
            );
        })
    }

    pub(in crate::script_vm) fn context_destroyed<'s>(&self, context: v8::Local<'s, v8::Context>) {
        self.inspector.context_destroyed(context);
    }

    fn reset_context_group(&self, context_group_id: DocumentInspectorContextGroupId) {
        self.inspector.reset_context_group(context_group_id.get());
    }

    pub(super) fn default_context_destroyed<'s>(
        &self,
        context_group_id: DocumentInspectorContextGroupId,
        registration_id: DocumentInspectorContextRegistrationId,
        context: v8::Local<'s, v8::Context>,
    ) {
        if self
            .context_registry
            .default_context_is_owned_by(context_group_id, registration_id)
        {
            self.reset_context_group(context_group_id);
            self.context_registry
                .remove_default_context_if_owned_by(context_group_id, registration_id);
        }
        self.inspector.context_destroyed(context);
    }

    pub(super) fn detach_default_context_if_same(
        &self,
        context_group_id: DocumentInspectorContextGroupId,
        registration_id: DocumentInspectorContextRegistrationId,
    ) {
        if self
            .context_registry
            .default_context_is_owned_by(context_group_id, registration_id)
        {
            self.reset_context_group(context_group_id);
            self.context_registry
                .remove_default_context_if_owned_by(context_group_id, registration_id);
        }
    }

    pub(super) fn reset_default_context_group_before_replacement(
        &self,
        context_group_id: DocumentInspectorContextGroupId,
    ) -> bool {
        if self.context_registry.has_default_context(context_group_id) {
            self.reset_context_group(context_group_id);
            self.context_registry
                .remove_default_context(context_group_id);
            true
        } else {
            false
        }
    }

    pub(in crate::script_vm) fn default_context_registry_count(&self) -> usize {
        self.context_registry.len()
    }
}

impl RendererInspectorIsolateBackendHandle {
    pub(super) fn devtools_target(&self) -> RendererDevToolsTargetHandle {
        self.target.clone()
    }

    pub(super) fn assert_matches(&self, backend: &RendererInspectorIsolateBackend) {
        assert!(
            Rc::ptr_eq(&self.identity, &backend.identity),
            "renderer DevTools agent used a different isolate Inspector backend"
        );
    }

    #[cfg(test)]
    pub(super) fn new_for_test() -> Self {
        let pause_bridge = RendererInspectorPauseBridge::default();
        let route_id = RendererInspectorSessionExecutorRouteId::new(1);
        let main_ingress =
            RendererInspectorMainIngress::new(route_id, pause_bridge.pause_loop_wake());
        let io_ingress = RendererInspectorIoIngress::new(pause_bridge.pause_loop_wake(), None);
        Self {
            identity: Rc::new(RendererInspectorIsolateBackendIdentity),
            target: RendererDevToolsTargetHandle::new(pause_bridge, main_ingress, io_ingress),
        }
    }

    #[cfg(test)]
    pub(super) fn is_same_backend(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.identity, &other.identity)
    }
}
