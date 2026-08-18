use std::rc::Rc;

use tokio::sync::mpsc;

use crate::v8_platform::{V8ForegroundTaskWake, V8PlatformIsolateRegistration};
use crate::worker::{
    handle::WorkerToParentMessage, inspector_task_runner::WorkerInspectorTaskRunner,
};

use super::{
    super::module_runtime::{
        worker_dynamic_import_callback, worker_dynamic_import_with_phase_callback,
        worker_initialize_import_meta_object_callback,
    },
    runtime_inspector::WorkerRuntimeInspector,
};

pub(super) struct WorkerIsolateState {
    // Inspector teardown touches V8-owned state, and platform registration must
    // be gone before the isolate is destroyed. Keep both fields before
    // `isolate` so Rust drops them first on normal worker teardown.
    runtime_inspector: Rc<WorkerRuntimeInspector>,
    platform_registration: V8PlatformIsolateRegistration,
    isolate: v8::OwnedIsolate,
}

impl WorkerIsolateState {
    pub(super) fn new(
        platform_wake: V8ForegroundTaskWake,
        inspector_task_runner: WorkerInspectorTaskRunner,
        parent_tx: mpsc::UnboundedSender<WorkerToParentMessage>,
        shared_worker: bool,
    ) -> Self {
        let mut isolate = v8::Isolate::new(Default::default());
        crate::context_bootstrap::install_agent_microtask_checkpoint_tasks(&mut isolate);
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        isolate.set_capture_stack_trace_for_uncaught_exceptions(true, 32);
        isolate.set_host_initialize_import_meta_object_callback(
            worker_initialize_import_meta_object_callback,
        );
        isolate.set_host_import_module_dynamically_callback(worker_dynamic_import_callback);
        isolate.set_host_import_module_with_phase_dynamically_callback(
            worker_dynamic_import_with_phase_callback,
        );
        isolate.set_modify_code_generation_from_strings_callback(
            crate::context_bootstrap::trusted_types_code_generation_check_callback,
        );
        let runtime_inspector = WorkerRuntimeInspector::new(
            &mut isolate,
            inspector_task_runner,
            parent_tx,
            shared_worker,
        );
        let platform_registration = V8PlatformIsolateRegistration::register(
            &mut isolate,
            platform_wake.into_platform_wake(),
        );

        Self {
            runtime_inspector,
            platform_registration,
            isolate,
        }
    }

    pub(super) fn worker_isolate_mut(&mut self) -> &mut v8::OwnedIsolate {
        &mut self.isolate
    }

    pub(super) fn worker_runtime_inspector(&self) -> &WorkerRuntimeInspector {
        &self.runtime_inspector
    }

    pub(super) fn worker_isolate_and_runtime_inspector(
        &mut self,
    ) -> (&mut v8::OwnedIsolate, Rc<WorkerRuntimeInspector>) {
        (&mut self.isolate, Rc::clone(&self.runtime_inspector))
    }

    pub(super) fn unregister_worker_isolate_platform(&mut self) {
        self.platform_registration.unregister();
    }
}
