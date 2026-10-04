use super::*;

/// V8 owns one Inspector per isolate; each Page owns its command queues,
/// pause state and output route. A context group selects that Page's executor.
#[derive(Clone, Default)]
pub(super) struct RendererInspectorPageTargets {
    groups: Rc<RefCell<HashMap<i32, Weak<RendererInspectorSessionExecutorLocal>>>>,
    paused: Rc<RefCell<Option<Weak<RendererInspectorSessionExecutorLocal>>>>,
}

impl RendererInspectorPageTargets {
    pub(super) fn bind_group(
        &self,
        group: DocumentInspectorContextGroupId,
        executor: &Rc<RendererInspectorSessionExecutorLocal>,
    ) {
        let previous = self
            .groups
            .borrow_mut()
            .insert(group.get(), Rc::downgrade(executor));
        assert!(
            previous
                .and_then(|previous| previous.upgrade())
                .is_none_or(|previous| Rc::ptr_eq(&previous, executor)),
            "an Inspector context group cannot move between Page executors"
        );
    }

    pub(super) fn remove_group(&self, group: DocumentInspectorContextGroupId) {
        self.groups.borrow_mut().remove(&group.get());
    }

    pub(super) fn executor_for_group(
        &self,
        group: i32,
    ) -> Option<Rc<RendererInspectorSessionExecutorLocal>> {
        self.groups.borrow().get(&group).and_then(Weak::upgrade)
    }

    pub(super) fn paused_executor(&self) -> Option<Rc<RendererInspectorSessionExecutorLocal>> {
        self.paused.borrow().as_ref().and_then(Weak::upgrade)
    }

    pub(super) fn enter_pause(
        &self,
        executor: &Rc<RendererInspectorSessionExecutorLocal>,
    ) -> InspectorPagePauseGuard {
        let previous = self.paused.borrow_mut().replace(Rc::downgrade(executor));
        InspectorPagePauseGuard {
            targets: self.clone(),
            previous,
        }
    }
}

pub(super) struct InspectorPagePauseGuard {
    targets: RendererInspectorPageTargets,
    previous: Option<Weak<RendererInspectorSessionExecutorLocal>>,
}

impl Drop for InspectorPagePauseGuard {
    fn drop(&mut self) {
        *self.targets.paused.borrow_mut() = self.previous.take();
    }
}

impl RendererInspectorIsolateBackendHandle {
    /// Allocate an independent Page endpoint while retaining the shared V8
    /// Inspector. Document replacements clone the existing handle instead.
    pub(crate) fn new_page_handle(&self, isolate: &mut v8::Isolate) -> anyhow::Result<Self> {
        let source = self
            .endpoint
            .session_executor
            .as_ref()
            .expect("a live Page has an executor");
        let isolate_ptr = unsafe { isolate.as_raw_isolate_ptr() };
        assert!(
            isolate
                .get_slot::<Rc<RendererInspectorIsolateBackendIdentity>>()
                .is_some_and(|identity| Rc::ptr_eq(identity, &self.endpoint.identity)),
            "a related Page must share its admitted isolate"
        );
        let pause_bridge = RendererInspectorPauseBridge::default();
        let route_id =
            RendererInspectorSessionExecutorRouteId::new(allocate_session_executor_route_id());
        let main = RendererInspectorMainIngress::new(route_id, pause_bridge.pause_loop_wake());
        let io = RendererInspectorIoIngress::new_for_page(
            pause_bridge.pause_loop_wake(),
            (
                isolate.thread_safe_handle(),
                dispatch_inspector_interrupt,
                route_id,
            ),
        );
        let target = RendererDevToolsTargetHandle::new(pause_bridge, main, io);
        self.endpoint
            .isolate_environment_ingress
            .register_page_pause_wake(route_id, target.pause_ref().pause_loop_wake());
        let executor = RendererInspectorSessionExecutorLocal::new(
            isolate_ptr,
            target.clone(),
            route_id,
            source.page_targets.clone(),
            Some(self.endpoint.isolate_environment_ingress.clone()),
        );
        let registration = self
            .endpoint
            .shutdown_registry
            .as_ref()
            .map(|registry| registry.register(target.clone()))
            .transpose()
            .map_err(|message| anyhow::anyhow!(message))?;
        Ok(Self {
            endpoint: Rc::new(RendererInspectorPageEndpoint {
                identity: self.endpoint.identity.clone(),
                target,
                isolate_environment_ingress: self.endpoint.isolate_environment_ingress.clone(),
                session_executor: Some(executor),
                shutdown_registry: self.endpoint.shutdown_registry.clone(),
                _shutdown_registration: registration,
            }),
        })
    }

    pub(in crate::script_vm::inspector) fn bind_context_group(
        &self,
        group: DocumentInspectorContextGroupId,
    ) {
        if let Some(executor) = &self.endpoint.session_executor {
            executor.page_targets.bind_group(group, executor);
        }
    }

    pub(in crate::script_vm::inspector) fn remove_context_group(
        &self,
        group: DocumentInspectorContextGroupId,
    ) {
        if let Some(executor) = &self.endpoint.session_executor {
            executor.page_targets.remove_group(group);
        }
    }
}
