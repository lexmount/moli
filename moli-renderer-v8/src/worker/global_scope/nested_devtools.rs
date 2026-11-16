//! A child Worker inherits interception control, but owns its target and output.
use super::*;
use crate::runtime::{
    PendingRendererOutputRecord, RendererDedicatedWorkerTargetEvent as Event,
    RendererDedicatedWorkerTargetInfo, RendererDedicatedWorkerTargetOwner, RendererOwnerAction,
    RendererTurnOutputJournal, RendererWorkerTargetIdentity,
};

pub(crate) struct NestedWorkerDevTools {
    instance_id: u64,
    journal: RendererTurnOutputJournal,
    runtime: crate::runtime::RendererWorkerContextRuntime,
    interception: crate::network::interception_output::ResourceInterceptionContext,
}

impl NestedWorkerDevTools {
    fn publish(&self, event: Event) {
        publish(&self.journal, event);
    }

    pub(crate) fn publish_response(
        &self,
        publication: crate::runtime::RendererRuntimeInspectorResponsePublication,
    ) {
        let predecessor = self
            .journal
            .last_published_cursor()
            .map(|cursor| self.journal.declare_fence(cursor));
        let _ = publication.commit(predecessor);
    }

    pub(crate) fn message(&self, message: &WorkerToParentMessage) {
        match message {
            WorkerToParentMessage::Console(console) => self.publish(Event::Console {
                instance_id: self.instance_id,
                message: crate::runtime::RendererSharedWorkerConsoleMessage {
                    message: console.message.clone(),
                    args: console.args.clone(),
                    stack: console.stack.clone(),
                },
            }),
            WorkerToParentMessage::RuntimeInspectorMessages(batches) => {
                for batch in batches {
                    let messages = batch.messages.iter().filter(|message| {
                        !matches!(message, crate::runtime::RendererRuntimeInspectorMessage::Protocol(message) if message.payload().has_field("id"))
                    }).cloned().collect::<Vec<_>>();
                    if !messages.is_empty() {
                        self.publish(Event::RuntimeInspectorMessages {
                            instance_id: self.instance_id,
                            inspector_session_id: batch.inspector_session_id.clone(),
                            messages,
                        });
                    }
                }
            }
            _ => {}
        }
    }
}

impl Drop for NestedWorkerDevTools {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.browser_context_runtime() {
            runtime.unregister_dedicated_worker_devtools_handle(self.instance_id);
        }
        self.publish(Event::Destroyed {
            instance_id: self.instance_id,
        });
    }
}

fn publish(journal: &RendererTurnOutputJournal, event: Event) -> bool {
    journal.try_publish_record(
        PendingRendererOutputRecord::owner_action(
            None,
            RendererOwnerAction::DedicatedWorkerTargetLifecycle(event),
        )
        .resolve()
        .expect("Worker target output has its creator's native source"),
    )
}

/// Register the target before script loading, including failed HTTP/MIME loads.
pub(crate) fn prepare_nested_worker(
    context: &super::nested_workers::NestedWorkerContext,
    request_url: &Url,
    name: String,
) -> Option<NestedWorkerDevTools> {
    let control = context.interception_context.as_ref()?;
    let runtime = context.worker_context_runtime.browser_context_runtime()?;
    let (parent, journal) = control.worker_target()?;
    let instance_id = runtime.allocate_dedicated_worker_instance_id();
    let output = journal.clone();
    let interception = crate::network::interception_output::ResourceInterceptionContext::worker(
        Some(control.clone()),
        move |event| publish(&output, Event::Resource { instance_id, event }),
    )
    .with_worker_target(
        RendererWorkerTargetIdentity::Dedicated(instance_id),
        journal.clone(),
    );
    let devtools = NestedWorkerDevTools {
        instance_id,
        journal,
        runtime: context.worker_context_runtime.clone(),
        interception,
    };
    devtools.publish(Event::Created(RendererDedicatedWorkerTargetInfo {
        owner: RendererDedicatedWorkerTargetOwner::Worker(parent),
        instance_id,
        request_url: request_url.to_string(),
        document_url: context.base_url.to_string(),
        name,
    }));
    Some(devtools)
}

impl NestedWorkerDevTools {
    pub(crate) fn script_failed(
        &self,
        request_url: &Url,
        error_message: String,
        response: Option<Box<crate::protocol_types::NavigationResponse>>,
    ) {
        self.publish(Event::ScriptLoadFailed {
            instance_id: self.instance_id,
            script_url: request_url.to_string(),
            error_message,
            response,
        });
    }
}

pub(crate) fn start_nested_worker(
    options: crate::worker::WorkerSpawnOptions,
    context: &super::nested_workers::NestedWorkerContext,
    response: Box<crate::protocol_types::NavigationResponse>,
    devtools: Option<NestedWorkerDevTools>,
) -> (crate::worker::WorkerHandle, Option<NestedWorkerDevTools>) {
    let Some(devtools) = devtools else {
        // An embedding without DevTools has its own unobserved loader.
        return (
            crate::worker::spawn_worker_with_options(options.with_interception_context(None)),
            None,
        );
    };
    let runtime = context
        .worker_context_runtime
        .browser_context_runtime()
        .expect("prepared Worker runtime");
    let script_url = options.script_url.clone();
    let worker = crate::worker::spawn_worker_with_options(
        options
            .with_interception_context(Some(devtools.interception.clone()))
            .with_pause_evaluation_until_debugger(true),
    );
    runtime.attach_dedicated_worker_devtools_handle(
        devtools.instance_id,
        &worker,
        Some(devtools.journal.clone()),
    );
    devtools.publish(Event::ScriptLoaded {
        instance_id: devtools.instance_id,
        script_url,
        response,
    });
    if !context.pause_on_start.load(Ordering::Acquire) {
        worker.devtools_handle().run_if_waiting_for_debugger();
    }
    (worker, Some(devtools))
}
